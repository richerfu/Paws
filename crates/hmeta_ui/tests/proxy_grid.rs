#[path = "../src/proxy_filter.rs"]
mod proxy_filter;
#[path = "../src/proxy_grid.rs"]
mod proxy_grid;

use hmeta_model::{ProxyGroup, ProxyItem};
use proxy_grid::{
    effective_group_leaf, grouped_proxy_rows, primary_selected_group_leaf, proxy_group_summary,
    ProxyGroupRow,
};

fn proxy(name: impl Into<String>, selected: bool) -> ProxyItem {
    ProxyItem {
        name: name.into(),
        proxy_type: "vless".to_owned(),
        delay_ms: Some(42),
        selected,
    }
}

fn group(name: &str, selected: &str, members: impl IntoIterator<Item = ProxyItem>) -> ProxyGroup {
    ProxyGroup {
        name: name.to_owned(),
        group_type: "Selector".to_owned(),
        selected: Some(selected.to_owned()),
        fixed: None,
        proxies: members.into_iter().collect(),
    }
}

#[test]
fn collapsed_groups_render_headers_without_eagerly_building_members() {
    let groups = vec![group(
        "Proxy",
        "Hong Kong",
        [proxy("Hong Kong", true), proxy("Japan", false)],
    )];

    let rows = grouped_proxy_rows(&groups, "", None);

    assert_eq!(rows.len(), 1);
    let ProxyGroupRow::Group(header) = &rows[0] else {
        panic!("group header");
    };
    assert_eq!(header.name, "Proxy");
    assert_eq!(header.selected.as_deref(), Some("Hong Kong"));
    assert_eq!(header.member_count, 2);
    assert!(!header.expanded);
}

#[test]
fn every_group_keeps_its_own_selected_member() {
    let groups = vec![
        group(
            "Google",
            "Hong Kong",
            [proxy("Hong Kong", true), proxy("Japan", false)],
        ),
        group(
            "Streaming",
            "United States",
            [proxy("Japan", false), proxy("United States", true)],
        ),
    ];

    let google = grouped_proxy_rows(&groups, "", Some("Google"));
    let streaming = grouped_proxy_rows(&groups, "", Some("Streaming"));

    let selected_member = |rows: &[ProxyGroupRow]| {
        rows.iter().find_map(|row| match row {
            ProxyGroupRow::Member(member) if member.selected => Some(member.name.clone()),
            _ => None,
        })
    };
    assert_eq!(selected_member(&google).as_deref(), Some("Hong Kong"));
    assert_eq!(
        selected_member(&streaming).as_deref(),
        Some("United States")
    );
}

#[test]
fn nested_group_members_remain_selectable_edges() {
    let groups = vec![
        group(
            "Parent",
            "Child",
            [proxy("Child", true), proxy("DIRECT", false)],
        ),
        group(
            "Child",
            "Hong Kong",
            [proxy("Hong Kong", true), proxy("Japan", false)],
        ),
    ];

    let rows = grouped_proxy_rows(&groups, "", Some("Parent"));
    let child = rows.iter().find_map(|row| match row {
        ProxyGroupRow::Member(member) if member.name == "Child" => Some(member),
        _ => None,
    });

    assert!(child.is_some_and(|member| member.subgroup && member.selected));
}

#[test]
fn global_selector_exposes_only_selectable_subscription_nodes() {
    let groups = vec![
        group(
            "GLOBAL",
            "Hong Kong",
            [
                proxy("Hong Kong", true),
                ProxyItem {
                    name: "DIRECT".to_owned(),
                    proxy_type: "Direct".to_owned(),
                    delay_ms: None,
                    selected: false,
                },
                proxy("Proxy", false),
            ],
        ),
        group("Proxy", "Hong Kong", [proxy("Hong Kong", true)]),
    ];

    let rows = grouped_proxy_rows(&groups, "", Some("GLOBAL"));
    let section = ProxyGroupRow::Section;

    assert!(matches!(section, ProxyGroupRow::Section));
    let global = rows.iter().find_map(|row| match row {
        ProxyGroupRow::Group(group) if group.name == "GLOBAL" => Some(group),
        _ => None,
    });
    assert_eq!(global.map(|group| group.member_count), Some(1));
    assert_eq!(
        global.and_then(|group| group.selected.as_deref()),
        Some("Hong Kong")
    );
    assert!(rows.iter().any(|row| {
        matches!(row, ProxyGroupRow::Member(member)
            if member.group == "GLOBAL" && member.name == "Hong Kong" && member.selectable)
    }));
    assert!(!rows.iter().any(|row| {
        matches!(row, ProxyGroupRow::Member(member)
            if member.group == "GLOBAL" && matches!(member.name.as_str(), "DIRECT" | "Proxy"))
    }));
    assert!(rows
        .iter()
        .any(|row| matches!(row, ProxyGroupRow::Group(group) if group.name == "Proxy")));
    assert_eq!(proxy_group_summary(&groups).groups, 1);
}

#[test]
fn searching_members_expands_only_matching_groups() {
    let groups = vec![
        group("Fallback", "US Premium", [proxy("US Premium", true)]),
        group("香港节点", "HK Premium", [proxy("HK Premium", true)]),
    ];

    let rows = grouped_proxy_rows(&groups, "premium", None);
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(row, ProxyGroupRow::Group(_)))
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(row, ProxyGroupRow::Member(_)))
            .count(),
        2
    );

    let fallback = grouped_proxy_rows(&groups, "fallback", None);
    assert!(matches!(
        fallback.as_slice(),
        [ProxyGroupRow::Group(_), ProxyGroupRow::Member(_)]
    ));
}

#[test]
fn effective_leaf_resolution_is_scoped_to_the_requested_group() {
    let groups = vec![
        group("GLOBAL", "Parent", [proxy("Parent", true)]),
        group("Parent", "Child", [proxy("Child", true)]),
        group("Child", "Hong Kong", [proxy("Hong Kong", true)]),
        group("Streaming", "Japan", [proxy("Japan", true)]),
    ];

    assert_eq!(
        effective_group_leaf(&groups, "GLOBAL").as_deref(),
        Some("Hong Kong")
    );
    assert_eq!(
        effective_group_leaf(&groups, "Streaming").as_deref(),
        Some("Japan")
    );
}

#[test]
fn primary_rule_selection_prefers_proxy_and_resolves_its_final_node() {
    let groups = vec![
        group("GLOBAL", "Hong Kong", [proxy("Hong Kong", true)]),
        group("Streaming", "United States", [proxy("United States", true)]),
        group("Proxy", "Regional", [proxy("Regional", true)]),
        group("Regional", "Japan", [proxy("Japan", true)]),
    ];

    assert_eq!(
        primary_selected_group_leaf(&groups).as_deref(),
        Some("Japan")
    );
}

#[test]
fn primary_rule_selection_falls_back_to_the_first_selected_subscription_group() {
    let groups = vec![
        group("GLOBAL", "Hong Kong", [proxy("Hong Kong", true)]),
        group("节点选择", "Hong Kong", [proxy("Hong Kong", true)]),
    ];

    assert_eq!(
        primary_selected_group_leaf(&groups).as_deref(),
        Some("Hong Kong")
    );
}

#[test]
fn group_order_is_deterministic_and_case_insensitive() {
    let groups = vec![
        group("zulu", "Z", [proxy("Z", true)]),
        group("Alpha", "A", [proxy("A", true)]),
    ];
    let rows = grouped_proxy_rows(&groups, "", None);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            ProxyGroupRow::Group(group) => Some(group.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["Alpha", "zulu"]);
}
