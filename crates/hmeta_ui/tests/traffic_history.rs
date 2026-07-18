#[path = "../src/traffic_history.rs"]
mod traffic_history;

use traffic_history::summarize_traffic_history;

#[test]
fn traffic_history_summary_reports_peaks_and_latest_sample() {
    let summary = summarize_traffic_history(&[(100, 20), (40, 90), (80, 10)]).expect("summary");

    assert_eq!(summary.samples, 3);
    assert_eq!(summary.peak_download, 100);
    assert_eq!(summary.peak_upload, 90);
    assert_eq!(summary.latest_download, 80);
    assert_eq!(summary.latest_upload, 10);
}

#[test]
fn traffic_history_summary_handles_empty_history() {
    assert!(summarize_traffic_history(&[]).is_none());
}
