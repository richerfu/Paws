use crate::l10n::UiStrings;

pub(crate) fn rule_import_message(
    imported_count: usize,
    reload_error: Option<&str>,
    restart_requested: bool,
    restart_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    let base = format!(
        "{}{}{}",
        strings.feedback_rule_imported_prefix,
        imported_count,
        strings.feedback_rule_imported_suffix
    );
    if let Some(error) = reload_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}{error}",
            strings.feedback_clause_separator, strings.feedback_rule_reload_failed_suffix
        )
    } else if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}{error}",
            strings.feedback_clause_separator, strings.feedback_vpn_restart_failed_suffix
        )
    } else if restart_requested {
        format!(
            "{base}{}{}",
            strings.feedback_clause_separator, strings.feedback_vpn_restart_requested_suffix
        )
    } else {
        base
    }
}
