#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/provider_refresh_feedback.rs"]
mod provider_refresh_feedback;

use l10n::{strings, UiLocale};
use provider_refresh_feedback::provider_batch_refresh_message;

#[test]
fn provider_batch_refresh_message_reports_empty_work() {
    assert_eq!(
        provider_batch_refresh_message(0, 0, None, strings(UiLocale::ZhCn)),
        "没有可刷新的资源"
    );
    assert_eq!(
        provider_batch_refresh_message(0, 0, None, strings(UiLocale::En)),
        "No resources to refresh"
    );
}

#[test]
fn provider_batch_refresh_message_reports_full_success() {
    assert_eq!(
        provider_batch_refresh_message(2, 0, None, strings(UiLocale::ZhCn)),
        "资源已刷新：2 个成功"
    );
}

#[test]
fn provider_batch_refresh_message_reports_partial_failure() {
    assert_eq!(
        provider_batch_refresh_message(2, 1, None, strings(UiLocale::ZhCn)),
        "资源刷新完成：1 成功，1 失败"
    );
}

#[test]
fn provider_batch_refresh_message_reports_refresh_error_after_snapshot_sync() {
    assert_eq!(
        provider_batch_refresh_message(
            2,
            2,
            Some("all 2 provider refreshes failed: HTTP 500"),
            strings(UiLocale::ZhCn)
        ),
        "资源刷新完成：0 成功，2 失败：all 2 provider refreshes failed: HTTP 500"
    );
}
