#![allow(dead_code)]

#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/ui_preferences.rs"]
mod ui_preferences;

use l10n::UiLocale;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use ui_preferences::{LanguagePreference, ThemePreference, UiPreferences};

#[test]
fn preferences_round_trip_and_keep_system_defaults() {
    let path = temporary_path();
    let preferences = UiPreferences {
        language: LanguagePreference::En,
        theme: ThemePreference::Dark,
    };

    preferences.save_to(&path).unwrap();
    assert_eq!(UiPreferences::load_from(&path).unwrap(), preferences);

    fs::write(&path, "{}").unwrap();
    assert_eq!(
        UiPreferences::load_from(&path).unwrap(),
        UiPreferences::default()
    );
    let _ = fs::remove_file(path);
}

#[test]
fn system_preferences_resolve_harmony_configuration_values() {
    assert_eq!(LanguagePreference::System.resolve("en-US"), UiLocale::En);
    assert_eq!(
        LanguagePreference::System.resolve("zh-Hans"),
        UiLocale::ZhCn
    );
    assert!(ThemePreference::System.resolve_dark(0));
    assert!(!ThemePreference::System.resolve_dark(1));
    assert_eq!(ThemePreference::System.platform_color_mode(), -1);
    assert_eq!(ThemePreference::Dark.platform_color_mode(), 0);
    assert_eq!(ThemePreference::Light.platform_color_mode(), 1);
}

fn temporary_path() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "paws-ui-preferences-{}-{nonce}.json",
        std::process::id()
    ))
}
