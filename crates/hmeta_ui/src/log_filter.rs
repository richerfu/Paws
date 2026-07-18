use hmeta_model::LogEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LogLevelFilter {
    #[default]
    All,
    Info,
    Warning,
    Error,
    Debug,
}

impl LogLevelFilter {
    pub(crate) const ALL: [Self; 5] = [
        Self::All,
        Self::Info,
        Self::Warning,
        Self::Error,
        Self::Debug,
    ];

    fn level(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Info => Some("info"),
            Self::Warning => Some("warning"),
            Self::Error => Some("error"),
            Self::Debug => Some("debug"),
        }
    }
}

pub(crate) fn matches_log_filter(log: &LogEntry, filter: LogLevelFilter, query: &str) -> bool {
    if let Some(level) = filter.level() {
        if !log.level.eq_ignore_ascii_case(level) {
            return false;
        }
    }

    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_ascii_lowercase();
    log.level.to_ascii_lowercase().contains(&query)
        || log.message.to_ascii_lowercase().contains(&query)
        || log.timestamp.to_ascii_lowercase().contains(&query)
}
