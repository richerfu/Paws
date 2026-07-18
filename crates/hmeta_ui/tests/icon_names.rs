const UI_ICON_NAMES: &[&str] = &[
    "activity",
    "archive",
    "arrow-left",
    "arrow-down",
    "arrow-up",
    "badge-info",
    "check",
    "chevron-down",
    "chevron-right",
    "chevron-up",
    "circle",
    "circle-check",
    "clock",
    "compass",
    "download",
    "ellipsis-vertical",
    "external-link",
    "file-pen-line",
    "file-text",
    "file-up",
    "gauge",
    "git-branch",
    "globe",
    "history",
    "layout-grid",
    "list",
    "network",
    "palette",
    "play",
    "plus",
    "radar",
    "refresh-cw",
    "route",
    "rotate-ccw",
    "rss",
    "save",
    "search",
    "settings",
    "shield-check",
    "square",
    "toggle-left",
    "toggle-right",
    "trash-2",
    "unplug",
    "x",
];

#[test]
fn ui_icon_names_exist_in_embedded_lucide_assets() {
    let asset_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../ohos-rs/arkit/crates/arkit_icon/assets/lucide");
    let missing = UI_ICON_NAMES
        .iter()
        .copied()
        .filter(|name| !asset_dir.join(format!("{name}.svg")).is_file())
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing lucide icons: {missing:?}");
}
