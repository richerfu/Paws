use super::super::*;

pub(crate) fn proxies_page(state: Signal<State>) -> Element {
    let mut query = use_signal(String::new);
    let mut expanded_group = use_signal(|| None::<String>);
    let current = state.read().clone();
    let query_value = query();
    let expanded = expanded_group();
    let mut rows = Vec::new();
    let subscription_rows = grouped_proxy_rows(
        &current.snapshot.proxy_groups,
        &query_value,
        expanded.as_deref(),
    );
    if !subscription_rows.is_empty() {
        rows.push(ProxyGroupRow::Section);
        rows.extend(subscription_rows);
    }
    let matching_group_count = rows
        .iter()
        .filter(|row| matches!(row, ProxyGroupRow::Group(group) if !group.name.eq_ignore_ascii_case("GLOBAL")))
        .count();
    let matching_member_count = rows
        .iter()
        .filter_map(|row| match row {
            ProxyGroupRow::Group(group) if !group.name.eq_ignore_ascii_case("GLOBAL") => {
                Some(group.member_count)
            }
            _ => None,
        })
        .sum::<usize>();
    let global_node_count = rows
        .iter()
        .find_map(|row| match row {
            ProxyGroupRow::Group(group) if group.name.eq_ignore_ascii_case("GLOBAL") => {
                Some(group.member_count)
            }
            _ => None,
        })
        .unwrap_or(0);
    let result_summary = match current.locale {
        UiLocale::ZhCn => {
            format!(
                "{global_node_count} 个全局节点 · {matching_group_count} 个策略分组 · {matching_member_count} 个成员"
            )
        }
        UiLocale::En => {
            format!(
                "{global_node_count} global nodes · {matching_group_count} policy groups · {matching_member_count} members"
            )
        }
    };
    let palette = VirtualProxyPalette {
        surface: surface(),
        selected_surface: muted(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
    };
    let empty = !rows
        .iter()
        .any(|row| matches!(row, ProxyGroupRow::Group(_)));
    let body = rsx! {
        column {
            width: "100%",
            layout_weight: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).proxies_search_placeholder.to_owned()),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row {
                width: "100%",
                height: 34.0,
                align_items: "center",
                text {
                    content: result_summary,
                    font_size: 11.0,
                    font_color: subtle(),
                }
            }
            if empty {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    justify_content: "center",
                    {empty_state("git-branch", strings(current.locale).proxies_empty_title, strings(current.locale).proxies_empty_subtitle)}
                }
            } else {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    VirtualProxyGroupList {
                        rows,
                        locale: current.locale,
                        palette,
                        selection_pending: current.proxy_selection_pending.clone(),
                        on_toggle: move |group: String| {
                            let next = (expanded_group().as_deref() != Some(group.as_str()))
                                .then_some(group);
                            expanded_group.set(next);
                        },
                        on_select: move |(group, proxy): (String, String)| {
                            if proxy.is_empty() {
                                dispatch(state, Action::UnfixProxy { group });
                            } else {
                                dispatch(state, Action::SelectProxy { group, proxy });
                            }
                        },
                    }
                }
            }
        }
    };
    let proxy_delay_loading = current.proxy_delay_loading;
    let actions = rsx! {
        row {
            FlatButton {
                variant: FlatButtonVariant::Outline,
                size: ButtonSize::Icon,
                disabled: Some(proxy_delay_loading),
                onclick: move |_| {
                    if !proxy_delay_loading {
                        dispatch(state, Action::TestAllProxyDelays);
                    }
                },
                if proxy_delay_loading {
                    Spinner { size: 16.0, color: Some(text_color()) }
                } else {
                    {arkit::icon("gauge", 17.0, text_color())}
                }
            }
        }
    };
    fixed_scaffold(state, Route::Proxies {}, actions, body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct VirtualProxyPalette {
    pub(crate) surface: u32,
    pub(crate) selected_surface: u32,
    pub(crate) foreground: u32,
    pub(crate) muted_foreground: u32,
    pub(crate) border: u32,
    pub(crate) success: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VirtualProxyListState {
    rows: Vec<ProxyGroupRow>,
    locale: UiLocale,
    palette: VirtualProxyPalette,
    selection_pending: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum VirtualProxyRowKey {
    Section,
    Group(String),
    Member { group: String, proxy: String },
}

fn virtual_proxy_row_keys(rows: &[ProxyGroupRow]) -> Vec<VirtualProxyRowKey> {
    rows.iter()
        .map(|row| match row {
            ProxyGroupRow::Section => VirtualProxyRowKey::Section,
            ProxyGroupRow::Group(group) => VirtualProxyRowKey::Group(group.name.clone()),
            ProxyGroupRow::Member(member) => VirtualProxyRowKey::Member {
                group: member.group.clone(),
                proxy: member.name.clone(),
            },
        })
        .collect()
}

#[component]
pub(crate) fn VirtualProxyGroupList(
    rows: Vec<ProxyGroupRow>,
    locale: UiLocale,
    palette: VirtualProxyPalette,
    selection_pending: Option<(String, String)>,
    on_toggle: EventHandler<String>,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let item_keys = virtual_proxy_row_keys(&rows);
    let next_list_state = VirtualProxyListState {
        rows,
        locale,
        palette,
        selection_pending,
    };
    let mut list_state = use_signal(|| next_list_state.clone());
    use_effect(use_reactive(
        (&next_list_state,),
        move |(next_list_state,)| {
            if *list_state.peek() != next_list_state {
                list_state.set(next_list_state);
            }
        },
    ));

    let source = use_virtual_source_items_keyed(VirtualKind::List, item_keys, move |index| {
        rsx! {
            VirtualProxyRow {
                index,
                list_state,
                on_toggle,
                on_select,
            }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            list_cached_count: 20_i32,
        }
    }
}

#[component]
fn VirtualProxyRow(
    index: u32,
    list_state: Signal<VirtualProxyListState>,
    on_toggle: EventHandler<String>,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let current = list_state.read();
    let Some(row) = current.rows.get(index as usize).cloned() else {
        return rsx! {};
    };
    let locale = current.locale;
    let palette = current.palette;
    let selection_pending = current.selection_pending.clone();
    drop(current);

    match row {
        ProxyGroupRow::Section => rsx! {
            VirtualProxySectionRow { locale, palette }
        },
        ProxyGroupRow::Group(group) => rsx! {
            VirtualProxyGroupRow {
                group,
                locale,
                palette,
                on_toggle,
            }
        },
        ProxyGroupRow::Member(member) => rsx! {
            VirtualProxyMemberRow {
                member,
                locale,
                palette,
                selection_pending,
                on_select,
            }
        },
    }
}

#[component]
fn VirtualProxySectionRow(locale: UiLocale, palette: VirtualProxyPalette) -> Element {
    let title = tr(
        locale,
        "全局节点与策略分组",
        "Global node and policy groups",
    );
    let description = tr(
        locale,
        "全局模式使用所选节点；规则按分组名称命中并保留独立选择",
        "Global mode uses the selected node; rules keep independent group selections",
    );
    rsx! {
        column {
            width: "100%",
            height: 52.0,
            padding_top: 8.0,
            align_items: "start",
            justify_content: "center",
            text {
                content: title,
                font_size: 12.0,
                line_height: 17.0,
                font_weight: 700,
                font_color: palette.foreground,
            }
            text {
                content: description,
                width: "100%",
                font_size: 9.0,
                line_height: 14.0,
                font_color: palette.muted_foreground,
                max_lines: 1,
                text_overflow: "ellipsis",
            }
        }
    }
}

#[component]
fn VirtualProxyGroupRow(
    group: ProxyGroupHeaderRow,
    locale: UiLocale,
    palette: VirtualProxyPalette,
    on_toggle: EventHandler<String>,
) -> Element {
    let selected = group
        .selected
        .clone()
        .unwrap_or_else(|| tr(locale, "未选择", "Unselected").to_owned());
    let selection_mode = match group.fixed.as_deref() {
        Some("") => tr(locale, "自动", "Auto"),
        Some(_) => tr(locale, "已固定", "Pinned"),
        None if !group.selectable => tr(locale, "自动策略", "Automatic policy"),
        None => tr(locale, "手动选择", "Manual selection"),
    };
    let global_selector = group.name.eq_ignore_ascii_case("GLOBAL");
    let title = if global_selector {
        tr(locale, "全局节点", "Global node").to_owned()
    } else {
        group.name.clone()
    };
    let group_kind = if global_selector {
        tr(locale, "全局模式", "Global mode")
    } else {
        group.group_type.as_str()
    };
    let group_name = group.name.clone();
    rsx! {
        row {
            width: "100%",
            height: 78.0,
            background_color: palette.surface,
            padding_left: 13.0,
            padding_right: 12.0,
            margin_bottom: 6.0,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: 10.0,
            clip: true,
            align_items: "center",
            justify_content: "center",
            onclick: move |_| on_toggle.call(group_name.clone()),
            row {
                width: 34.0,
                height: 34.0,
                align_items: "center",
                justify_content: "center",
                background_color: palette.selected_surface,
                border_radius: 8.0,
                {arkit::icon("git-branch", 17.0, palette.foreground)}
            }
            column {
                layout_weight: 1.0,
                margin_left: 10.0,
                align_items: "start",
                text {
                    width: "100%",
                    content: title,
                    font_size: 13.0,
                    font_weight: 650,
                    font_color: palette.foreground,
                    line_height: 18.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: format!(
                        "{} · {} · {}",
                        group_kind,
                        selection_mode,
                        match locale {
                            UiLocale::ZhCn => format!("{} 个成员", group.member_count),
                            UiLocale::En => format!("{} members", group.member_count),
                        }
                    ),
                    font_size: 9.0,
                    line_height: 14.0,
                    font_color: palette.muted_foreground,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: format!("{} · {}", tr(locale, "当前", "Current"), selected),
                    font_size: 10.0,
                    line_height: 15.0,
                    font_weight: 600,
                    font_color: palette.success,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
            row {
                width: 26.0,
                height: 36.0,
                align_items: "center",
                justify_content: "center",
                {arkit::icon(if group.expanded { "chevron-up" } else { "chevron-down" }, 16.0, palette.muted_foreground)}
            }
        }
    }
}

#[component]
fn VirtualProxyMemberRow(
    member: ProxyGroupMemberRow,
    locale: UiLocale,
    palette: VirtualProxyPalette,
    selection_pending: Option<(String, String)>,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let pending_for_group = selection_pending
        .as_ref()
        .filter(|(pending_group, _)| pending_group == &member.group);
    let selected = pending_for_group
        .map(|(_, pending_proxy)| pending_proxy == &member.name)
        .unwrap_or(member.selected);
    let pending = pending_for_group.is_some();
    let delay = member
        .delay_ms
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| strings(locale).proxies_untested.to_owned());
    let detail = if member.subgroup {
        format!(
            "{} · {}",
            tr(locale, "子分组", "Subgroup"),
            member.proxy_type
        )
    } else if member.pinned {
        format!(
            "{} · {}",
            tr(locale, "已固定", "Pinned"),
            member.proxy_type.to_ascii_uppercase()
        )
    } else {
        member.proxy_type.to_ascii_uppercase()
    };
    let group = member.group.clone();
    let proxy = member.name.clone();
    // The reducer serializes proxy changes. Keeping this closure independent
    // from the list-wide pending state lets unaffected virtual rows stay
    // mounted without retaining a stale enabled/disabled value.
    let can_select = member.selectable;
    let unfix = member.pinned;
    rsx! {
        row {
            width: "100%",
            height: 62.0,
            background_color: if selected || member.pinned {
                palette.selected_surface
            } else {
                palette.surface
            },
            padding_right: 12.0,
            padding_left: 22.0,
            margin_bottom: 4.0,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: 9.0,
            clip: true,
            align_items: "center",
            onclick: move |_| {
                if can_select {
                    on_select.call((
                        group.clone(),
                        if unfix { String::new() } else { proxy.clone() },
                    ));
                }
            },
            row {
                width: 24.0,
                height: 32.0,
                align_items: "center",
                justify_content: "center",
                if pending && selected {
                    Spinner { size: 15.0, color: Some(palette.success) }
                } else {
                    {arkit::icon(
                        if selected { "circle-check" } else if member.subgroup { "folder-tree" } else { "circle" },
                        16.0,
                        if selected { palette.success } else { palette.muted_foreground },
                    )}
                }
            }
            column {
                layout_weight: 1.0,
                margin_left: 7.0,
                align_items: "start",
                justify_content: "center",
                text {
                    width: "100%",
                    content: member.name,
                    font_size: 13.0,
                    font_weight: if selected { 650 } else { 450 },
                    font_color: if selected { palette.success } else { palette.foreground },
                    line_height: 18.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: detail,
                    font_size: 9.0,
                    font_color: palette.muted_foreground,
                    line_height: 14.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
            text {
                content: if !member.selectable {
                    tr(locale, "自动", "Auto").to_owned()
                } else if member.pinned {
                    tr(locale, "恢复自动", "Use auto").to_owned()
                } else {
                    delay
                },
                margin_left: 8.0,
                font_size: 10.0,
                font_weight: if selected { 600 } else { 400 },
                font_color: if member.delay_ms.is_some() || selected {
                    palette.success
                } else {
                    palette.muted_foreground
                },
            }
        }
    }
}
