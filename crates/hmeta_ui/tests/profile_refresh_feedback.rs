#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/profile_refresh_feedback.rs"]
mod profile_refresh_feedback;

use l10n::{strings, UiLocale};
use profile_refresh_feedback::{
    profile_activation_message, profile_backup_restore_message, profile_batch_refresh_message,
    profile_delete_message, profile_import_message,
};

#[test]
fn profile_batch_refresh_message_reports_empty_work() {
    assert_eq!(
        profile_batch_refresh_message("到期订阅", 0, 0, strings(UiLocale::ZhCn)),
        "到期订阅没有可刷新的订阅"
    );
    assert_eq!(
        profile_batch_refresh_message("Due subscriptions", 0, 0, strings(UiLocale::En)),
        "Due subscriptions has no subscriptions to refresh"
    );
}

#[test]
fn profile_batch_refresh_message_reports_full_success() {
    assert_eq!(
        profile_batch_refresh_message("全部订阅", 3, 0, strings(UiLocale::ZhCn)),
        "全部订阅已刷新：3 个成功"
    );
}

#[test]
fn profile_batch_refresh_message_reports_partial_failure() {
    assert_eq!(
        profile_batch_refresh_message("全部订阅", 3, 1, strings(UiLocale::ZhCn)),
        "全部订阅刷新完成：2 成功，1 失败"
    );
}

#[test]
fn profile_backup_restore_message_reports_success() {
    assert_eq!(
        profile_backup_restore_message("Work", None),
        "配置 Work 已回滚到备份"
    );
}

#[test]
fn profile_backup_restore_message_reports_failure() {
    assert_eq!(
        profile_backup_restore_message("Work", Some("profile id has no backup")),
        "配置 Work 回滚失败：profile id has no backup"
    );
}

#[test]
fn profile_activation_message_reports_plain_success() {
    assert_eq!(
        profile_activation_message("Work", false, None, strings(UiLocale::ZhCn)),
        "配置 Work 已启用"
    );
    assert_eq!(
        profile_activation_message("Work", false, None, strings(UiLocale::En)),
        "Profile Work activated"
    );
}

#[test]
fn profile_activation_message_reports_restart_request() {
    assert_eq!(
        profile_activation_message("Work", true, None, strings(UiLocale::ZhCn)),
        "配置 Work 已启用，已请求重启 VPN"
    );
}

#[test]
fn profile_activation_message_reports_restart_failure() {
    assert_eq!(
        profile_activation_message(
            "Work",
            true,
            Some("启动回调失败：VPN denied"),
            strings(UiLocale::ZhCn)
        ),
        "配置 Work 已启用，VPN 重启请求失败：启动回调失败：VPN denied"
    );
}

#[test]
fn profile_delete_message_reports_plain_success() {
    assert_eq!(
        profile_delete_message("Work", None, None, strings(UiLocale::ZhCn)),
        "配置 Work 已删除"
    );
}

#[test]
fn profile_delete_message_reports_stop_request() {
    assert_eq!(
        profile_delete_message("Work", Some("停止"), None, strings(UiLocale::ZhCn)),
        "配置 Work 已删除，已请求停止 VPN"
    );
}

#[test]
fn profile_delete_message_reports_restart_request() {
    assert_eq!(
        profile_delete_message("Work", Some("重启"), None, strings(UiLocale::ZhCn)),
        "配置 Work 已删除，已请求重启 VPN"
    );
    assert_eq!(
        profile_delete_message("Work", Some("restart"), None, strings(UiLocale::En)),
        "Profile Work deleted; VPN restart requested"
    );
}

#[test]
fn profile_delete_message_reports_vpn_error_after_delete() {
    assert_eq!(
        profile_delete_message(
            "Work",
            Some("重启"),
            Some("启动回调失败：VPN denied"),
            strings(UiLocale::ZhCn)
        ),
        "配置 Work 已删除，VPN 状态更新失败：启动回调失败：VPN denied"
    );
}

#[test]
fn profile_import_message_reports_plain_success() {
    assert_eq!(
        profile_import_message("Work", false, None),
        "配置 Work 已导入并启用"
    );
}

#[test]
fn profile_import_message_reports_restart_request() {
    assert_eq!(
        profile_import_message("Work", true, None),
        "配置 Work 已导入并启用，已请求重启 VPN"
    );
}

#[test]
fn profile_import_message_reports_restart_failure() {
    assert_eq!(
        profile_import_message("Work", true, Some("启动回调失败：VPN denied")),
        "配置 Work 已导入并启用，VPN 重启请求失败：启动回调失败：VPN denied"
    );
}
