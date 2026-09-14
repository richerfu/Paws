//! Check both halves of declarative bridge assembly, not only package presence.
const ENTRY: &str = include_str!("../src/lib.rs");
const ABILITY: &str = include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");

#[test]
fn url_and_files_capabilities_are_registered_on_both_sides() {
    let registrations = ENTRY
        .split("#[entry(plugins = [")
        .nth(1)
        .unwrap()
        .split("])]")
        .next()
        .unwrap();
    for (rust_plugin, arkts_plugin) in [
        (
            "openharmony_ability_plugin_url::UrlBridgePlugin",
            "UrlPlugin",
        ),
        (
            "openharmony_ability_plugin_files::FilesBridgePlugin",
            "FilesPlugin",
        ),
    ] {
        assert_eq!(
            registrations.matches(rust_plugin).count(),
            1,
            "missing or duplicate native capability {rust_plugin}"
        );
        assert!(ABILITY.contains(&format!("new LazyPlugin(() => new {arkts_plugin}())")));
    }
}

#[test]
fn about_links_keep_the_typed_url_plugin_and_surface_failures() {
    let bridge = include_str!("../src/bridge/mod.rs");
    let page = include_str!("../src/view/pages/tools.rs");
    assert!(bridge.contains("app.open_url(url).await.map_err(|err| err.to_string())"));
    assert!(page.contains("services.open_external_url("));
    assert!(
        !bridge.contains("@ohos.url"),
        "ohos.url is a bridge capability, not an importable JS module"
    );
}

#[test]
fn about_revision_comes_from_the_actual_dependency_pin() {
    let manifest = include_str!("../../../Cargo.toml");
    let core = include_str!("../../paws_core/src/lib.rs");
    let build = include_str!("../../paws_core/build.rs");
    let arkit = manifest
        .lines()
        .find(|line| line.starts_with("arkit = "))
        .unwrap();
    assert!(arkit.contains("rev = \""));
    assert!(core.contains("env!(\"PAWS_ARKIT_REV\")"));
    assert!(build.contains("cargo:rerun-if-changed="));
    assert!(build.contains("cargo:rustc-env=PAWS_ARKIT_REV={revision}"));
}
