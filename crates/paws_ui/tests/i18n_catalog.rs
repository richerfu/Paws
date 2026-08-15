//! Contract: every `UiLocale::language_tag()` id must be declared in the
//! `tr::CATALOG` `locales:` list (src/i18n.rs), and `translate_ui` must route
//! through `language_tag()` so the two stay in sync.
//!
//! Regression: `translate_ui` resolved `UiLocale::En` to `"en"` while the
//! catalog only ships `zh-CN`/`en-US` (fallback `zh-CN`). The exact-match
//! lookup missed and silently fell back to Chinese, so switching to English
//! appeared to have no effect.

#[path = "../src/locale.rs"]
mod locale;

use locale::UiLocale;

const I18N_SOURCE: &str = include_str!("../src/i18n.rs");

#[test]
fn every_ui_locale_tag_is_declared_in_the_catalog() {
    let start = I18N_SOURCE
        .find("locales:")
        .expect("catalog `locales:` list");
    let tail = &I18N_SOURCE[start..];
    let open = tail.find('[').expect("catalog locales array");
    let close = tail[open..].find(']').expect("catalog locales close") + open;
    let declared = &tail[open + 1..close];

    for id in [UiLocale::ZhCn.language_tag(), UiLocale::En.language_tag()] {
        assert!(
            declared.contains(&format!("\"{id}\"")),
            "catalog locales {declared:?} is missing `{id}`"
        );
    }
}

#[test]
fn translate_ui_routes_through_language_tag() {
    assert!(
        I18N_SOURCE.contains("locale.language_tag()"),
        "translate_ui must use `locale.language_tag()` as the catalog locale id"
    );
}
