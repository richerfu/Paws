use crate::i18n::{translate_ui, tr};
use crate::locale::UiLocale;
use hmeta_model::GeodataFileSummary;

pub(crate) fn geodata_readiness(
    locale: UiLocale,
    files: &[GeodataFileSummary],
) -> (String, String) {
    if files.is_empty() {
        return (
            translate_ui(locale, tr::hard_zh_003()),
            translate_ui(locale, tr::hard_zh_004()),
        );
    }

    let missing = files
        .iter()
        .filter(|file| !file.exists)
        .map(|file| file.name.as_str())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        (
            translate_ui(locale, tr::hard_zh_005()),
            translate_ui(locale, tr::hard_zh_001(files.len())),
        )
    } else {
        (
            translate_ui(locale, tr::hard_zh_006()),
            translate_ui(locale, tr::hard_zh_002(missing.join(", "))),
        )
    }
}
