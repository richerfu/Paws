use paws_model::LogEntry;

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
    let query = normalize_log_query(query);
    matches_log_filter_normalized(log, filter, &query)
}

/// Normalize the query once per UI update instead of once for every log row.
pub(crate) fn normalize_log_query(query: &str) -> String {
    query.trim().to_ascii_lowercase()
}

pub(crate) fn matches_log_filter_normalized(
    log: &LogEntry,
    filter: LogLevelFilter,
    normalized_query: &str,
) -> bool {
    if let Some(level) = filter.level() {
        if !log.level.eq_ignore_ascii_case(level) {
            return false;
        }
    }

    if normalized_query.is_empty() {
        return true;
    }
    log.level.to_ascii_lowercase().contains(normalized_query)
        || log.message.to_ascii_lowercase().contains(normalized_query)
        || log
            .timestamp
            .to_ascii_lowercase()
            .contains(normalized_query)
}
