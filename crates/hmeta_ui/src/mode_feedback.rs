use crate::l10n::UiStrings;
use hmeta_model::RuntimeMode;

pub(crate) fn mode_changed_message(mode: RuntimeMode, strings: &UiStrings) -> String {
    format!(
        "{}{}",
        strings.feedback_mode_changed_prefix,
        mode_label(mode, strings)
    )
}

pub(crate) fn mode_label(mode: RuntimeMode, strings: &UiStrings) -> &'static str {
    match mode {
        RuntimeMode::Rule => strings.dashboard_mode_rule,
        RuntimeMode::Global => strings.dashboard_mode_global,
        RuntimeMode::Direct => strings.dashboard_mode_direct,
    }
}
