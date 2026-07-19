const VIEW: &str = include_str!("../src/view.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn dashboard_stays_flat_and_decision_focused() {
    let page = section(VIEW, "fn dashboard_page", "fn proxies_page");

    assert!(page.contains("ToggleGroup"));
    assert!(page.contains("flatten_proxy_groups"));
    assert!(page.contains("VirtualQuickProxyList"));
    assert!(page.contains("fixed_scaffold_flush_bottom"));
    assert!(page.contains("stabilize_proxy_items"));
    assert!(page.contains("key: \"dashboard-quick-proxy-list\""));
    assert!(!page.contains("preview_proxies.truncate(3)"));
    assert!(!page.contains("row { height: 72.0 }"));
    assert!(page.contains("当前节点"));
    assert!(page.contains("VPN IP"));
    assert!(!page.contains("format_speed"));

    // The home page uses one flat hierarchy. Cards belong on detail pages,
    // while immediate choices remain visible as shadcn controls and rows.
    assert!(!page.contains("Card {"));
    assert!(!page.contains("{card("));

    // ToggleGroup already invokes its handler in the Dioxus runtime. Queuing
    // this callback a second time makes state access panic on HarmonyOS.
    assert!(!page.contains("arkit::queue_ui_loop"));
}

#[test]
fn dashboard_long_values_are_width_constrained() {
    let page = section(VIEW, "fn dashboard_page", "fn proxies_page");

    assert!(page.contains("content: current_node"));
    assert!(page.contains("text_overflow: 2_i32"));
}

#[test]
fn quick_switch_owns_an_internal_virtual_scroll_list() {
    let page = section(VIEW, "fn dashboard_page", "fn proxies_page");
    let virtual_list = section(
        VIEW,
        "fn VirtualQuickProxyList",
        "fn render_virtual_proxy_card",
    );

    assert!(page.contains("layout_weight: 1.0"));
    assert!(virtual_list.contains("VirtualKind::List"));
    assert!(virtual_list.contains("percent_height: 1.0"));
    assert!(virtual_list.contains("list_cached_count: 20_i32"));

    let keys = section(
        VIEW,
        "fn virtual_quick_proxy_item_keys",
        "fn render_virtual_proxy_card",
    );
    assert!(keys.contains("item.name.hash"));
    assert!(keys.contains("item.selected.hash"));

    let quick_row = section(
        VIEW,
        "fn render_virtual_quick_proxy_row",
        "fn virtual_proxy_text",
    );
    assert!(quick_row.contains("selection_marker"));
    assert!(quick_row.contains("palette.success"));
}

#[test]
fn dashboard_list_meets_bottom_navigation_without_a_padding_strip() {
    let layout = section(VIEW, "fn scaffold", "fn use_parent_back_handler");
    let page = section(VIEW, "fn dashboard_page", "fn proxies_page");

    assert!(layout.contains("fn fixed_scaffold_flush_bottom"));
    assert!(layout.contains("if flush_fixed_bottom { 0.0 } else { 16.0 }"));
    assert!(page.contains("fixed_scaffold_flush_bottom"));
}
