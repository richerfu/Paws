#[path = "../src/log_filter.rs"]
mod log_filter;

use hmeta_model::LogEntry;
use log_filter::{matches_log_filter, LogLevelFilter};

fn log(level: &str, message: &str, timestamp: &str) -> LogEntry {
    LogEntry {
        level: level.to_owned(),
        message: message.to_owned(),
        timestamp: timestamp.to_owned(),
    }
}

#[test]
fn filters_logs_by_level() {
    let info = log("info", "vpn started", "100");
    let warning = log("warning", "provider refresh failed", "101");

    assert!(matches_log_filter(&info, LogLevelFilter::All, ""));
    assert!(matches_log_filter(&info, LogLevelFilter::Info, ""));
    assert!(!matches_log_filter(&info, LogLevelFilter::Warning, ""));
    assert!(matches_log_filter(&warning, LogLevelFilter::Warning, ""));
    assert!(!matches_log_filter(&warning, LogLevelFilter::Debug, ""));
}

#[test]
fn filters_logs_by_case_insensitive_query() {
    let entry = log("warning", "Provider refresh failed", "1893456000");

    assert!(matches_log_filter(&entry, LogLevelFilter::All, "provider"));
    assert!(matches_log_filter(&entry, LogLevelFilter::All, "WARNING"));
    assert!(matches_log_filter(&entry, LogLevelFilter::All, "189345"));
    assert!(!matches_log_filter(&entry, LogLevelFilter::All, "dns"));
}

#[test]
fn combines_level_and_query_filters() {
    let entry = log("error", "dns upstream failed", "102");

    assert!(matches_log_filter(&entry, LogLevelFilter::Error, "dns"));
    assert!(!matches_log_filter(&entry, LogLevelFilter::Warning, "dns"));
    assert!(!matches_log_filter(
        &entry,
        LogLevelFilter::Error,
        "provider"
    ));
}

#[test]
fn exposes_all_level_filters_for_ui_segments() {
    assert_eq!(
        LogLevelFilter::ALL,
        [
            LogLevelFilter::All,
            LogLevelFilter::Info,
            LogLevelFilter::Warning,
            LogLevelFilter::Error,
            LogLevelFilter::Debug
        ]
    );
}
