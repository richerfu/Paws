use super::*;

// Separate from CoreState: tracing is emitted while engine/configuration locks
// are held, so a subscriber must never re-enter those locks to find its sink.
static ENGINE_LOG_SINK: std::sync::RwLock<Option<std::sync::Weak<paws_engine::EngineLogBuffer>>> =
    std::sync::RwLock::new(None);

pub(super) fn set_engine_log_sink(sink: Option<Arc<paws_engine::EngineLogBuffer>>) {
    if let Ok(mut current) = ENGINE_LOG_SINK.write() {
        *current = sink.as_ref().map(Arc::downgrade);
    }
}

pub(super) fn info_log(message: impl Into<String>) -> LogEntry {
    LogEntry {
        level: "info".to_owned(),
        message: message.into(),
        timestamp: unix_timestamp_string(),
    }
}

pub(super) fn warning_log(message: impl Into<String>) -> LogEntry {
    LogEntry {
        level: "warning".to_owned(),
        message: message.into(),
        timestamp: unix_timestamp_string(),
    }
}

pub(super) fn error_log(message: impl Into<String>) -> LogEntry {
    LogEntry {
        level: "error".to_owned(),
        message: message.into(),
        timestamp: unix_timestamp_string(),
    }
}

pub(super) fn broadcast_log_entry(entry: &LogEntry) {
    if let Ok(senders) = API_LOG_TXS.lock() {
        for tx in senders.iter() {
            let _ = tx.send(meow_api::log_stream::LogMessage {
                level: meow_api::log_stream::LogLevel::Error,
                payload: entry.message.clone(),
                time: time::OffsetDateTime::now_utc(),
            });
        }
    }
}

pub(super) fn install_runtime_log_layer() {
    INSTALL_RUNTIME_LOG_LAYER.call_once(|| {
        // ProfileStore renders the application's engine configuration with
        // log-level=info. Filter before event fields are formatted, including
        // verbose TLS traces that otherwise dominate debug CPU and log buffers.
        let subscriber = tracing_subscriber::registry().with(
            PawsLogLayer {
                logs: RUNTIME_LOGS.clone(),
            }
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
        );
        if let Err(error) = tracing::subscriber::set_global_default(subscriber) {
            push_runtime_log(&RUNTIME_LOGS, warning_log(format!(
                "engine tracing subscriber was not installed: {error}; config failures are still recorded directly"
            )));
        }
    });
}

pub(super) struct PawsLogLayer {
    logs: Arc<Mutex<RuntimeLogBuffer>>,
}

impl<S> Layer<S> for PawsLogLayer
where
    S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            if span.name() == "paws_flow" {
                let mut visitor = LogMessageVisitor::default();
                attrs.record(&mut visitor);
                let validated_probe = visitor.validated_probe;
                span.extensions_mut().insert(SocketFlowFields {
                    text: visitor.finish("flow"),
                    validated_probe,
                });
            }
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let target = event.metadata().target();
        let level = match *event.metadata().level() {
            Level::TRACE | Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warning",
            Level::ERROR => "error",
        };
        let mut visitor = LogMessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.finish(target);
        let wire_event = message.contains("wire-write-sample");
        if message.contains("socket-trace") || wire_event {
            let mut flow = String::new();
            let mut validated_probe = false;
            if let Some(scope) = ctx.event_scope(event) {
                for span in scope.from_root() {
                    if let Some(fields) = span.extensions().get::<SocketFlowFields>() {
                        append_flow_fragment(&mut flow, &fields.text);
                        validated_probe |= fields.validated_probe;
                    }
                }
            }
            if wire_event {
                if wire_trace_allowed(
                    option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC") == Some("1"),
                    option_env!("PAWS_SOCKET_WIRE_CAPTURE") == Some("1"),
                    validated_probe,
                ) {
                    append_socket_trace(&message, &flow);
                }
                // Wire data never enters runtime/UI logs, archives or API broadcasts,
                // even when a gate is disabled or the span has no validated probe.
                return;
            }
            append_socket_trace(&message, &flow);
        }
        if is_vpn_log_target(event.metadata().target()) {
            let entry = LogEntry {
                level: level.to_owned(),
                message,
                timestamp: unix_timestamp_string(),
            };
            let native = if target.starts_with("meow_")
                || target.starts_with("meow-")
                || target.starts_with("paws_vpn")
            {
                ENGINE_LOG_SINK
                    .read()
                    .ok()
                    .and_then(|sink| sink.as_ref().and_then(std::sync::Weak::upgrade))
            } else {
                None
            };
            if let Some(sink) = native {
                // The engine journal has its own fenced archive consumer.
                // Sending it to the application writer too would duplicate the
                // same native entry and bypass the recording cut watermark.
                let _ = sink.record(entry);
            } else {
                push_runtime_log(&self.logs, entry);
            }
        }
        if let Ok(senders) = API_LOG_TXS.lock() {
            for tx in senders.iter() {
                let _ = tx.send(meow_api::log_stream::LogMessage {
                    level: meow_log_level(*event.metadata().level()),
                    payload: visitor_payload(event),
                    time: time::OffsetDateTime::now_utc(),
                });
            }
        }
    }
}

pub(super) fn is_vpn_log_target(target: &str) -> bool {
    target.starts_with("paws_core")
        || target.starts_with("paws_vpn")
        || target.starts_with("meow_")
        || target.starts_with("meow-")
}

pub(super) fn meow_log_level(level: Level) -> meow_api::log_stream::LogLevel {
    match level {
        Level::TRACE | Level::DEBUG => meow_api::log_stream::LogLevel::Debug,
        Level::INFO => meow_api::log_stream::LogLevel::Info,
        Level::WARN => meow_api::log_stream::LogLevel::Warning,
        Level::ERROR => meow_api::log_stream::LogLevel::Error,
    }
}

pub(super) fn visitor_payload(event: &tracing::Event<'_>) -> String {
    let mut visitor = LogMessageVisitor::default();
    event.record(&mut visitor);
    visitor.finish(event.metadata().target())
}

#[derive(Default)]
pub(super) struct LogMessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
    validated_probe: bool,
}

impl LogMessageVisitor {
    fn finish(self, fallback: &str) -> String {
        let mut message = self.message.unwrap_or_else(|| fallback.to_owned());
        if !self.fields.is_empty() {
            message.push_str(" · ");
            message.push_str(&self.fields.join(", "));
        }
        message
    }
}

impl tracing::field::Visit for LogMessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "probe_id" {
            self.validated_probe = valid_probe_id(value.trim_matches('"'));
        }
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_owned());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "probe_id" {
            self.validated_probe = valid_probe_id(value);
        }
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

pub(super) fn push_runtime_log(logs: &Mutex<RuntimeLogBuffer>, entry: LogEntry) {
    if let Ok(mut logs) = logs.lock() {
        logs.capture(entry);
    }
}

pub(super) fn merged_logs(state_logs: &[LogEntry]) -> Vec<LogEntry> {
    let state_start = state_logs.len().saturating_sub(MAX_IN_MEMORY_LOGS);
    let runtime = RUNTIME_LOGS.lock().map_or_else(
        |_| Vec::new(),
        |logs| {
            let start = logs.len().saturating_sub(MAX_IN_MEMORY_LOGS);
            logs.entries().skip(start).cloned().collect()
        },
    );
    merge_platform_logs(state_logs[state_start..].to_vec(), &runtime)
}

pub(super) fn merge_platform_logs(local: Vec<LogEntry>, platform: &[LogEntry]) -> Vec<LogEntry> {
    // A full, older VPN buffer must not evict a newly recorded UI failure.
    // Keep both sources chronological; local entries win equal-second ties.
    let mut merged = platform.to_vec();
    for entry in local {
        if !merged.iter().any(|existing| {
            existing.level == entry.level
                && existing.message == entry.message
                && existing.timestamp == entry.timestamp
        }) {
            merged.push(entry);
        }
    }
    merged.sort_by_key(|entry| entry.timestamp.parse::<u64>().unwrap_or_default());
    if merged.len() > MAX_IN_MEMORY_LOGS {
        merged.drain(..merged.len() - MAX_IN_MEMORY_LOGS);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_runtime_filter_skips_verbose_field_formatting_and_keeps_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct ExpensiveField<'a>(&'a AtomicUsize);
        impl std::fmt::Debug for ExpensiveField<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fetch_add(1, Ordering::Relaxed);
                f.write_str("verbose TLS diagnostic")
            }
        }
        let formats = AtomicUsize::new(0);
        let logs = Arc::new(Mutex::new(RuntimeLogBuffer::default()));
        let subscriber = tracing_subscriber::registry().with(
            PawsLogLayer { logs: logs.clone() }
                .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "meow_transport::tls", field = ?ExpensiveField(&formats), "TLS trace");
            tracing::trace!(target: "meow_transport::tls", field = ?ExpensiveField(&formats), "TLS trace");
            tracing::error!(target: "paws_core::http", "profile refresh failed");
        });
        assert_eq!(formats.load(Ordering::Relaxed), 0);
        let logs = logs.lock().unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs.entries().any(|entry| entry.level == "error"));
    }

    #[test]
    fn full_vpn_log_buffer_preserves_recent_app_errors_and_deduplicates() {
        let platform: Vec<_> = (0..MAX_IN_MEMORY_LOGS)
            .map(|i| LogEntry {
                level: "info".to_owned(),
                message: format!("VPN entry {i}"),
                timestamp: "100".to_owned(),
            })
            .collect();
        for timestamp in ["100", "101"] {
            let mut error = error_log("profile refresh failed: connection refused");
            error.timestamp = timestamp.to_owned();
            let merged = merge_platform_logs(vec![platform[0].clone(), error.clone()], &platform);
            assert_eq!(merged.len(), MAX_IN_MEMORY_LOGS);
            assert!(merged.contains(&error));
            assert_eq!(merged.last(), Some(&error));
        }
    }
}

pub(super) fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

pub(super) fn system_time_secs(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().to_string())
}

struct SocketFlowFields {
    text: String,
    validated_probe: bool,
}
fn append_flow_fragment(flow: &mut String, fragment: &str) {
    if !flow.is_empty() {
        flow.push_str(" | ");
    }
    flow.push_str(fragment);
}
fn wire_trace_allowed(raw: bool, wire: bool, validated_probe: bool) -> bool {
    raw && wire && validated_probe
}
fn valid_probe_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(21..=30).contains(&bytes.len())
        || &bytes[..3] != b"tp-"
        || bytes[19] != b'-'
        || !bytes[3..19]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        || bytes[20] == b'0'
        || !bytes[20..].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    value[20..]
        .parse::<u32>()
        .is_ok_and(|id| id <= 1_000_000_000)
}
fn append_socket_trace(message: &str, flow: &str) {
    if option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC") != Some("1") {
        return;
    }
    use std::io::Write;
    static TRACE: std::sync::Mutex<Option<(std::fs::File, usize)>> = std::sync::Mutex::new(None);
    let Ok(mut state) = TRACE.lock() else {
        return;
    };
    if state.is_none() {
        let Some(root) = std::env::var_os("PAWS_HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(root)
            .join(format!("socket-trace-{}.jsonl", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o644);
        }
        let Ok(file) = options.open(path) else {
            return;
        };
        *state = Some((file, 0));
    }
    let (file, bytes) = state.as_mut().unwrap();
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let record =
        serde_json::json!({"atMs":now,"pid":std::process::id(),"flow":flow,"event":message});
    if let Ok(mut line) = serde_json::to_vec(&record) {
        line.push(b'\n');
        if *bytes + line.len() <= 4 * 1024 * 1024 && file.write_all(&line).is_ok() {
            *bytes += line.len();
        }
    }
}

#[cfg(test)]
mod socket_wire_tests {
    use super::*;
    #[test]
    fn wire_trace_requires_both_compile_gates_and_validated_probe_field() {
        for raw in [false, true] {
            for wire in [false, true] {
                for probe in [false, true] {
                    assert_eq!(wire_trace_allowed(raw, wire, probe), raw && wire && probe);
                }
            }
        }
        assert!(valid_probe_id("tp-0123456789abcdef-1"));
        assert!(valid_probe_id("tp-0123456789abcdef-1000000000"));
        for id in [
            "",
            "tp-0123456789abcdef-0",
            "tp-0123456789abcdef-01",
            "tp-0123456789abcdeF-1",
            "tp-0123456789abcdef-1000000001",
            "tp-0123456789abcdef-1 secret",
        ] {
            assert!(!valid_probe_id(id));
        }
    }
    #[test]
    fn flow_fragments_have_an_explicit_separator() {
        let mut flow = String::new();
        append_flow_fragment(&mut flow, "flow · probe_id=tp-0123456789abcdef-1");
        append_flow_fragment(&mut flow, "flow · connection=18");
        assert_eq!(
            flow,
            "flow · probe_id=tp-0123456789abcdef-1 | flow · connection=18"
        );
    }
    #[test]
    fn wire_events_never_reach_runtime_ring_or_api_even_with_spoofed_text() {
        let logs = Arc::new(Mutex::new(RuntimeLogBuffer::default()));
        let subscriber = tracing_subscriber::registry().with(PawsLogLayer { logs: logs.clone() });
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        API_LOG_TXS.lock().unwrap().push_back(tx.clone());
        tracing::subscriber::with_default(subscriber, || {
            // A target string resembling a probe field is not a structured probe_id.
            let span = tracing::info_span!("paws_flow", target = "probe_id=tp-0123456789abcdef-1");
            let _entered = span.enter();
            tracing::info!(target: "paws_core::wire_test", id=999, offset=0, n=2, hex="abcd", "socket-trace event=wire-write-sample");
            tracing::info!(target: "paws_core::wire_test", "wire-test-public-control");
        });
        API_LOG_TXS
            .lock()
            .unwrap()
            .retain(|sender| !sender.same_channel(&tx));
        let logs = logs.lock().unwrap();
        assert!(!logs
            .entries()
            .any(|entry| entry.message.contains("wire-write-sample")));
        assert!(logs
            .entries()
            .any(|entry| entry.message == "wire-test-public-control"));
        let mut public_control = false;
        for _ in 0..256 {
            let Ok(message) = rx.try_recv() else {
                break;
            };
            assert!(!message.payload.contains("wire-write-sample"));
            public_control |= message.payload == "wire-test-public-control";
        }
        assert!(public_control);
    }
}
