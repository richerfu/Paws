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
fn pages_use_the_shared_type_scale_and_radius_tokens() {
    for file in fs::read_dir("src/view/pages").unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
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
