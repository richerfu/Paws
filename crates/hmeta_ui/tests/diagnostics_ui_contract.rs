const VIEW_SOURCE: &str = include_str!("../src/view.rs");
const ACTIVITY_SOURCE: &str = include_str!("../src/view/pages/activity.rs");

#[test]
fn logs_use_arkit_native_virtual_rows_and_expose_full_details() {
    assert!(VIEW_SOURCE.contains("fn VirtualLogList("));
    assert!(
        VIEW_SOURCE.contains("use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys")
    );
    assert!(VIEW_SOURCE.contains("use_layout_frame_node(move |host_node, _frame|"));
    assert!(VIEW_SOURCE.contains(".on_click(move || on_open.call(item.clone()))"));
    assert!(!VIEW_SOURCE.contains("NodeEventType::OnClick"));
    assert!(VIEW_SOURCE.contains("list_cached_count: 18_i32"));
    assert!(VIEW_SOURCE.contains("fn log_detail_dialog("));
    assert!(VIEW_SOURCE.contains("matches_log_filter_normalized"));
}

#[test]
fn geodata_rows_open_file_metadata_and_paths() {
    assert!(VIEW_SOURCE.contains("fn geodata_detail_dialog("));
    assert!(VIEW_SOURCE.contains("文件位置"));
    assert!(VIEW_SOURCE.contains("time_format::format_unix_seconds"));
}

#[test]
fn connection_cards_keep_the_close_action_compact() {
    assert!(ACTIVITY_SOURCE.contains("fn compact_connection_card("));
    assert!(ACTIVITY_SOURCE.contains("size: ButtonSize::Icon"));
    assert!(!ACTIVITY_SOURCE.contains("variant: FlatButtonVariant::Destructive,\n                                percent_width: Some(1.0)"));
}
