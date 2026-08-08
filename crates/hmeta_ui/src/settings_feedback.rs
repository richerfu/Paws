use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;

pub(crate) fn settings_saved_message(
    label: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    locale: UiLocale,
) -> String {
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}",
            label,
            translate_ui(locale, tr::feedback_settings_saved_restart_failed_suffix()),
            error
        )
    } else if restart_requested {
        format!(
            "{}{}",
            label,
            translate_ui(locale, tr::feedback_settings_saved_restart_suffix())
        )
    } else {
        format!(
            "{}{}",
            label,
            translate_ui(locale, tr::feedback_settings_saved_suffix())
        )
    }
}
