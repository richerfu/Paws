use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn general_settings_opens_a_dedicated_converter_route() {
    let route = fs::read_to_string(root().join("crates/paws_ui/src/view/route.rs")).unwrap();
    let tools = fs::read_to_string(root().join("crates/paws_ui/src/view/pages/tools.rs")).unwrap();

    assert!(route.contains("#[route(\"/settings/subscription-converter\")]"));
    assert!(route.contains("SubscriptionConverter {}"));
    assert!(route.contains("tr::page_tr_002()"));
    assert!(tools.contains("Route::SubscriptionConverter {}"));
}

#[test]
fn converter_page_exposes_sub_web_actions_and_privacy_context() {
    let page =
        fs::read_to_string(root().join("crates/paws_ui/src/view/pages/subscription_converter.rs"))
            .unwrap();

    for message in [
        "tr::page_tr_072()",
        "tr::page_tr_079()",
        "tr::page_tr_083()",
        "tr::page_tr_084()",
        "tr::page_tr_094()",
        "tr::hard_zh_046()",
    ] {
        assert!(
            page.contains(message),
            "missing converter UI message: {message}"
        );
    }
    assert!(page.contains("version_tasks.query("));
    assert!(page.contains("short_tasks.query("));
    assert!(page.contains("parse_tasks.query("));
    assert!(page.contains("upload_tasks.mutate("));
}

#[test]
fn system_clipboard_and_clash_scheme_are_wired_through_entry_ability() {
    let callbacks = fs::read_to_string(root().join("crates/paws_ui/src/bridge/mod.rs")).unwrap();
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

#[test]
fn converter_draft_failures_are_visible_and_never_silently_replace_the_file() {
    let logic =
        fs::read_to_string(root().join("crates/paws_ui/src/subscription_converter.rs")).unwrap();
    let page =
        fs::read_to_string(root().join("crates/paws_ui/src/view/pages/subscription_converter.rs"))
            .unwrap();

    let load = logic
        .split("pub(crate) fn load_draft")
        .nth(1)
        .and_then(|tail| tail.split("pub(crate) fn save_draft").next())
        .expect("load_draft section");
    assert!(load.contains("Result<SubscriptionConverterDraft, String>"));
    assert!(load.contains("ErrorKind::NotFound"));
    assert!(load.contains("serde_json::from_str(&text).map_err"));
    assert!(!load.contains(".ok()"));
    assert!(page.contains("persistence_error"));
    assert!(page.contains("persist_converter_draft"));
    assert!(page.contains("if !unchanged"));
    assert!(!page.contains("let _ = save_draft"));
}
