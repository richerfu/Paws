#![allow(dead_code)]

#[path = "../src/locale.rs"]
mod locale;

use locale::UiLocale;

#[test]
fn locale_parser_prefers_english_for_en_tags() {
    assert_eq!(UiLocale::from_language_tag("en-US"), UiLocale::En);
    assert_eq!(UiLocale::from_language_tag("EN"), UiLocale::En);
}

#[test]
fn locale_parser_defaults_to_zh_cn() {
    assert_eq!(UiLocale::from_language_tag("zh-Hans-CN"), UiLocale::ZhCn);
    assert_eq!(UiLocale::from_language_tag("fr-FR"), UiLocale::ZhCn);
}

#[test]
fn language_tags_roundtrip() {
    assert_eq!(UiLocale::ZhCn.language_tag(), "zh-CN");
    assert_eq!(UiLocale::En.language_tag(), "en-US");
}
