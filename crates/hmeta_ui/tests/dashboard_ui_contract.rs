const VIEW: &str = include_str!("../src/view.rs");
const DASHBOARD: &str = include_str!("../src/view/pages/dashboard.rs");
const PROXIES: &str = include_str!("../src/view/pages/proxies.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

fn section_to_end<'a>(source: &'a str, start: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    &source[start..]
}

#[test]
fn dashboard_stays_flat_and_decision_focused() {
    let page = DASHBOARD;

    assert!(page.contains("FlatSegmented"));
    assert!(page.contains("grouped_proxy_rows"));
    assert!(page.contains("VirtualProxyGroupList"));
    assert!(page.contains("fixed_scaffold_flush_bottom"));
    assert!(!page.contains("ProxyGroupScope"));
    assert!(!page.contains("snapshot.mode == RuntimeMode::Global"));
    assert!(page.contains("key: \"dashboard-quick-proxy-list\""));
    assert!(!page.contains("preview_proxies.truncate(3)"));
    assert!(!page.contains("row { height: 72.0 }"));
    assert!(page.contains("当前节点"));
    assert!(page.contains("全局节点与策略分组"));
    assert!(page.contains("global_node_count"));
    assert!(page.contains("出口 IP"));
    assert!(page.contains("snapshot.exit_location"));
    assert!(page.contains("height: 89.0"));
    assert_eq!(page.matches("dashboard_connection_row(").count(), 3);
    assert!(page.contains("height: 44.0"));
    assert!(page.contains("width: 68.0"));
    assert_eq!(page.matches("row { height: 14.0 }").count(), 3);
    assert_eq!(page.matches("Separator {}").count(), 1);
    assert!(!page.contains("VPN IP"));
    assert!(!page.contains("vpn_options.addresses"));
    assert!(!page.contains("format_speed"));

    // The home page uses one flat hierarchy. Cards belong on detail pages,
    // while immediate choices remain visible as shadcn controls and rows.
    assert!(!page.contains("Card {"));
    assert!(!page.contains("{card("));

    // Keep the mode selector on the app-owned segmented buttons. ToggleGroup's
    // nested hit-test surfaces have regressed on HarmonyOS across arkit
    // renderer revisions, leaving the visible labels unable to change mode.
    assert!(!page.contains("ToggleGroup"));
    assert!(!page.contains("arkit::queue_ui_loop"));
}

#[test]
fn dashboard_mode_selector_dispatches_every_runtime_mode() {
    let picker = section_to_end(DASHBOARD, "fn mode_picker");

    assert!(picker.contains("RuntimeMode::Rule"));
    assert!(picker.contains("RuntimeMode::Global"));
    assert!(picker.contains("RuntimeMode::Direct"));
    assert!(picker.contains("dispatch(state, Action::SetMode(mode))"));
}

#[test]
fn dashboard_long_values_are_width_constrained() {
    let page = DASHBOARD;

    assert!(page.contains("current_node,"));
    assert!(page.contains("text_overflow: \"ellipsis\""));
    assert!(page.contains("RuntimeMode::Direct"));
    assert!(page.contains("RuntimeMode::Global"));
    assert!(page.contains("RuntimeMode::Rule"));
    assert!(page.contains("effective_group_leaf"));
    assert!(page.contains("latest_active_rule_node"));
    assert!(page.contains("primary_selected_group_leaf"));
    assert!(!page.contains("由命中规则的策略分组决定"));
    assert!(!page.contains("暂无命中"));
    assert!(page.contains("s.proxies_direct.to_owned()"));
}

#[test]
fn quick_switch_owns_an_internal_virtual_scroll_list() {
    let page = DASHBOARD;
    let virtual_list = section(
        PROXIES,
        "fn VirtualProxyGroupList",
        "fn VirtualProxySectionRow",
    );

    assert!(page.contains("layout_weight: 1.0"));
    assert!(virtual_list.contains("VirtualKind::List"));
    assert!(virtual_list.contains("height: \"100%\""));
    assert!(virtual_list.contains("list_cached_count: 20_i32"));

    let keys = section(
        PROXIES,
        "fn virtual_proxy_row_keys",
        "fn VirtualProxyGroupList",
    );
    assert!(keys.contains("VirtualProxyRowKey::Group"));
    assert!(keys.contains("VirtualProxyRowKey::Member"));
    assert!(!keys.contains("selection_pending"));
    assert!(virtual_list.contains("use_signal"));
    assert!(virtual_list.contains("VirtualProxyRow"));

    let group_row = section(
        PROXIES,
        "fn VirtualProxyGroupRow",
        "fn VirtualProxyMemberRow",
    );
    assert!(group_row.contains("let selected = group"));
    assert!(group_row.contains(".selected"));
    assert!(group_row.contains("group.expanded"));
    assert!(!group_row.contains("selection_pending"));
    assert!(!group_row.contains("切换中"));

    let member_row = section_to_end(PROXIES, "fn VirtualProxyMemberRow");
    assert!(member_row.contains("member.selected"));
    assert!(member_row.contains("member.subgroup"));
    assert!(member_row.contains("let can_select = member.selectable;"));
}

#[test]
fn dashboard_list_meets_bottom_navigation_without_a_padding_strip() {
    let layout = section(VIEW, "fn scaffold", "fn use_parent_back_handler");
    let page = DASHBOARD;

    assert!(layout.contains("fn fixed_scaffold_flush_bottom"));
    assert!(layout.contains("if flush_fixed_bottom { 0.0 } else { spacing::LG }"));
    assert!(page.contains("fixed_scaffold_flush_bottom"));
}
