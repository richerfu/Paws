use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;

pub(crate) fn provider_batch_refresh_message(
    attempted: usize,
    failed: usize,
    error: Option<&str>,
    locale: UiLocale,
) -> String {
    if attempted == 0 {
        return translate_ui(locale, tr::feedback_provider_batch_empty());
    }
    let failed = failed.min(attempted);
    let succeeded = attempted.saturating_sub(failed);
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
        return format!(
            "{}{}{}{}{}{}{}",
            translate_ui(locale, tr::feedback_provider_batch_complete_prefix()),
            succeeded,
            translate_ui(locale, tr::feedback_provider_batch_success_mid()),
            failed,
            translate_ui(locale, tr::feedback_provider_batch_failed_suffix()),
            translate_ui(locale, tr::feedback_error_separator()),
            error
        );
    }
    if failed == 0 {
        format!(
            "{}{}{}",
            translate_ui(locale, tr::feedback_provider_batch_success_prefix()),
            succeeded,
            translate_ui(locale, tr::feedback_provider_batch_success_suffix())
        )
    } else {
        format!(
            "{}{}{}{}{}",
            translate_ui(locale, tr::feedback_provider_batch_complete_prefix()),
            succeeded,
            translate_ui(locale, tr::feedback_provider_batch_success_mid()),
            failed,
            translate_ui(locale, tr::feedback_provider_batch_failed_suffix())
        )
    }
}
