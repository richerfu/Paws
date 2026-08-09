use std::fs;

#[test]
fn application_root_delegates_safe_area_to_arkts_host() {
    let source = fs::read_to_string("src/view.rs").unwrap();
    let ability =
        fs::read_to_string("../../entry/src/main/ets/entryability/EntryAbility.ets").unwrap();
    let page = fs::read_to_string("../../entry/src/main/ets/pages/Index.ets").unwrap();
    let app = source
        .split("pub(crate) fn App()")
        .nth(1)
        .unwrap()
        .split("fn AppShell()")
        .next()
        .unwrap();

    assert!(!app.contains("SafeArea {"));
    assert!(app.contains("ThemeProvider {"));
    assert!(ability.contains("initializeSafeArea(win);"));
    assert!(
        ability.find("initializeSafeArea(win);").unwrap()
            < ability.find("setUIContent('pages/Index')").unwrap()
    );
    assert!(page.contains("getSafeAreaInsets()"));
    assert!(page.contains(".padding({"));
}

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
    // RouteProvider restores the route's saved scroll position when the
    // page is mounted again after navigation back.
    assert!(source.contains("RouteProvider {"));
    assert!(!source.contains("page-scroll-{page:?}"));
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
fn about_page_has_one_privacy_entry_and_detail_page_discloses_exit_ip_providers() {
    let page = fs::read_to_string("src/view/pages/tools.rs").unwrap();
    let route = fs::read_to_string("src/view/route.rs").unwrap();
    let about = page
        .split("pub(crate) fn about_page")
        .nth(1)
        .expect("about page")
        .split("pub(crate) fn privacy_page")
        .next()
        .unwrap();
    let privacy = page
        .split("pub(crate) fn privacy_page")
        .nth(1)
        .expect("privacy page");

    assert_eq!(about.matches("Route::Privacy {}").count(), 1);
    assert!(!about.contains(".privacy_summary"));
    assert!(!about.contains(".exit_ip_services"));
    assert!(route.contains("/settings/about/privacy"));
    assert!(route.contains("Self::Privacy {} => Some(Self::About {})"));
    assert!(privacy.contains(".privacy_summary"));
    assert!(privacy.contains(".exit_ip_services"));
    assert!(privacy.contains("documentation_url"));
    assert!(privacy.contains("translate_ui(current.locale, tr::"));
    assert!(
        !privacy.contains("max_lines"),
        "privacy disclosures must never be truncated"
    );
}

#[test]
fn privacy_policy_states_collection_sharing_retention_and_lan_risks() {
    let policy = fs::read_to_string("../paws_core/src/runtime_snapshot.rs").unwrap();

    for disclosure in [
        "不接入广告、行为分析或远程遥测服务",
        "订阅与规则提供方",
        "运行诊断",
        "出口 IP 查询",
        "请求不包含订阅、节点、规则、DNS 记录或连接记录",
        "日志与导出",
        "局域网控制器",
        "删除与保留",
        "外部链接",
    ] {
        assert!(
            policy.contains(disclosure),
            "missing privacy disclosure: {disclosure}"
        );
    }
}

#[test]
fn appearance_settings_use_arkit_shadcn_choices_and_persist_actions() {
    let page = fs::read_to_string("src/view/pages/appearance.rs").unwrap();
    let view = fs::read_to_string("src/view.rs").unwrap();
    let platform = fs::read_to_string("src/bridge/mod.rs").unwrap();
    let color_mode =
        fs::read_to_string("../../entry/src/main/ets/plugins/ColorModePlugin.ets").unwrap();

    assert!(page.contains("RadioGroup"));
    assert!(page.contains("Action::SetLanguagePreference"));
    assert!(page.contains("Action::SetThemePreference"));
    assert!(!page.contains("button {"));
    assert!(view.contains("ThemeProvider"));
    assert!(view.contains("use_theme().colors"));
    assert!(platform.contains("set_color_mode"));
    assert!(color_mode.contains("context.abilityContext.setColorMode"));
}

#[test]
fn network_stack_is_a_bounded_selector_with_two_real_backends() {
    let page = fs::read_to_string("src/view/pages/settings.rs").unwrap();
    let view = fs::read_to_string("src/view.rs").unwrap();
    let stack_field = page
        .split("tr::page_tr_231()")
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
fn network_ports_are_editable_and_lan_access_requires_a_secret() {
    let page = fs::read_to_string("src/view/pages/settings.rs").unwrap();
    let ui = fs::read_to_string("src/ui.rs").unwrap();
    let model = fs::read_to_string("../paws_model/src/lib.rs").unwrap();
    let core = fs::read_to_string("../paws_core/src/lib.rs").unwrap();
    let callbacks = fs::read_to_string("src/bridge/mod.rs").unwrap();

    assert!(page.contains("translate_ui(current.locale, tr::page_tr_"));
    assert!(page.contains("translate_ui(current.locale, tr::page_tr_"));
    assert!(page.matches("Input {").count() >= 2);
    assert!(page.contains("placeholder: Some(\"7890\""));
    assert!(page.contains("placeholder: Some(\"9090\""));
    assert!(page.contains("translate_ui(current.locale, tr::"));
    assert!(page.contains("0.0.0.0:{controller_port_value}"));
    assert!(page.contains("Authorization: Bearer <secret>"));
    assert!(page.contains("copy_controller_secret"));
    assert!(page.contains("crate::bridge::copy_text(secret)"));
    assert!(ui.contains("Action::SaveNetworkSettings"));
    assert!(ui.contains("set_profile_network_config"));
    assert!(ui.contains("mixed_port != state.snapshot.network_ports.mixed_port"));
    assert!(model.contains("pub const DEFAULT_MIXED_PORT: u16 = 7890"));
    assert!(model.contains("pub const DEFAULT_CONTROLLER_PORT: u16 = 9090"));
    assert!(core.contains("restart_mixed_listener(tunnel, mixed_port)"));
    assert!(core.contains("probe_exit_location(mixed_port)"));
    assert!(callbacks.contains("pub(crate) async fn copy_text"));
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
