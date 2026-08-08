use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;

pub(crate) fn rule_import_message(
    imported_count: usize,
    reload_error: Option<&str>,
    restart_requested: bool,
    restart_error: Option<&str>,
    locale: UiLocale,
) -> String {
    let base = format!(
        "{}{}{}",
        translate_ui(locale, tr::feedback_rule_imported_prefix()),
        imported_count,
        translate_ui(locale, tr::feedback_rule_imported_suffix())
    );
    if let Some(error) = reload_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}{error}",
            translate_ui(locale, tr::feedback_clause_separator()),
            translate_ui(locale, tr::feedback_rule_reload_failed_suffix())
        )
    } else if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}{error}",
            translate_ui(locale, tr::feedback_clause_separator()),
            translate_ui(locale, tr::feedback_vpn_restart_failed_suffix())
        )
    } else if restart_requested {
        format!(
            "{base}{}{}",
            translate_ui(locale, tr::feedback_clause_separator()),
            translate_ui(locale, tr::feedback_vpn_restart_requested_suffix())
        )
    } else {
        base
    }
}
