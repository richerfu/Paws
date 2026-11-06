use std::fs;

#[test]
fn common_controls_delegate_to_upstream_shadcn() {
    let view = include_str!("../src/view.rs");
    for component in [
        "Button {",
        "TabsList {",
        "TabsTrigger {",
        "Dialog {",
        "CardTitle {",
        "CardDescription {",
    ] {
        assert!(
            view.contains(component),
            "missing upstream primitive {component}"
        );
    }
    assert!(
        !view.contains("ModalPortal {"),
        "dialog styling/lifecycle must not fork upstream"
    );
    assert!(!view.contains("FormItem"));
    let activity = include_str!("../src/view/pages/activity.rs");
    assert!(activity.contains("ACTIVITY_ACTION_SIZE: f32 = control::ICON_SM"));
}

#[test]
fn new_features_use_settings_routes_and_upstream_controls() {
    let tools = include_str!("../src/view/pages/tools.rs");
    let routes = include_str!("../src/view/route.rs");
    let view = include_str!("../src/view.rs");
    // Keep the 1.2.0 section order and direct access to existing destinations.
    let positions = [
        "tr::page_tr_022()",
        "tr::page_tr_024()",
        "tr::page_tr_272()",
        "tr::page_tr_026()",
        "tr::page_tr_031()",
    ]
    .map(|title| tools.find(title).expect("1.2.0 settings section"));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!tools.contains("if advanced"));
    for destination in [
        "Diagnostics",
        "Backups",
        "AdvancedDiagnostics",
        "Requests",
        "Connections",
        "Resources",
        "Logs",
    ] {
        assert!(tools.contains(&format!("Route::{destination}")));
        assert!(routes.contains(&format!("Self::{destination}")));
    }
    for (page, route) in [
        ("diagnostics", "Diagnostics"),
        ("backups", "Backups"),
        ("advanced_diagnostics", "AdvancedDiagnostics"),
    ] {
        let source = fs::read_to_string(format!("src/view/pages/{page}.rs")).unwrap();
        assert!(source.contains(&format!("scaffold(Route::{route}")));
        assert!(source.contains("FieldGroup {"));
        assert!(source.contains("OperationButton {"));
        assert!(!source.contains("FlatSegmented {"));
        assert!(!source.contains("button {"));
        assert!(!source.contains("background_color:"));
        assert!(!source.contains("border_radius:"));
    }
    for primitive in [
        "Badge {",
        "Alert {",
        "AlertTitle {",
        "AlertDescription {",
        "Spinner {",
    ] {
        assert!(view.contains(primitive));
    }
}

#[test]
fn pages_use_the_shared_type_scale_and_radius_tokens() {
    let mut paths = fs::read_dir("src/view/pages")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect::<Vec<_>>();
    paths.push("src/view.rs".into());
    paths.sort();
    for path in paths {
        let source = fs::read_to_string(&path).unwrap();
        for line in source.lines() {
            if let Some((_, value)) = line.split_once("font_size:") {
                assert!(
                    !value
                        .trim_start()
                        .starts_with(|ch: char| ch.is_ascii_digit()),
                    "numeric font size in {}: {line}",
                    path.display()
                );
            }
            for legacy_radius in [
                "border_radius: 6.0",
                "border_radius: 8.0",
                "border_radius: 16.0",
                "border_radius: 999.0",
            ] {
                assert!(
                    !line.contains(legacy_radius),
                    "hardcoded radius in {}: {line}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn virtual_lists_use_identity_revision_api_without_copying_the_removed_adapter() {
    for page in ["activity", "logs", "proxies", "resources"] {
        let source = fs::read_to_string(format!("src/view/pages/{page}.rs")).unwrap();
        assert!(source.contains("use_virtual_items("));
        assert!(source.contains("VirtualItemStamp::new("));
        assert!(!source.contains("use_virtual_source_items_keyed"));
    }
}
