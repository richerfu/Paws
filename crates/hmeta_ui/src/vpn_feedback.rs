use crate::l10n::UiStrings;
use hmeta_model::VpnLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VpnCommandAction {
    Start,
    Stop,
}

/// Keeps an in-flight command visible until the platform reports a terminal
/// state. Snapshot polling can briefly return the state from before the
/// command, so `Stopped` and `EngineLoaded` are not terminal while starting.
pub(crate) fn vpn_command_is_pending(
    action: VpnCommandAction,
    lifecycle: VpnLifecycle,
    vpn_running: bool,
) -> bool {
    match action {
        VpnCommandAction::Start => {
            !vpn_running
                && !matches!(
                    lifecycle,
                    VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
                )
        }
        VpnCommandAction::Stop => vpn_running || matches!(lifecycle, VpnLifecycle::Starting),
    }
}

pub(crate) fn vpn_command_message(
    action: VpnCommandAction,
    profile_name: Option<&str>,
    request_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    match action {
        VpnCommandAction::Start => start_message(profile_name, request_error, strings),
        VpnCommandAction::Stop => stop_message(request_error, strings),
    }
}

fn start_message(
    profile_name: Option<&str>,
    request_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    let profile_name = profile_name.filter(|name| !name.trim().is_empty());
    let request_error = request_error.filter(|error| !error.trim().is_empty());
    match (profile_name, request_error) {
        (Some(profile_name), Some(error)) => {
            format!(
                "{}{}{}{}",
                strings.feedback_vpn_start_loaded_prefix,
                profile_name,
                strings.feedback_vpn_start_loaded_failed_suffix,
                error
            )
        }
        (Some(profile_name), None) => {
            format!(
                "{}{}",
                strings.feedback_vpn_start_requested_prefix, profile_name
            )
        }
        (None, Some(error)) => format!("{}{}", strings.feedback_vpn_start_failed_prefix, error),
        (None, None) => strings.feedback_vpn_start_requested.to_owned(),
    }
}

fn stop_message(request_error: Option<&str>, strings: &UiStrings) -> String {
    if let Some(error) = request_error.filter(|error| !error.trim().is_empty()) {
        format!("{}{}", strings.feedback_vpn_stop_fallback_prefix, error)
    } else {
        strings.feedback_vpn_stop_requested.to_owned()
    }
}
