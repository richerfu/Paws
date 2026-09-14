const VIEW_SOURCE: &str = concat!(
    include_str!("../src/view/pages/logs.rs"),
    include_str!("../src/view/pages/resources.rs"),
    include_str!("../src/view.rs"),
);
const ACTIVITY_SOURCE: &str = include_str!("../src/view/pages/activity.rs");
const LOGS_SOURCE: &str = include_str!("../src/view/pages/logs.rs");
const PROXIES_SOURCE: &str = include_str!("../src/view/pages/proxies.rs");
const RESOURCES_SOURCE: &str = include_str!("../src/view/pages/resources.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn logs_use_arkit_rsx_virtual_rows_and_expose_full_details() {
    assert!(VIEW_SOURCE.contains("fn VirtualLogList("));
    assert!(VIEW_SOURCE.contains("use_virtual_items(VirtualKind::List, stamps"));
    assert!(!VIEW_SOURCE.contains("use_virtual_node_adapter_items_keyed"));
    assert_eq!(VIEW_SOURCE.matches("virtual_source: source").count(), 3);
    assert!(!VIEW_SOURCE.contains("use_layout_frame_node(move |host_node, _frame|"));
    assert!(VIEW_SOURCE.contains("onclick: move |_| on_open.call(open_item.clone())"));
    assert!(VIEW_SOURCE.contains("fn VirtualLogRowView("));
    assert!(!VIEW_SOURCE.contains("NodeBuilder::new"));
    assert!(!VIEW_SOURCE.contains("NodeEventType::OnClick"));
    assert!(VIEW_SOURCE.contains("list_cached_count: 18_i32"));
    assert!(VIEW_SOURCE.contains("fn log_detail_dialog("));
    assert!(VIEW_SOURCE.contains("matches_log_filter_normalized"));
}

#[test]
fn geodata_rows_open_file_metadata_and_paths() {
    assert!(VIEW_SOURCE.contains("fn geodata_detail_dialog("));
    assert!(VIEW_SOURCE.contains("translate_ui"));
    assert!(VIEW_SOURCE.contains("time_format::format_unix_seconds"));
}

#[test]
fn virtual_rows_receive_app_dependencies_explicitly() {
    let resource_rows = section(
        RESOURCES_SOURCE,
        "fn VirtualResourceList(",
        "fn RuleLookupDialog(",
    );
    let rule_row = section(RESOURCES_SOURCE, "fn rule_view(", "fn compact_rule_action");
    let archive_row = section(
        LOGS_SOURCE,
        "fn VirtualLogArchiveList(",
        "fn log_detail_dialog(",
    );
    let activity_rows = section(
        ACTIVITY_SOURCE,
        "fn VirtualRequestList(",
        "fn format_activity_timestamp(",
    );
    let proxy_rows = &PROXIES_SOURCE[PROXIES_SOURCE
        .find("fn VirtualProxyRow(")
        .expect("proxy virtual rows")..];

    for detached_subtree in [
        resource_rows,
        rule_row,
        archive_row,
        activity_rows,
        proxy_rows,
    ] {
        assert!(!detached_subtree.contains("use_context::<"));
        assert!(!detached_subtree.contains("Spinner {"));
    }
    assert!(VIEW_SOURCE.contains("resource_operations: Signal<ResourceOperationState>"));
    assert!(VIEW_SOURCE.contains("diagnostic_operations: Signal<DiagnosticOperationState>"));
    assert!(VIEW_SOURCE.contains("log_operations: Signal<LogOperationState>"));
    assert!(VIEW_SOURCE.contains("on_open_manual: EventHandler<()>"));
}

#[test]
fn activity_lists_use_compact_arkit_rsx_virtual_rows() {
    assert!(ACTIVITY_SOURCE.contains("fn VirtualRequestList("));
    assert!(ACTIVITY_SOURCE.contains("fn VirtualConnectionList("));
    assert_eq!(
        ACTIVITY_SOURCE
            .matches("use_virtual_items(VirtualKind::List, stamps")
            .count(),
        2,
    );
    assert!(!ACTIVITY_SOURCE.contains("use_virtual_node_adapter_items_keyed"));
    assert_eq!(ACTIVITY_SOURCE.matches("virtual_source: source").count(), 2,);
    assert_eq!(
        ACTIVITY_SOURCE.matches("list_cached_count: 18_i32").count(),
        2,
    );
    assert!(ACTIVITY_SOURCE.contains("const REQUEST_ROW_HEIGHT: f32 = 88.0;"));
    assert!(ACTIVITY_SOURCE.contains("const CONNECTION_ROW_HEIGHT: f32 = 88.0;"));
    assert!(ACTIVITY_SOURCE.contains("fn VirtualStatusBadge("));
    assert!(ACTIVITY_SOURCE.contains("fn VirtualRequestRowView("));
    assert!(ACTIVITY_SOURCE.contains("fn VirtualConnectionRowView("));
    assert!(!ACTIVITY_SOURCE.contains("NodeBuilder::new"));
    assert!(ACTIVITY_SOURCE.contains("format_activity_timestamp("));
    assert!(!ACTIVITY_SOURCE.contains("compact_connection_card"));
    assert!(!ACTIVITY_SOURCE.contains("{spaced(rows)}"));
}

#[test]
fn virtual_activity_rows_keep_their_previous_actions() {
    assert!(ACTIVITY_SOURCE.contains("on_open.call(connection_query.clone())"));
    assert!(ACTIVITY_SOURCE.contains("on_close.call(close_id.clone())"));
    assert!(ACTIVITY_SOURCE.contains("close_services.close_connection(id)"));
    assert!(ACTIVITY_SOURCE.contains("navigator.push(Route::Connections { query })"));
}

#[test]
fn activity_rows_create_structured_hot_rules_without_leaving_virtual_lists() {
    assert_eq!(
        ACTIVITY_SOURCE.matches("open_manual_rule_editor(").count(),
        2,
    );
    assert!(ACTIVITY_SOURCE.contains("fn ManualRuleDialog("));
    assert!(ACTIVITY_SOURCE.contains("fn ManualRuleDialogContent("));
    assert!(ACTIVITY_SOURCE.contains("let local_editors = use_local_rule_editors"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleDialog { local: local_editors }"));
    assert!(ACTIVITY_SOURCE.contains("let editors = local.signal.read().clone();"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::Domain"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::DomainSuffix"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::IpCidr"));
    assert!(ACTIVITY_SOURCE.contains("Select {"));
    assert!(!ACTIVITY_SOURCE.contains("ManualRuleTargetSelect"));
    assert!(!ACTIVITY_SOURCE.contains("\"Fruits\""));
    assert!(
        ACTIVITY_SOURCE.contains("set_manual_rule_disconnect(disconnect_editors.clone(), value)")
    );
    assert!(ACTIVITY_SOURCE.contains("manual_rule_preview("));
    assert!(ACTIVITY_SOURCE.contains("find_manual_rule_conflict("));
    assert_eq!(ACTIVITY_SOURCE.matches("on_add_rule.call(").count(), 2);
}

#[test]
fn resource_rules_are_compact_and_section_titles_have_no_counts() {
    let page = section(VIEW_SOURCE, "fn resources_page", "fn geodata_detail_dialog");
    let rule = section(VIEW_SOURCE, "fn rule_view", "fn reordered_rule_ids");
    let label = section(VIEW_SOURCE, "fn section_label", "fn empty_state");

    assert!(page.contains("translate_ui(locale, tr::"));
    assert!(page.contains("tr::resources_rules_title()"));
    assert!(page.contains("tr::resources_import_rules()"));
    assert!(VIEW_SOURCE.contains("import_services.import_rules(import_tasks.clone())"));
    assert!(page.contains("ResourceRulesHeader"));
    assert!(VIEW_SOURCE.contains("use_context::<UiOperationStores>()"));
    assert!(VIEW_SOURCE.contains("resource_operations.read().rule_import_loading"));
    assert!(page.contains("translate_ui(locale, tr::page_tr_"));
    assert!(VIEW_SOURCE.contains("services.open_manual_rule_editor"));
    assert!(page.contains("ManualRuleDialog { local: local_editors.clone() }"));
    assert!(!page.contains("section_label(tr(current.locale, \"Provider\", \"Providers\"),"));
    assert!(!page.contains("section_label(strings(current.locale).resources_rules_title,"));
    assert!(page.contains("VirtualResourceList"));

    assert!(rule.contains("height: 88.0"));
    assert!(rule.contains("max_lines: 2"));
    assert!(VIEW_SOURCE.contains("fn compact_rule_action<F>("));
    assert!(rule.contains("width: 32.0"));
    assert!(!rule.contains("{card("));
    assert!(!label.contains("count.to_string()"));
}

#[test]
fn segmented_filter_buttons_preserve_the_full_label_width() {
    let segmented = section(VIEW_SOURCE, "fn FlatSegmented(", "struct FlatDialogProps");

    // Upstream TabsTrigger owns equal-width layout and avoids native Button insets.
    assert!(segmented.contains("TabsList {"));
    assert!(segmented.contains("TabsTrigger {"));
    assert!(!segmented.contains("button {"));
}
