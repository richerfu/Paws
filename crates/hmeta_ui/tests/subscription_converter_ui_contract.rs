use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn general_settings_opens_a_dedicated_converter_route() {
    let route = fs::read_to_string(root().join("crates/hmeta_ui/src/view/route.rs")).unwrap();
    let tools = fs::read_to_string(root().join("crates/hmeta_ui/src/view/pages/tools.rs")).unwrap();

    assert!(route.contains("#[route(\"/settings/subscription-converter\")]"));
    assert!(route.contains("SubscriptionConverter {}"));
    assert!(route.contains("\"订阅转化规则\""));
    assert!(tools.contains("Route::SubscriptionConverter {}"));
}

#[test]
fn converter_page_exposes_sub_web_actions_and_privacy_context() {
    let page =
        fs::read_to_string(root().join("crates/hmeta_ui/src/view/pages/subscription_converter.rs"))
            .unwrap();

    for marker in [
        "生成订阅链接",
        "生成短链",
        "一键导入 Clash",
        "从长链或短链解析",
        "上传并使用配置",
        "第三方服务",
    ] {
        assert!(
            page.contains(marker),
            "missing converter UI marker: {marker}"
        );
    }
}

#[test]
fn system_clipboard_and_clash_scheme_are_wired_through_entry_ability() {
    let callbacks = fs::read_to_string(root().join("crates/hmeta_ui/src/bridge/mod.rs")).unwrap();
    let entry = fs::read_to_string(root().join("entry/src/main/ets/entryability/EntryAbility.ets"))
        .unwrap();
    let clipboard =
        fs::read_to_string(root().join("entry/src/main/ets/plugins/ClipboardPlugin.ets")).unwrap();

    assert!(callbacks.contains("set-text"));
    assert!(callbacks.contains("pub(crate) async fn copy_text"));
    assert!(clipboard.contains("pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN"));
    assert!(entry.contains("new LazyPlugin(() => new ClipboardPlugin())"));
    assert!(callbacks.contains("clash://install-config?url="));
}
