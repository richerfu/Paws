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

    assert!(
        route_row.contains("padding: 0.0") || route_row.contains("padding_left: 0.0"),
        "settings route rows must zero out native button insets"
    );
    assert!(
        route_row.contains("background_color: 0x00000000")
            || route_row.contains("background_color: surface()"),
        "settings route rows must not paint a competing surface fill"
    );
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
    assert!(page.matches("width: Some(\"46%\".into())").count() >= 2);
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

#[test]
fn network_stack_is_a_bounded_selector_with_two_real_backends() {
    let page = fs::read_to_string("src/view/pages/settings.rs").unwrap();
    let view = fs::read_to_string("src/view.rs").unwrap();
    let stack_field = page
        .split("label: tr(current.locale, \"网络栈\", \"Network stack\")")
        .nth(1)
        .expect("network stack field")
        .split("row { height: 12.0 }")
        .next()
        .unwrap();

    assert!(stack_field.contains("Select {"));
    assert!(!stack_field.contains("FlatSelect"));
    assert!(!stack_field.contains("Input"));
    assert!(page.contains("VpnStack::Smoltcp"));
    assert!(page.contains("VpnStack::Lwip"));
    assert!(page.contains("Action::SaveVpnSettings"));
    assert!(view.contains("Select,"));
}

#[test]
fn per_app_vpn_is_absent_from_ui_permissions_and_runtime_config() {
    let page = fs::read_to_string("src/view/pages/settings.rs").unwrap();
    let tools = fs::read_to_string("src/view/pages/tools.rs").unwrap();
    let route = fs::read_to_string("src/view/route.rs").unwrap();
    let entry =
        fs::read_to_string("../../entry/src/main/ets/entryability/EntryAbility.ets").unwrap();
    let module = fs::read_to_string("../../entry/src/main/module.json5").unwrap();
    let vpn_config =
        fs::read_to_string("../../entry/src/main/ets/vpnability/VpnConfig.ets").unwrap();

    assert!(!page.contains("per_app_settings_page"));
    assert!(!tools.contains("Route::PerApp"));
    assert!(!route.contains("PerApp"));
    assert!(!entry.contains("listInstalledApplications"));
    assert!(!module.contains("GET_BUNDLE_INFO"));
    assert!(!vpn_config.contains("trustedApplications"));
    assert!(!vpn_config.contains("blockedApplications"));
}
