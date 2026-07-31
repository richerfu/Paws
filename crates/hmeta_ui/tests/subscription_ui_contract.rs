const VIEW: &str = concat!(
    include_str!("../src/view/pages/proxies.rs"),
    include_str!("../src/view/pages/profiles.rs"),
    include_str!("../src/view/pages/traffic.rs"),
    include_str!("../src/view/pages/logs.rs"),
    include_str!("../src/view/pages/yaml_editor.rs"),
    include_str!("../src/view.rs"),
);
const UI: &str = concat!(
    include_str!("../src/ui.rs"),
    include_str!("../src/ui/tasks.rs")
);
const PLATFORM_CALLBACKS: &str = include_str!("../src/platform_callbacks.rs");
const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn imported_subscription_cards_follow_the_reference_mobile_interaction() {
    let page = section(VIEW, "fn profiles_page", "fn profile_action_dialog");

    assert!(page.contains("subscription_url"));
    assert!(page.contains("last_refresh_at"));
    assert!(page.contains("subscription_user_info"));
    assert!(page.contains("circle-check"));
    assert!(page.contains("ellipsis-vertical"));
    assert!(page.contains("Action::ActivateProfile"));
    assert!(page.contains("profiles_no_match_title"));

    // Destructive and maintenance actions belong in the mobile overflow dialog,
    // not in a dense row of card buttons.
    assert!(!page.contains("FlatButtonVariant::Destructive"));
    assert!(!page.contains("Action::DeleteProfile"));
    assert!(!page.contains("Action::OpenYamlEditor"));
}

#[test]
fn proxy_groups_use_an_arkit_rsx_heterogeneous_virtual_list() {
    let page = section(VIEW, "fn proxies_page", "fn profiles_page");

    assert!(page.contains("grouped_proxy_rows"));
    assert!(page.contains("use_virtual_node_adapter_items_keyed"));
    assert!(!page.contains("use_virtual_node_adapter_rsx_items_keyed"));
    assert!(page.contains("VirtualKind::List"));
    assert!(!page.contains("VirtualKind::Grid"));
    assert!(!page.contains("NodeBuilder::new"));
    assert!(page.contains("list_cached_count"));
    assert!(page.contains("fixed_scaffold"));
    assert!(page.contains("EventHandler<(String, String)>"));
    assert!(page.contains("on_select.call"));
    assert!(page.contains("EventHandler<String>"));
    assert!(page.contains("on_toggle.call"));
    assert!(page.contains("onclick: move |_|"));
    assert!(!page.contains("NodeEventType::OnClick"));
    assert!(page.contains("fn VirtualProxyGroupList("));
    assert!(page.contains("fn VirtualProxySectionRow("));
    assert!(page.contains("fn VirtualProxyGroupRow("));
    assert!(page.contains("fn VirtualProxyMemberRow("));
    assert!(page.contains("ProxyGroupRow::Section"));
    assert!(page.contains("ProxyGroupRow::Group"));
    assert!(page.contains("ProxyGroupRow::Member"));
    assert!(!page.contains("mounted.adapter.detach()"));
    assert!(!page.contains("arkit::queue_ui_loop"));
    assert!(page.contains("member.group"));
    assert!(page.contains("member.proxy_type"));
    assert!(page.contains("member.subgroup"));
    assert!(page.contains("proxies_untested"));

    // Expansion changes only the heterogeneous adapter's row model; ArkUI
    // continues to instantiate visible group/member rows on demand.
    assert!(page.contains("expanded_group"));
    assert!(!page.contains("ProxyGroupScope"));
    assert!(!page.contains("GLOBAL ·"));
    assert!(!page.contains("ProxyLayoutMode"));
    assert!(!page.contains("Action::TestProxyGroupDelays"));
    assert!(!page.contains("Action::TestProxyDelay"));
}

#[test]
fn proxy_selection_updates_only_the_exact_rule_group() {
    assert!(!UI.contains("proxy_selection_chain"));
    assert!(UI.contains("select_proxy_and_snapshot(group, proxy)"));
    assert!(UI.contains("select_proxy_via_controller(&group, &proxy)"));
}

#[test]
fn proxy_delay_test_has_visible_pending_feedback() {
    let page = section(VIEW, "fn proxies_page", "fn profiles_page");

    assert!(page.contains("proxy_delay_loading"));
    assert!(page.contains("Spinner"));
    assert!(page.contains("disabled: Some(proxy_delay_loading)"));
    assert!(page.contains("size: ButtonSize::Icon"));
    assert!(!page.contains("content: if proxy_delay_loading"));
}

#[test]
fn subscription_overflow_preserves_the_meow_action_set() {
    let menu = section(VIEW, "fn profile_action_dialog", "fn profile_edit_dialog");

    for action in [
        "Action::ActivateProfile",
        "Action::UpdateProfileSubscription",
        "Action::OpenYamlEditor",
        "Action::ExportProfile",
        "Action::RefreshProfile",
        "Action::RestoreProfileBackup",
    ] {
        // Update is dispatched by the edit dialog immediately following the
        // action dialog; the menu itself opens that edit dialog.
        if action != "Action::UpdateProfileSubscription" {
            assert!(menu.contains(action), "missing {action}");
        }
    }
    assert!(menu.contains("edit_profile_id.set"));
    assert!(menu.contains("delete_profile_id.set"));

    let edit = section(VIEW, "fn profile_edit_dialog", "fn profile_delete_dialog");
    assert!(edit.contains("Action::UpdateProfileSubscription"));
    assert!(edit.contains("Subscription URL"));

    let delete = section(VIEW, "fn profile_delete_dialog", "fn traffic_page");
    assert!(delete.contains("Action::DeleteProfile"));
}

#[test]
fn profile_export_reaches_the_harmony_document_picker() {
    assert!(UI.contains("platform_callbacks::export_profile"));
    assert!(PLATFORM_CALLBACKS.contains("exportProfile"));
    assert!(ENTRY_ABILITY.contains("DocumentSaveOptions"));
    assert!(ENTRY_ABILITY.contains("fileIo.writeSync"));
    assert!(ENTRY_ABILITY.contains("exportProfile: async"));
}

#[test]
fn log_recording_is_opt_in_with_daily_history_and_export() {
    let page = section(VIEW, "fn logs_page", "struct VirtualLogRow");

    assert!(page.contains("Action::ToggleLogRecording"));
    assert!(page.contains("\"play\""));
    assert!(page.contains("\"square\""));
    assert!(page.contains("history_open"));
    assert!(page.contains("log_recording.archives"));
    assert!(page.contains("VirtualLogArchiveList"));
    assert!(page.contains("Action::ExportLogArchive"));
    assert!(page.contains("Action::DeleteLogArchive"));
    assert!(page.contains("log_archive_delete_dialog"));
    assert!(page.contains("此操作无法撤销"));
    assert!(!page.contains("archive_rows"));
    let archive_list = section(VIEW, "fn VirtualLogArchiveList(", "fn VirtualLogList(");
    assert!(archive_list.contains("VirtualKind::List"));
    assert!(archive_list.contains("use_virtual_node_adapter_items_keyed"));
    assert!(!archive_list.contains("use_virtual_node_adapter_rsx_items_keyed"));
    assert!(!archive_list.contains("NodeBuilder::new"));
    assert!(archive_list.contains("list_cached_count: 12_i32"));
    assert!(archive_list.contains("on_delete"));
    assert!(UI.contains("set_log_recording_enabled"));
    assert!(UI.contains("read_log_archive"));
    assert!(UI.contains("delete_log_archive"));
    assert!(UI.contains("platform_callbacks::export_log"));
    assert!(PLATFORM_CALLBACKS.contains("exportLog"));
    assert!(ENTRY_ABILITY.contains("exportLog: async"));
    assert!(ENTRY_ABILITY.contains("DocumentSaveOptions"));
    assert!(ENTRY_ABILITY.contains("'.log'"));
}

#[test]
fn subscription_scan_reaches_scankit_and_the_import_pipeline() {
    let dialog = section(VIEW, "fn profile_import_dialog", "fn yaml_editor_dialog");

    assert!(dialog.contains("Action::ScanProfileSubscription"));
    assert!(dialog.contains("\"scan-qr-code\""));
    assert!(dialog.contains("profiles_scan_loading"));
    assert!(UI.contains("scan_profile_subscription_and_snapshot"));
    assert!(UI.contains("parse_scanned_subscription"));
    assert!(UI.contains("profile.subscription_url.as_deref()"));
    assert!(PLATFORM_CALLBACKS.contains("scanSubscription"));
    assert!(PLATFORM_CALLBACKS.contains("PromiseRaw<'static, String>"));
    assert!(ENTRY_ABILITY.contains("scanBarcode.startScanForResult"));
    assert!(ENTRY_ABILITY.contains("scanCore.ScanType.QR_CODE"));
    assert!(ENTRY_ABILITY.contains("enableAlbum: true"));
    assert!(ENTRY_ABILITY.contains("SCAN_CANCELLED_CODE"));
}

#[test]
fn profile_surfaces_do_not_reintroduce_shadow_attributes() {
    assert!(!VIEW.contains("shadow_radius:"));
    assert!(!VIEW.contains("shadow_color:"));
    assert!(!VIEW.contains("shadow_offset:"));
}

#[test]
fn network_import_has_a_real_pending_and_success_lifecycle() {
    let page = section(VIEW, "fn profiles_page", "fn profile_action_dialog");
    let dialog = section(VIEW, "fn profile_import_dialog", "fn yaml_editor_dialog");

    assert!(page.contains("profile_import_succeeded"));
    assert!(page.contains("import_open.set(false)"));
    assert!(dialog.contains("content_key"));
    assert!(dialog.contains("ProfileImportDialogBody"));
    assert!(dialog.contains("disabled: Some(import_loading)"));
    assert!(dialog.contains("Spinner { size: 16.0"));
    assert!(dialog.contains("profiles_import_loading"));
    assert!(UI.contains("profile_import_loading = true"));
    assert!(UI.contains("profile_import_succeeded = true"));
    assert!(UI.contains("Action::ImportLocalProfile"));
    assert!(UI.contains("profile_import_loading = false"));
}

#[test]
fn proxy_node_names_use_native_width_based_ellipsis() {
    let page = section(VIEW, "fn proxies_page", "fn profiles_page");

    assert!(page.contains("content: member.name"));
    assert!(page.contains("text_overflow: \"ellipsis\""));
    assert!(!page.contains("selected_title_limit"));
    assert!(!page.contains("title_limit"));
}
