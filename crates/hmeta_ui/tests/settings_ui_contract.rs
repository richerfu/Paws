use std::fs;

#[test]
fn settings_route_rows_do_not_inherit_native_button_insets() {
    let source = fs::read_to_string("src/view/pages/tools.rs").unwrap();
    let route_row = source
        .split("fn settings_route_row")
        .nth(1)
        .unwrap()
        .split("fn settings_value_row")
        .next()
        .unwrap();

    for inset in [
        "padding_left: 0.0",
        "padding_right: 0.0",
        "padding_top: 0.0",
        "padding_bottom: 0.0",
    ] {
        assert!(
            route_row.contains(inset),
            "settings route rows must align with value rows: missing {inset}"
        );
    }
}

#[test]
fn every_route_owns_its_scroll_node() {
    let source = fs::read_to_string("src/view.rs").unwrap();
    assert!(source.contains("page-scroll-{page:?}"));
    assert!(source.contains("key: \"{scroll_key}\""));
}

#[test]
fn about_page_constrains_long_values_and_aligns_repositories() {
    let page = fs::read_to_string("src/view/pages/tools.rs").unwrap();
    let view = fs::read_to_string("src/view.rs").unwrap();

    assert!(page.contains("middle_truncate_text(&about.arkit_rev, 18)"));
    assert!(page.matches("percent_width: 0.46").count() >= 2);
    assert!(page.matches("width: 18.0").count() >= 2);
    assert!(page.contains("layout_weight: 1.0"));
    assert!(view.contains("fn middle_truncate_text"));
    assert!(view.contains("max_lines: 1"));
}

#[test]
fn appearance_settings_use_arkit_shadcn_choices_and_persist_actions() {
    let page = fs::read_to_string("src/view/pages/appearance.rs").unwrap();
    let view = fs::read_to_string("src/view.rs").unwrap();
    let platform = fs::read_to_string("src/platform_callbacks.rs").unwrap();
    let entry =
        fs::read_to_string("../../entry/src/main/ets/entryability/EntryAbility.ets").unwrap();

    assert!(page.contains("RadioGroup"));
    assert!(page.contains("Action::SetLanguagePreference"));
    assert!(page.contains("Action::SetThemePreference"));
    assert!(!page.contains("button {"));
    assert!(view.contains("ThemeProvider"));
    assert!(view.contains("use_theme().colors"));
    assert!(platform.contains("set_color_mode"));
    assert!(entry.contains("this.context.setColorMode"));
}
