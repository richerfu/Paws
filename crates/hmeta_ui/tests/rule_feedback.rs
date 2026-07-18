#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/rule_feedback.rs"]
mod rule_feedback;

use l10n::{strings, UiLocale};
use rule_feedback::rule_import_message;

#[test]
fn rule_import_message_reports_plain_success() {
    assert_eq!(
        rule_import_message(3, None, false, None, strings(UiLocale::ZhCn)),
        "已导入 3 条规则"
    );
    assert_eq!(
        rule_import_message(3, None, false, None, strings(UiLocale::En)),
        "Imported 3 rules"
    );
}

#[test]
fn rule_import_message_reports_reload_failure_after_import() {
    assert_eq!(
        rule_import_message(
            2,
            Some("invalid rule"),
            false,
            None,
            strings(UiLocale::ZhCn)
        ),
        "已导入 2 条规则，重新加载失败：invalid rule"
    );
}

#[test]
fn rule_import_message_reports_restart_request() {
    assert_eq!(
        rule_import_message(1, None, true, None, strings(UiLocale::ZhCn)),
        "已导入 1 条规则，已请求重启 VPN"
    );
}

#[test]
fn rule_import_message_reports_restart_failure() {
    assert_eq!(
        rule_import_message(
            1,
            None,
            true,
            Some("启动回调失败：VPN denied"),
            strings(UiLocale::ZhCn)
        ),
        "已导入 1 条规则，VPN 重启请求失败：启动回调失败：VPN denied"
    );
}
