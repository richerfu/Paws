#[path = "../src/proxy_filter.rs"]
mod proxy_filter;
#[path = "../src/proxy_grid.rs"]
mod proxy_grid;

use hmeta_model::{ProxyGroup, ProxyItem};
use proxy_grid::{flatten_proxy_groups, proxy_selection_chain, stabilize_proxy_items};

fn proxy(name: String, selected: bool) -> ProxyItem {
    ProxyItem {
        name,
        proxy_type: "vless".to_owned(),
        delay_ms: Some(42),
        selected,
    }
}

#[test]
fn flattens_large_proxy_groups_without_losing_group_context() {
    let groups = vec![ProxyGroup {
        name: "自动选择".to_owned(),
        group_type: "url-test".to_owned(),
        selected: Some("node-09999".to_owned()),
        fixed: Some(String::new()),
        proxies: (0..10_000)
            .map(|index| proxy(format!("node-{index:05}"), false))
            .collect(),
    }];

    let items = flatten_proxy_groups(&groups, "");

    assert_eq!(items.len(), 10_000);
    assert_eq!(items[0].name, "node-00000");
    assert_eq!(items[0].group, "自动选择");
    assert_eq!(items[0].group_type, "url-test");
    assert!(items[0].automatic);
    assert!(!items[0].pinned);
    assert!(!items[0].selected);
    assert_eq!(items[9_999].name, "node-09999");
    assert!(items[9_999].selected);
}

#[test]
fn automatic_group_exposes_its_pinned_node() {
    let groups = vec![ProxyGroup {
        name: "Auto".to_owned(),
        group_type: "URLTest".to_owned(),
        selected: Some("Singapore".to_owned()),
        fixed: Some("Singapore".to_owned()),
        proxies: vec![
            proxy("Hong Kong".to_owned(), false),
            proxy("Singapore".to_owned(), true),
        ],
    }];

    let items = flatten_proxy_groups(&groups, "");

    assert!(items.iter().all(|item| item.automatic));
    assert!(!items[0].pinned);
    assert!(items[1].pinned);
}

#[test]
fn changing_selection_never_reorders_subscription_nodes() {
    let build_group = |selected: &str| ProxyGroup {
        name: "Proxy".to_owned(),
        group_type: "select".to_owned(),
        selected: Some(selected.to_owned()),
        fixed: None,
        proxies: ["Hong Kong", "Singapore", "Japan"]
            .into_iter()
            .map(|name| proxy(name.to_owned(), name == selected))
            .collect(),
    };

    let before = flatten_proxy_groups(&[build_group("Hong Kong")], "")
        .into_iter()
        .map(|item| item.name)
        .collect::<Vec<_>>();
    let after = flatten_proxy_groups(&[build_group("Japan")], "")
        .into_iter()
        .map(|item| item.name)
        .collect::<Vec<_>>();

    assert_eq!(before, vec!["Hong Kong", "Singapore", "Japan"]);
    assert_eq!(after, before);
}

#[test]
fn quick_switch_keeps_first_seen_order_when_runtime_groups_move() {
    let item = |group: &str, name: &str| proxy_grid::ProxyGridItem {
        group: group.to_owned(),
        group_type: "Selector".to_owned(),
        name: name.to_owned(),
        proxy_type: "vless".to_owned(),
        delay_ms: None,
        selected: false,
        automatic: false,
        pinned: false,
    };
    let mut order = Vec::new();
    let initial = stabilize_proxy_items(&mut order, vec![item("GLOBAL", "A"), item("Proxy", "B")]);
    let refreshed =
        stabilize_proxy_items(&mut order, vec![item("Proxy", "B"), item("GLOBAL", "A")]);

    let names = |items: Vec<proxy_grid::ProxyGridItem>| {
        items.into_iter().map(|item| item.name).collect::<Vec<_>>()
    };
    assert_eq!(names(initial), vec!["A", "B"]);
    assert_eq!(names(refreshed), vec!["A", "B"]);
}

#[test]
fn flat_grid_search_matches_node_and_group_metadata() {
    let groups = vec![
        ProxyGroup {
            name: "香港节点".to_owned(),
            group_type: "select".to_owned(),
            selected: None,
            fixed: None,
            proxies: vec![proxy("HK Premium".to_owned(), false)],
        },
        ProxyGroup {
            name: "Fallback".to_owned(),
            group_type: "fallback".to_owned(),
            selected: None,
            fixed: Some(String::new()),
            proxies: vec![proxy("US Premium".to_owned(), false)],
        },
    ];

    assert_eq!(flatten_proxy_groups(&groups, "香港").len(), 1);
    assert_eq!(flatten_proxy_groups(&groups, "fallback").len(), 1);
    assert_eq!(flatten_proxy_groups(&groups, "premium").len(), 2);
}

#[test]
fn selector_chain_marks_only_the_effective_leaf_node() {
    let groups = vec![
        ProxyGroup {
            name: "GLOBAL".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("Proxy".to_owned()),
            fixed: None,
            proxies: vec![
                proxy("Proxy".to_owned(), true),
                proxy("DIRECT".to_owned(), false),
            ],
        },
        ProxyGroup {
            name: "Proxy".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("HK Premium".to_owned()),
            fixed: None,
            proxies: vec![
                proxy("HK Premium".to_owned(), true),
                proxy("US Premium".to_owned(), false),
            ],
        },
    ];

    let items = flatten_proxy_groups(&groups, "");
    let selected = items
        .iter()
        .filter(|item| item.selected)
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 3);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].group, "Proxy");
    assert_eq!(selected[0].name, "HK Premium");
    assert!(!items.iter().any(|item| item.name == "Proxy"));
}

#[test]
fn selecting_a_flat_leaf_activates_its_parent_chain() {
    let groups = vec![
        ProxyGroup {
            name: "GLOBAL".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("DIRECT".to_owned()),
            fixed: None,
            proxies: vec![
                proxy("Proxy".to_owned(), false),
                proxy("DIRECT".to_owned(), true),
            ],
        },
        ProxyGroup {
            name: "Proxy".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("US Premium".to_owned()),
            fixed: None,
            proxies: vec![
                proxy("HK Premium".to_owned(), false),
                proxy("US Premium".to_owned(), true),
            ],
        },
    ];

    assert_eq!(
        proxy_selection_chain(&groups, "Proxy", "HK Premium"),
        vec![
            ("Proxy".to_owned(), "HK Premium".to_owned()),
            ("GLOBAL".to_owned(), "Proxy".to_owned()),
        ]
    );
}
