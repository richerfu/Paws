use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;

pub(crate) fn profile_batch_refresh_message(
    label: &str,
    attempted: usize,
    failed: usize,
    locale: UiLocale,
) -> String {
    if attempted == 0 {
        return format!(
            "{label}{}",
            translate_ui(locale, tr::feedback_profile_batch_empty_suffix())
        );
    }
    let failed = failed.min(attempted);
    let succeeded = attempted.saturating_sub(failed);
    if failed == 0 {
        format!(
            "{label}{}{}{}",
            translate_ui(locale, tr::feedback_profile_batch_success_suffix()),
            succeeded,
            translate_ui(locale, tr::feedback_provider_batch_success_suffix())
        )
    } else {
        format!(
            "{label}{}{}{}{}{}",
            translate_ui(locale, tr::feedback_profile_batch_complete_suffix()),
            succeeded,
            translate_ui(locale, tr::feedback_provider_batch_success_mid()),
            failed,
            translate_ui(locale, tr::feedback_provider_batch_failed_suffix())
        )
    }
}

#[cfg(test)]
pub(crate) fn profile_backup_restore_message(profile_name: &str, error: Option<&str>) -> String {
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
        format!("配置 {profile_name} 回滚失败：{error}")
    } else {
        format!("配置 {profile_name} 已回滚到备份")
    }
}

pub(crate) fn profile_activation_message(
    profile_name: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    locale: UiLocale,
) -> String {
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}{}",
            translate_ui(locale, tr::feedback_profile_prefix()),
            profile_name,
            translate_ui(
                locale,
                tr::feedback_profile_activated_restart_failed_suffix()
            ),
            error
        )
    } else if restart_requested {
        format!(
            "{}{}{}",
            translate_ui(locale, tr::feedback_profile_prefix()),
            profile_name,
            translate_ui(locale, tr::feedback_profile_activated_restart_suffix())
        )
    } else {
        format!(
            "{}{}{}",
            translate_ui(locale, tr::feedback_profile_prefix()),
            profile_name,
            translate_ui(locale, tr::feedback_profile_activated_suffix())
        )
    }
}

pub(crate) fn profile_delete_message(
    profile_name: &str,
    vpn_action: Option<&str>,
    vpn_error: Option<&str>,
    locale: UiLocale,
) -> String {
    let base = format!(
        "{}{}{}",
        translate_ui(locale, tr::feedback_profile_prefix()),
        profile_name,
        translate_ui(locale, tr::feedback_profile_deleted_suffix())
    );
    if let Some(error) = vpn_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}{}",
            translate_ui(locale, tr::feedback_profile_prefix()),
            profile_name,
            translate_ui(locale, tr::feedback_profile_deleted_vpn_failed_suffix()),
            error
        )
    } else if let Some(action) = vpn_action.filter(|action| !action.trim().is_empty()) {
        format!(
            "{}{}{}{}{}",
            translate_ui(locale, tr::feedback_profile_prefix()),
            profile_name,
            translate_ui(locale, tr::feedback_profile_deleted_vpn_request_mid()),
            action,
            translate_ui(locale, tr::feedback_profile_deleted_vpn_request_suffix())
        )
    } else {
        base
    }
}

#[cfg(test)]
pub(crate) fn profile_import_message(
    profile_name: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
) -> String {
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!("配置 {profile_name} 已导入并启用，VPN 重启请求失败：{error}")
    } else if restart_requested {
        format!("配置 {profile_name} 已导入并启用，已请求重启 VPN")
    } else {
        format!("配置 {profile_name} 已导入并启用")
    }
}
