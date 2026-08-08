const VIEW_SOURCE: &str = concat!(
    include_str!("../src/view/pages/logs.rs"),
    include_str!("../src/view/pages/resources.rs"),
    include_str!("../src/view.rs"),
);
const ACTIVITY_SOURCE: &str = include_str!("../src/view/pages/activity.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn logs_use_arkit_rsx_virtual_rows_and_expose_full_details() {
    assert!(VIEW_SOURCE.contains("fn VirtualLogList("));
    assert!(VIEW_SOURCE.contains("use_virtual_source_items_keyed(VirtualKind::List, item_keys"));
    assert!(!VIEW_SOURCE.contains("use_virtual_node_adapter_items_keyed"));
    assert_eq!(VIEW_SOURCE.matches("virtual_source: source").count(), 2);
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
fn activity_lists_use_compact_arkit_rsx_virtual_rows() {
    assert!(ACTIVITY_SOURCE.contains("fn VirtualRequestList("));
    assert!(ACTIVITY_SOURCE.contains("fn VirtualConnectionList("));
    assert_eq!(
        ACTIVITY_SOURCE
            .matches("use_virtual_source_items_keyed(VirtualKind::List, item_keys")
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
    assert!(ACTIVITY_SOURCE.contains("Action::CloseConnection(id)"));
    assert!(ACTIVITY_SOURCE.contains("navigator.push(Route::Connections { query })"));
}

#[test]
fn activity_rows_create_structured_hot_rules_without_leaving_virtual_lists() {
    assert_eq!(
        ACTIVITY_SOURCE
            .matches("Action::OpenManualRuleEditor")
            .count(),
        2,
    );
    assert!(ACTIVITY_SOURCE.contains("fn manual_rule_dialog("));
    assert!(ACTIVITY_SOURCE.contains("fn ManualRuleDialogContent("));
    assert!(ACTIVITY_SOURCE.contains("let current = state.read().clone();"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::Domain"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::DomainSuffix"));
    assert!(ACTIVITY_SOURCE.contains("ManualRuleMatchKind::IpCidr"));
    assert!(ACTIVITY_SOURCE.contains("Select {"));
    assert!(!ACTIVITY_SOURCE.contains("ManualRuleTargetSelect"));
    assert!(!ACTIVITY_SOURCE.contains("\"Fruits\""));
    assert!(ACTIVITY_SOURCE.contains("Action::SetManualRuleDisconnect(value)"));
    assert!(ACTIVITY_SOURCE.contains("manual_rule_preview("));
    assert!(ACTIVITY_SOURCE.contains("find_manual_rule_conflict("));
    assert_eq!(ACTIVITY_SOURCE.matches("on_add_rule.call(").count(), 2);
}

#[test]
fn resource_rules_are_compact_and_section_titles_have_no_counts() {
    let page = section(VIEW_SOURCE, "fn resources_page", "fn geodata_detail_dialog");
    let rule = section(VIEW_SOURCE, "fn rule_view", "fn reordered_rule_ids");
    let label = section(VIEW_SOURCE, "fn section_label", "fn empty_state");

    assert!(page.contains("translate_ui(current.locale, tr::"));
    assert!(page.contains("tr::resources_rules_title()"));
    assert!(page.contains("tr::resources_import_rules()"));
    assert!(page.contains("Action::ImportRules"));
    assert!(page.contains("current.rule_import_loading"));
    assert!(page.contains("translate_ui(current.locale, tr::page_tr_"));
    assert!(page.contains("Action::OpenManualRuleEditor"));
    assert!(page.contains("manual_rule_dialog(state, &current)"));
    assert!(!page.contains("section_label(tr(current.locale, \"Provider\", \"Providers\"),"));
    assert!(!page.contains("section_label(strings(current.locale).resources_rules_title,"));
    assert!(page.contains("compact_rule_list(rules)"));

    assert!(rule.contains("height: 88.0"));
    assert!(rule.contains("max_lines: 2"));
    assert!(rule.contains("fn compact_rule_action("));
    assert!(rule.contains("width: 32.0"));
    assert!(!rule.contains("{card("));
    assert!(!label.contains("count.to_string()"));
}

#[test]
fn segmented_filter_buttons_preserve_the_full_label_width() {
    let segmented = section(VIEW_SOURCE, "fn FlatSegmented(", "struct FlatDialogProps");

    // The default ArkUI button padding squeezes the label (e.g. "Debug" in
    // the log level filter); zero it out so every option stays readable.
    assert!(segmented.contains("padding: 0.0"));
}
