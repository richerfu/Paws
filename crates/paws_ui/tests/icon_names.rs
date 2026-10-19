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
    "house",
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
    "paw-print",
    "play",
    "power",
    "plus",
    "radar",
    "refresh-cw",
    "route",
    "rotate-ccw",
    "rss",
    "save",
    "scan-qr-code",
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
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cargo")
        });
    let checkouts = cargo_home.join("git/checkouts");
    let workspace = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
    )
    .unwrap();
    let arkit_line = workspace
        .lines()
        .find(|line| line.starts_with("arkit ="))
        .unwrap();
    let revision = arkit_line
        .split("rev = \"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let asset_dir = std::fs::read_dir(checkouts)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("arkit-"))
        .flat_map(|entry| {
            std::fs::read_dir(entry.path())
                .unwrap()
                .filter_map(Result::ok)
        })
        .filter(|entry| revision.starts_with(entry.file_name().to_string_lossy().as_ref()))
        .map(|entry| entry.path().join("crates/arkit_icon/assets/lucide"))
        .find(|path| path.is_dir())
        .expect("Pinned arkit icon assets must exist in the Cargo checkout");
    let missing = UI_ICON_NAMES
        .iter()
        .copied()
        .filter(|name| !asset_dir.join(format!("{name}.svg")).is_file())
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing lucide icons: {missing:?}");
}
