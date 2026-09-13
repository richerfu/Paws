#![allow(dead_code)]

#[path = "../src/locale.rs"]
mod locale;
#[path = "../src/ui_preferences.rs"]
mod ui_preferences;

use locale::UiLocale;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use ui_preferences::{LanguagePreference, ThemePreference, UiPreferences};

static NEXT_TEMP_PATH: AtomicU64 = AtomicU64::new(0);

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

#[test]
fn missing_preferences_are_first_run_but_corruption_is_not_defaulted() {
    let path = temporary_path();
    assert_eq!(
        UiPreferences::load_from(&path).unwrap(),
        UiPreferences::default()
    );
    fs::write(&path, "corrupted preferences").unwrap();
    assert!(UiPreferences::load_from(&path)
        .unwrap_err()
        .contains("parse UI preferences"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "corrupted preferences");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(UiPreferences::load_from(&path)
        .unwrap_err()
        .contains("read UI preferences"));
    fs::remove_dir(path).unwrap();
}

fn temporary_path() -> std::path::PathBuf {
    let sequence = NEXT_TEMP_PATH.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "paws-ui-preferences-{}-{nonce}-{sequence}.json",
        std::process::id(),
    ))
}
