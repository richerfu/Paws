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

pub(super) fn install_runtime_log_layer() {
    INSTALL_RUNTIME_LOG_LAYER.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(HMetaLogLayer {
            logs: RUNTIME_LOGS.clone(),
        });
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

pub(super) struct HMetaLogLayer {
    logs: Arc<Mutex<RuntimeLogBuffer>>,
}

impl<S> Layer<S> for HMetaLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = match *event.metadata().level() {
            Level::TRACE | Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warning",
            Level::ERROR => "error",
        };
        let mut visitor = LogMessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.finish(event.metadata().target());
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
    target.starts_with("hmeta_core")
        || target.starts_with("hmeta_vpn")
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
    let mut logs: Vec<_> = state_logs[state_start..].to_vec();
    let remaining = MAX_IN_MEMORY_LOGS.saturating_sub(logs.len());
    if let Ok(runtime_logs) = RUNTIME_LOGS.lock() {
        let runtime_start = runtime_logs.len().saturating_sub(remaining);
        logs.extend(runtime_logs.entries().skip(runtime_start).cloned());
    }
    logs
}

pub(super) fn merge_platform_logs(
    mut local: Vec<LogEntry>,
    platform: &[LogEntry],
) -> Vec<LogEntry> {
    for entry in platform {
        if !local.iter().any(|existing| {
            existing.level == entry.level
                && existing.message == entry.message
                && existing.timestamp == entry.timestamp
        }) {
            local.push(entry.clone());
        }
    }
    if local.len() > MAX_IN_MEMORY_LOGS {
        local.drain(..local.len() - MAX_IN_MEMORY_LOGS);
    }
    local
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
