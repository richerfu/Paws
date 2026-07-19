use crate::proxy_filter::matches_proxy_query;
use hmeta_model::ProxyGroup;
use std::collections::{BTreeMap, BTreeSet};

/// A presentation item for Arkit's virtual Grid. Keeping this model flat means
/// the native adapter can address every node by index without eagerly building
/// a nested group tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProxyGridItem {
    pub(crate) group: String,
    pub(crate) group_type: String,
    pub(crate) name: String,
    pub(crate) proxy_type: String,
    pub(crate) delay_ms: Option<u32>,
    pub(crate) selected: bool,
}

pub(crate) fn flatten_proxy_groups(groups: &[ProxyGroup], query: &str) -> Vec<ProxyGridItem> {
    let normalized_query = query.trim().to_ascii_lowercase();
    let group_names = groups
        .iter()
        .map(|group| group.name.as_str())
        .collect::<BTreeSet<_>>();
    let active_proxy = resolve_active_proxy(groups);
    let capacity = groups.iter().map(|group| group.proxies.len()).sum();
    let mut items = Vec::with_capacity(capacity);
    for group in groups {
        let group_matches = normalized_query.is_empty()
            || group.name.to_ascii_lowercase().contains(&normalized_query)
            || group
                .group_type
                .to_ascii_lowercase()
                .contains(&normalized_query);
        for proxy in &group.proxies {
            // Selector-to-selector links are navigation edges, not real proxy
            // nodes. The flattened grid only exposes selectable leaf nodes.
            if group_names.contains(proxy.name.as_str()) {
                continue;
            }
            if !group_matches && !matches_proxy_query(proxy, &normalized_query) {
                continue;
            }
            items.push(ProxyGridItem {
                group: group.name.clone(),
                group_type: group.group_type.clone(),
                name: proxy.name.clone(),
                proxy_type: proxy.proxy_type.clone(),
                delay_ms: proxy.delay_ms,
                selected: active_proxy
                    .as_ref()
                    .is_some_and(|(active_group, active_name)| {
                        active_group == &group.name && active_name == &proxy.name
                    }),
            });
        }
    }

    items
}

/// Preserve the first-seen identity order while refreshing each item's latest
/// metadata. Runtime selector snapshots may return groups in a different
/// order after a selection; a visible quick-switch list must not follow that
/// incidental reordering.
pub(crate) fn stabilize_proxy_items(
    previous_order: &mut Vec<(String, String)>,
    items: Vec<ProxyGridItem>,
) -> Vec<ProxyGridItem> {
    let incoming_order = items
        .iter()
        .map(|item| (item.group.clone(), item.name.clone()))
        .collect::<Vec<_>>();
    let mut by_identity = items
        .into_iter()
        .map(|item| ((item.group.clone(), item.name.clone()), item))
        .collect::<BTreeMap<_, _>>();
    let mut stable = Vec::with_capacity(by_identity.len());

    for identity in previous_order.iter() {
        if let Some(item) = by_identity.remove(identity) {
            stable.push(item);
        }
    }
    for identity in incoming_order {
        if let Some(item) = by_identity.remove(&identity) {
            stable.push(item);
        }
    }

    *previous_order = stable
        .iter()
        .map(|item| (item.group.clone(), item.name.clone()))
        .collect();
    stable
}

/// Build the selector updates needed to make a flattened leaf node effective.
/// The leaf selector is updated first, followed by each parent up to GLOBAL.
pub(crate) fn proxy_selection_chain(
    groups: &[ProxyGroup],
    target_group: &str,
    target_proxy: &str,
) -> Vec<(String, String)> {
    let mut selections = vec![(target_group.to_owned(), target_proxy.to_owned())];
    if target_group.eq_ignore_ascii_case("GLOBAL") {
        return selections;
    }
    let Some(root) = groups
        .iter()
        .find(|group| group.name.eq_ignore_ascii_case("GLOBAL"))
    else {
        return selections;
    };
    let mut pending = vec![vec![root.name.clone()]];
    let mut target_path = None;
    while let Some(path) = pending.pop() {
        let Some(current_name) = path.last() else {
            continue;
        };
        if current_name == target_group {
            target_path = Some(path);
            break;
        }
        let Some(current_group) = groups.iter().find(|group| group.name == *current_name) else {
            continue;
        };
        for proxy in &current_group.proxies {
            if path.contains(&proxy.name) || !groups.iter().any(|group| group.name == proxy.name) {
                continue;
            }
            let mut next_path = path.clone();
            next_path.push(proxy.name.clone());
            pending.push(next_path);
        }
    }
    if let Some(path) = target_path {
        for pair in path.windows(2).rev() {
            selections.push((pair[0].clone(), pair[1].clone()));
        }
    }
    selections
}

fn resolve_active_proxy(groups: &[ProxyGroup]) -> Option<(String, String)> {
    let mut group = groups
        .iter()
        .find(|group| group.name.eq_ignore_ascii_case("GLOBAL"))
        .or_else(|| groups.iter().find(|group| group.selected.is_some()))?;
    let mut visited = BTreeSet::new();

    loop {
        if !visited.insert(group.name.as_str()) {
            return None;
        }
        let selected = group.selected.as_ref().or_else(|| {
            group
                .proxies
                .iter()
                .find(|proxy| proxy.selected)
                .map(|proxy| &proxy.name)
        })?;
        if let Some(next_group) = groups.iter().find(|candidate| candidate.name == *selected) {
            group = next_group;
            continue;
        }
        return Some((group.name.clone(), selected.clone()));
    }
}
