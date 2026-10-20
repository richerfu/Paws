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
    // Read the resolved package assets without linking the OHOS renderer into
    // this host test. Filtering metadata to the host also avoids downloading
    // packages for unrelated target platforms during an offline test run.
    let rustc =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("-vV")
            .output()
            .expect("query Rust host target");
    assert!(rustc.status.success());
    let version = String::from_utf8(rustc.stdout).unwrap();
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("Rust host target");
    let metadata = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version=1",
            "--filter-platform",
            host,
        ])
        .output()
        .expect("query resolved Cargo packages");
    assert!(
        metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "arkit_icon")
        .expect("resolved Arkit icon package");
    let asset_dir = std::path::Path::new(package["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("assets/lucide");
    let missing = UI_ICON_NAMES
        .iter()
        .copied()
        .filter(|name| !asset_dir.join(format!("{name}.svg")).is_file())
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing lucide icons: {missing:?}");
}
