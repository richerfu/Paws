#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/settings_feedback.rs"]
mod settings_feedback;

use l10n::{strings, UiLocale};
use settings_feedback::settings_saved_message;

#[test]
fn settings_saved_message_mentions_restart_when_running() {
    assert_eq!(
        settings_saved_message("DNS 设置", true, None, strings(UiLocale::ZhCn)),
        "DNS 设置已保存，已请求重启 VPN"
    );
    assert_eq!(
        settings_saved_message("DNS settings", true, None, strings(UiLocale::En)),
        "DNS settings saved; VPN restart requested"
    );
}

#[test]
fn settings_saved_message_keeps_save_success_when_restart_fails() {
    assert_eq!(
        settings_saved_message(
            "VPN 设置",
            true,
            Some("VPN start callback is not registered"),
            strings(UiLocale::ZhCn)
        ),
        "VPN 设置已保存，VPN 重启请求失败：VPN start callback is not registered"
    );
}

#[test]
fn settings_saved_message_uses_plain_save_when_vpn_is_stopped() {
    assert_eq!(
        settings_saved_message("分应用 VPN 设置", false, None, strings(UiLocale::ZhCn)),
        "分应用 VPN 设置已保存"
    );
}

#[test]
fn settings_saved_message_can_describe_rule_restart_status() {
    assert_eq!(
        settings_saved_message(
            "规则",
            true,
            Some("启动回调失败：VPN denied"),
            strings(UiLocale::ZhCn)
        ),
        "规则已保存，VPN 重启请求失败：启动回调失败：VPN denied"
    );
}
