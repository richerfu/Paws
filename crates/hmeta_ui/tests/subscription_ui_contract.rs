const VIEW: &str = include_str!("../src/view.rs");
const UI: &str = include_str!("../src/ui.rs");
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
fn proxy_nodes_use_arkit_native_virtual_list_and_grid() {
    let page = section(VIEW, "fn proxies_page", "fn profiles_page");

    assert!(page.contains("flatten_proxy_groups"));
    assert!(page.contains("use_virtual_node_adapter_items_keyed"));
    assert!(!page.contains("use_virtual_node_adapter_keyed("));
    assert!(page.contains("VirtualKind::Grid"));
    assert!(page.contains("VirtualKind::List"));
    assert!(page.contains("NodeBuilder::new"));
    assert!(page.contains("grid_column_template: \"1fr 1fr\""));
    assert!(page.contains("grid_cached_count"));
    assert!(page.contains("list_cached_count"));
    assert!(page.contains("ProxyLayoutMode"));
    assert!(page.contains("toggle_icon"));
    assert!(page.contains("\"list\""));
    assert!(page.contains("\"layout-grid\""));
    assert!(page.contains("fixed_scaffold"));
    assert!(page.contains("EventHandler<(String, String)>"));
    assert!(page.contains("on_select.call"));
    assert!(page.contains(".on_click(move ||"));
    assert!(!page.contains("NodeEventType::OnClick"));
    assert!(page.contains("VirtualProxyGridRenderState"));
    assert!(page.contains("selection_pending && item.selected"));
    assert!(page.contains("if item.selected {"));
    assert!(!page.contains("if !item.selected && !selection_pending"));
    assert!(!page.contains("mounted.adapter.detach()"));
    assert!(!page.contains("arkit::queue_ui_loop"));
    assert!(page.contains("item.group"));
    assert!(page.contains("item.proxy_type"));
    assert!(page.contains("proxies_untested"));

    // The grid owns scrolling and creates visible native nodes on demand;
    // nested expandable groups would eagerly instantiate every proxy again.
    assert!(!page.contains("expanded_group"));
    assert!(!page.contains("Action::TestProxyGroupDelays"));
    assert!(!page.contains("Action::TestProxyDelay"));
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
    assert!(dialog.contains("disabled: Some(import_loading)"));
    assert!(dialog.contains("Spinner { size: 16.0"));
    assert!(UI.contains("profile_import_succeeded = true"));
}

#[test]
fn proxy_node_names_use_native_width_based_ellipsis() {
    let page = section(VIEW, "fn proxies_page", "fn profiles_page");

    assert!(page.contains("item.name.clone()"));
    assert!(page.contains("ArkUINodeAttributeType::TextOverflow, 2_i32"));
    assert!(!page.contains("selected_title_limit"));
    assert!(!page.contains("title_limit"));
}
