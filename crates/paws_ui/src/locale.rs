//! Locale state type shared by the app runtime (pure Rust, no arkit link).

/// Locale state used by the app runtime (profile preference, platform sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiLocale {
    ZhCn,
    En,
}

impl UiLocale {
    pub(crate) fn from_language_tag(tag: &str) -> Self {
        if tag.to_ascii_lowercase().starts_with("en") {
            Self::En
        } else {
            Self::ZhCn
        }
    }

    /// The catalog locale id (matches `tr::CATALOG` in `i18n.rs`).
    pub(crate) fn language_tag(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en-US",
        }
    }
}
