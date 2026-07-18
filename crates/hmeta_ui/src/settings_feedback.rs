use crate::l10n::UiStrings;

pub(crate) fn settings_saved_message(
    label: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}",
            label, strings.feedback_settings_saved_restart_failed_suffix, error
        )
    } else if restart_requested {
        format!(
            "{}{}",
            label, strings.feedback_settings_saved_restart_suffix
        )
    } else {
        format!("{}{}", label, strings.feedback_settings_saved_suffix)
    }
}
