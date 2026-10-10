use super::*;

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
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let target = event.metadata().target();
        // dioxus's internal diff/template traces carry the entire Template
        // tree as a debug field; formatting it on the UI thread for every
        // rendered node stalls the main loop (observed as a 6s+ ANR on
        // device). These internals are worthless for the app log anyway.
        if target.starts_with("dioxus") {
            return;
        }
        let level = match *event.metadata().level() {
            Level::TRACE | Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warning",
            Level::ERROR => "error",
        };
        let mut visitor = LogMessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.finish(target);
        if is_vpn_log_target(event.metadata().target()) {
            push_runtime_log(
                &self.logs,
                LogEntry {
                    level: level.to_owned(),
                    message,
                    timestamp: unix_timestamp_string(),
                },
            );
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
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_owned());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
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
