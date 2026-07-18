#[path = "../src/installed_app_filter.rs"]
mod installed_app_filter;

use hmeta_model::InstalledApplication;
use installed_app_filter::{matches_installed_application_query, normalize_installed_applications};

fn app(name: &str, bundle_name: &str) -> InstalledApplication {
    InstalledApplication {
        name: name.to_owned(),
        bundle_name: bundle_name.to_owned(),
    }
}

#[test]
fn installed_app_filter_matches_name_and_bundle() {
    let item = app("Browser", "com.example.browser");

    assert!(matches_installed_application_query(&item, ""));
    assert!(matches_installed_application_query(&item, "browser"));
    assert!(matches_installed_application_query(&item, "EXAMPLE"));
    assert!(!matches_installed_application_query(&item, "music"));
}

#[test]
fn installed_app_filter_trims_query() {
    let item = app("Chat", "com.example.chat");

    assert!(matches_installed_application_query(&item, " chat "));
}

#[test]
fn installed_applications_are_normalized_for_settings_picker() {
    let applications = normalize_installed_applications(vec![
        app("  Zebra  ", " com.example.zebra "),
        app("Alpha Duplicate", "com.example.zebra"),
        app("", " com.example.browser "),
        app("Music", ""),
        app("Alpha", "com.example.alpha"),
    ]);

    assert_eq!(applications.len(), 3);
    assert_eq!(applications[0], app("Alpha", "com.example.alpha"));
    assert_eq!(
        applications[1],
        app("com.example.browser", "com.example.browser")
    );
    assert_eq!(applications[2], app("Zebra", "com.example.zebra"));
}
