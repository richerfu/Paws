#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/vpn_feedback.rs"]
mod vpn_feedback;

use hmeta_model::VpnLifecycle;
use l10n::{strings, UiLocale};
use vpn_feedback::{vpn_command_is_pending, vpn_command_message, VpnCommandAction};

#[test]
fn vpn_start_message_names_profile() {
    assert_eq!(
        vpn_command_message(
            VpnCommandAction::Start,
            Some("Work"),
            None,
            strings(UiLocale::ZhCn)
        ),
        "已请求启动 VPN：Work"
    );
    assert_eq!(
        vpn_command_message(
            VpnCommandAction::Start,
            Some("Work"),
            None,
            strings(UiLocale::En)
        ),
        "VPN start requested: Work"
    );
}

#[test]
fn vpn_start_message_keeps_loaded_profile_when_request_fails() {
    assert_eq!(
        vpn_command_message(
            VpnCommandAction::Start,
            Some("Work"),
            Some("VPN start callback is not registered"),
            strings(UiLocale::ZhCn)
        ),
        "配置 Work 已加载，VPN 启动请求失败：VPN start callback is not registered"
    );
}

#[test]
fn vpn_stop_message_reports_requested_stop() {
    assert_eq!(
        vpn_command_message(VpnCommandAction::Stop, None, None, strings(UiLocale::ZhCn)),
        "已请求停止 VPN"
    );
}

#[test]
fn vpn_stop_message_reports_callback_fallback() {
    assert_eq!(
        vpn_command_message(
            VpnCommandAction::Stop,
            None,
            Some("VPN stop callback is not registered"),
            strings(UiLocale::ZhCn)
        ),
        "VPN 停止回调失败，已回退本地停止：VPN stop callback is not registered"
    );
}

#[test]
fn vpn_start_stays_pending_across_stale_polled_snapshots() {
    for lifecycle in [
        VpnLifecycle::Stopped,
        VpnLifecycle::EngineLoaded,
        VpnLifecycle::Starting,
    ] {
        assert!(vpn_command_is_pending(
            VpnCommandAction::Start,
            lifecycle,
            false,
        ));
    }
}

#[test]
fn vpn_start_finishes_only_on_success_or_terminal_failure() {
    assert!(!vpn_command_is_pending(
        VpnCommandAction::Start,
        VpnLifecycle::Connected,
        true,
    ));
    assert!(!vpn_command_is_pending(
        VpnCommandAction::Start,
        VpnLifecycle::Failed,
        false,
    ));
    assert!(!vpn_command_is_pending(
        VpnCommandAction::Start,
        VpnLifecycle::ProtectFailed,
        false,
    ));
}

#[test]
fn vpn_stop_stays_pending_until_platform_is_stopped() {
    assert!(vpn_command_is_pending(
        VpnCommandAction::Stop,
        VpnLifecycle::Connected,
        true,
    ));
    assert!(!vpn_command_is_pending(
        VpnCommandAction::Stop,
        VpnLifecycle::EngineLoaded,
        false,
    ));
}
