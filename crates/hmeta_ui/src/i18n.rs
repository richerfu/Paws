//! App-level internationalization through arkit's compile-time i18n catalog.
//!
//! All user-facing messages live in `locales/*.ftl` (Fluent) and are resolved
//! through [`tr::CATALOG`]. Component code can use `use_i18n`/`t!` from the
//! arkit facade; logic handlers (non-hook contexts) use [`translate_ui`].

use arkit::i18n::i18n;

i18n! {
    pub mod tr {
        path: "locales",
        fallback: "zh-CN",
        locales: ["zh-CN", "en-US"],
    }
}

use crate::locale::UiLocale;

/// Resolve one catalog message for the given locale (non-hook context).
pub(crate) fn translate_ui(locale: UiLocale, message: arkit::i18n::TypedMessage) -> String {
    arkit::i18n::translate(&tr::CATALOG, locale.language_tag(), message)
}
