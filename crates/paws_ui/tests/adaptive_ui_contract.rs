use std::fs;

const VIEW: &str = include_str!("../src/view.rs");
const ROUTE: &str = include_str!("../src/view/route.rs");

#[test]
fn package_declares_every_supported_form_factor() {
    let module = fs::read_to_string("../../entry/src/main/module.json5").unwrap();
    for device in ["\"phone\"", "\"tablet\"", "\"2in1\""] {
        assert!(
            module.contains(device),
            "missing supported device: {device}"
        );
    }
}

#[test]
fn shell_switches_structure_from_live_window_metrics() {
    assert!(VIEW.contains("AdaptiveProvider {"));
    assert!(VIEW.contains("AdaptiveConfig::new(AdaptiveMode::Auto)"));
    assert!(VIEW.contains("let adaptive = use_adaptive_layout();"));
    assert!(VIEW.contains("if adaptive.is_pc()"));
    assert!(VIEW.contains("DesktopSidebar {}"));
    assert!(VIEW.contains("BottomBar {}"));

    // Compatibility windows must remain free to select the compact branch.
    // A physical-device check or forced desktop minimum would incorrectly
    // turn a phone compatibility window on 2in1 into the desktop shell.
    let ability =
        fs::read_to_string("../../entry/src/main/ets/entryability/EntryAbility.ets").unwrap();
    assert!(!ability.contains("setWindowLimits"));
    assert!(!VIEW.contains("deviceInfo.deviceType"));
}

#[test]
fn compact_and_desktop_content_have_bounded_shadcn_measures() {
    assert!(VIEW.contains("COMPACT_CONTENT_MAX_WIDTH: f32 = 640.0"));
    assert!(VIEW.contains("page.desktop_content_width()"));
    assert!(VIEW.contains("adaptive.select(spacing::LG, spacing::XXL)"));
    assert!(ROUTE.contains("Self::Dashboard {} => 960.0"));
    assert!(ROUTE.contains("Self::Profiles {} | Self::Proxies {} => 1120.0"));
    assert!(ROUTE.contains("Self::Tools {}"));
}

#[test]
fn desktop_navigation_and_primary_rows_expose_pointer_and_focus_states() {
    let dashboard = include_str!("../src/view/pages/dashboard.rs");
    let tools = include_str!("../src/view/pages/tools.rs");

    assert!(VIEW.contains("ButtonVariant::Secondary"));
    assert!(VIEW.contains("ButtonVariant::Ghost"));
    for source in [dashboard, tools] {
        assert!(source.contains("onhover:"));
        assert!(source.contains("onfocus:"));
        assert!(source.contains("onblur:"));
        assert!(source.contains("focusable: adaptive.is_pc()"));
    }
}
