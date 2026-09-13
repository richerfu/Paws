use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;
use crate::vpn_operation::VpnCommandAction;

pub(crate) fn vpn_command_message(
    action: VpnCommandAction,
    profile_name: Option<&str>,
    request_error: Option<&str>,
    locale: UiLocale,
) -> String {
    match action {
        VpnCommandAction::Start | VpnCommandAction::Restart => {
            start_message(profile_name, request_error, locale)
        }
        VpnCommandAction::Stop | VpnCommandAction::OwnedStop => stop_message(request_error, locale),
    }
}

fn start_message(
    profile_name: Option<&str>,
    request_error: Option<&str>,
    locale: UiLocale,
) -> String {
    let profile_name = profile_name.filter(|name| !name.trim().is_empty());
    let request_error = request_error.filter(|error| !error.trim().is_empty());
    match (profile_name, request_error) {
        (Some(profile_name), Some(error)) => {
            format!(
                "{}{}{}{}",
                translate_ui(locale, tr::feedback_vpn_start_loaded_prefix()),
                profile_name,
                translate_ui(locale, tr::feedback_vpn_start_loaded_failed_suffix()),
                error
            )
        }
        (Some(profile_name), None) => {
            format!(
                "{}{}",
                translate_ui(locale, tr::feedback_vpn_start_requested_prefix()),
                profile_name
            )
        }
        (None, Some(error)) => format!(
            "{}{}",
            translate_ui(locale, tr::feedback_vpn_start_failed_prefix()),
            error
        ),
        (None, None) => translate_ui(locale, tr::feedback_vpn_start_requested()),
    }
}

fn stop_message(request_error: Option<&str>, locale: UiLocale) -> String {
    if let Some(error) = request_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}",
            translate_ui(locale, tr::feedback_vpn_stop_failed_prefix()),
            error
        )
    } else {
        translate_ui(locale, tr::feedback_vpn_stop_requested())
    }
}
