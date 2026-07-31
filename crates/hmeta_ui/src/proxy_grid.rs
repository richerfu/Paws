use crate::proxy_filter::matches_proxy_query;
use hmeta_model::{ProxyGroup, ProxyItem};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ProxyGroupRow {
    Section,
    Group(ProxyGroupHeaderRow),
    Member(ProxyGroupMemberRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProxyGroupHeaderRow {
    pub(crate) name: String,
    pub(crate) group_type: String,
    pub(crate) selected: Option<String>,
    pub(crate) fixed: Option<String>,
    pub(crate) member_count: usize,
    pub(crate) expanded: bool,
    pub(crate) selectable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProxyGroupMemberRow {
    pub(crate) group: String,
    pub(crate) group_type: String,
    pub(crate) name: String,
    pub(crate) proxy_type: String,
    pub(crate) delay_ms: Option<u32>,
    pub(crate) selected: bool,
    pub(crate) automatic: bool,
    pub(crate) pinned: bool,
    pub(crate) subgroup: bool,
    pub(crate) selectable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProxyGroupSummary {
    pub(crate) groups: usize,
    pub(crate) members: usize,
}

pub(crate) fn grouped_proxy_rows(
    groups: &[ProxyGroup],
    query: &str,
    expanded_group: Option<&str>,
) -> Vec<ProxyGroupRow> {
    let normalized_query = query.trim().to_ascii_lowercase();
    let group_names = groups
        .iter()
        .map(|group| group.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut visible_groups = groups
        .iter()
        .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
        .collect::<Vec<_>>();
    visible_groups.sort_by_cached_key(|group| group.name.to_ascii_lowercase());

    let mut rows = Vec::new();
    for group in visible_groups {
        let selected = selected_member(group);
        let group_matches = normalized_query.is_empty()
            || text_matches(&group.name, &normalized_query)
            || text_matches(&group.group_type, &normalized_query)
            || selected
                .as_deref()
                .is_some_and(|name| text_matches(name, &normalized_query));
        let matching_members = group
            .proxies
            .iter()
            .filter(|proxy| group_matches || matches_proxy_query(proxy, &normalized_query))
            .collect::<Vec<_>>();
        if !group_matches && matching_members.is_empty() {
            continue;
        }

        let expanded = expanded_group == Some(group.name.as_str())
            || (!normalized_query.is_empty() && !matching_members.is_empty());
        let selectable = selectable_group_type(&group.group_type);
        rows.push(ProxyGroupRow::Group(ProxyGroupHeaderRow {
            name: group.name.clone(),
            group_type: group.group_type.clone(),
            selected: selected.clone(),
            fixed: group.fixed.clone(),
            member_count: group.proxies.len(),
            expanded,
            selectable,
        }));

        if expanded {
            rows.extend(matching_members.into_iter().map(|proxy| {
                ProxyGroupRow::Member(proxy_member_row(
                    group,
                    proxy,
                    selected.as_deref(),
                    selectable,
                    group_names.contains(proxy.name.as_str()),
                ))
            }));
        }
    }
    rows
}

pub(crate) fn proxy_group_summary(groups: &[ProxyGroup]) -> ProxyGroupSummary {
    groups
        .iter()
        .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
        .fold(ProxyGroupSummary::default(), |summary, group| {
            ProxyGroupSummary {
                groups: summary.groups + 1,
                members: summary.members + group.proxies.len(),
            }
        })
}

pub(crate) fn effective_group_leaf(groups: &[ProxyGroup], root: &str) -> Option<String> {
    let mut group = groups.iter().find(|group| group.name == root)?;
    let mut visited = std::collections::BTreeSet::new();
    loop {
        if !visited.insert(group.name.as_str()) {
            return None;
        }
        let selected = selected_member(group)?;
        if let Some(next_group) = groups.iter().find(|candidate| candidate.name == selected) {
            group = next_group;
        } else {
            return Some(selected);
        }
    }
}

pub(crate) fn primary_selected_group_leaf(groups: &[ProxyGroup]) -> Option<String> {
    let visible_groups = groups
        .iter()
        .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
        .collect::<Vec<_>>();
    let primary = visible_groups
        .iter()
        .copied()
        .find(|group| group.name.eq_ignore_ascii_case("Proxy") && selected_member(group).is_some())
        .or_else(|| {
            visible_groups
                .into_iter()
                .find(|group| selected_member(group).is_some())
        })?;
    effective_group_leaf(groups, &primary.name)
}

fn selected_member(group: &ProxyGroup) -> Option<String> {
    group.selected.clone().or_else(|| {
        group
            .proxies
            .iter()
            .find(|proxy| proxy.selected)
            .map(|proxy| proxy.name.clone())
    })
}

fn proxy_member_row(
    group: &ProxyGroup,
    proxy: &ProxyItem,
    selected: Option<&str>,
    selectable: bool,
    subgroup: bool,
) -> ProxyGroupMemberRow {
    ProxyGroupMemberRow {
        group: group.name.clone(),
        group_type: group.group_type.clone(),
        name: proxy.name.clone(),
        proxy_type: proxy.proxy_type.clone(),
        delay_ms: proxy.delay_ms,
        selected: selected == Some(proxy.name.as_str()),
        automatic: group.fixed.is_some(),
        pinned: group.fixed.as_deref() == Some(proxy.name.as_str()),
        subgroup,
        selectable,
    }
}

fn selectable_group_type(group_type: &str) -> bool {
    matches!(
        group_type
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .as_str(),
        "select" | "selector" | "urltest" | "fallback"
    )
}

fn text_matches(value: &str, normalized_query: &str) -> bool {
    value.to_ascii_lowercase().contains(normalized_query)
}
