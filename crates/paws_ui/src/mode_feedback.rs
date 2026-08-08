use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;
use paws_model::RuntimeMode;

pub(crate) fn mode_changed_message(mode: RuntimeMode, locale: UiLocale) -> String {
    format!(
        "{}{}",
        translate_ui(locale, tr::feedback_mode_changed_prefix()),
        mode_label(mode, locale)
    )
}

pub(crate) fn mode_label(mode: RuntimeMode, locale: UiLocale) -> String {
    match mode {
        RuntimeMode::Rule => translate_ui(locale, tr::dashboard_mode_rule()),
        RuntimeMode::Global => translate_ui(locale, tr::dashboard_mode_global()),
        RuntimeMode::Direct => translate_ui(locale, tr::dashboard_mode_direct()),
    }
}
