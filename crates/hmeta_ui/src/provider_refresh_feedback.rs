use crate::l10n::UiStrings;

pub(crate) fn provider_batch_refresh_message(
    attempted: usize,
    failed: usize,
    error: Option<&str>,
    strings: &UiStrings,
) -> String {
    if attempted == 0 {
        return strings.feedback_provider_batch_empty.to_owned();
    }
    let failed = failed.min(attempted);
    let succeeded = attempted.saturating_sub(failed);
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
        return format!(
            "{}{}{}{}{}{}{}",
            strings.feedback_provider_batch_complete_prefix,
            succeeded,
            strings.feedback_provider_batch_success_mid,
            failed,
            strings.feedback_provider_batch_failed_suffix,
            strings.feedback_error_separator,
            error
        );
    }
    if failed == 0 {
        format!(
            "{}{}{}",
            strings.feedback_provider_batch_success_prefix,
            succeeded,
            strings.feedback_provider_batch_success_suffix
        )
    } else {
        format!(
            "{}{}{}{}{}",
            strings.feedback_provider_batch_complete_prefix,
            succeeded,
            strings.feedback_provider_batch_success_mid,
            failed,
            strings.feedback_provider_batch_failed_suffix
        )
    }
}
