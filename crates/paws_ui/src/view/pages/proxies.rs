use super::super::*;

pub(crate) fn proxies_page() -> Element {
    let services = use_context::<UiServices>();
    let selection_services = services.clone();
    let mut query = use_signal(String::new);
    let mut expanded_group = use_signal(|| None::<String>);
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let proxies = stores.proxies.read();
    let query_value = query();
    let expanded = expanded_group();
    let mut rows = Vec::new();
    let subscription_rows = grouped_proxy_rows(&proxies.groups, &query_value, expanded.as_deref());
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
    let result_summary = match locale {
        UiLocale::ZhCn => translate_ui(
            locale,
            tr::hard_zh_018(
                global_node_count,
                matching_group_count,
                matching_member_count,
            ),
        ),
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
                placeholder: Some(translate_ui(locale, tr::proxies_search_placeholder())),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row {
                width: "100%",
                height: 34.0,
                align_items: "center",
                text {
                    content: result_summary,
                    font_size: typography::XS,
                    font_color: subtle(),
                }
            }
            if empty {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    justify_content: "center",
                    {empty_state("git-branch", translate_ui(locale, tr::proxies_empty_title()), translate_ui(locale, tr::proxies_empty_subtitle()))}
                }
            } else {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    VirtualProxyGroupList {
                        rows,
                        locale,
                        palette,
                        on_toggle: move |group: String| {
                            let next = (expanded_group().as_deref() != Some(group.as_str()))
                                .then_some(group);
                            expanded_group.set(next);
                        },
                        on_select: move |(group, proxy): (String, String)| {
                            let proxy = (!proxy.is_empty()).then_some(proxy);
                            selection_services.select_proxy(group, proxy);
                        },
                    }
                }
            }
        }
    };
    let actions = rsx! {
        ProxyDelayAction {}
    };
    fixed_scaffold(Route::Proxies {}, actions, body)
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
    on_toggle: EventHandler<String>,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let selection_pending = use_context::<UiOperationStores>()
        .proxy
        .read()
        .selection_pending
        .clone();
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

    // Row projections are read through list_state; only identity is structural.
    let stamps = item_keys
        .into_iter()
        .map(|id| VirtualItemStamp::new(id, ()))
        .collect();
    let source = use_virtual_items(VirtualKind::List, stamps, move |index| {
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
fn ProxyDelayAction() -> Element {
    let services = use_context::<UiServices>();
    let operations = use_context::<UiOperationStores>();
    let loading = operations.proxy.read().delay_loading;
    rsx! {
        row {
            FlatButton {
                variant: FlatButtonVariant::Outline,
                size: ButtonSize::Icon,
                disabled: Some(loading),
                onclick: move |_| {
                    if !operations.proxy.peek().delay_loading {
                        services.test_all_proxy_delays();
                    }
                },
                if loading {
                    Spinner { size: 16.0, color: Some(text_color()) }
                } else {
                    {arkit::icon("gauge", 17.0, text_color())}
                }
            }
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
    let title = translate_ui(locale, tr::hard_zh_019());
    let description = translate_ui(locale, tr::hard_zh_020());
    rsx! {
        column {
            width: "100%",
            height: 52.0,
            padding_top: 8.0,
            align_items: "start",
            justify_content: "center",
            text {
                content: title,
                font_size: typography::SM,
                line_height: 20.0,
                font_weight: 500,
                font_color: palette.muted_foreground,
            }
            text {
                content: description,
                width: "100%",
                font_size: typography::XS,
                line_height: 16.0,
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
        .unwrap_or_else(|| translate_ui(locale, tr::page_tr_154()));
    let selection_mode = match group.fixed.as_deref() {
        Some("") => translate_ui(locale, tr::page_tr_155()),
        Some(_) => translate_ui(locale, tr::page_tr_156()),
        None if !group.selectable => translate_ui(locale, tr::page_tr_157()),
        None => translate_ui(locale, tr::page_tr_158()),
    };
    let global_selector = group.name.eq_ignore_ascii_case("GLOBAL");
    let title = if global_selector {
        translate_ui(locale, tr::page_tr_159())
    } else {
        group.name.clone()
    };
    let group_kind = if global_selector {
        translate_ui(locale, tr::page_tr_160())
    } else {
        group.group_type.clone()
    };
    let group_name = group.name.clone();
    rsx! {
        row {
            width: "100%",
            height: 86.0,
            background_color: palette.surface,
            padding_left: spacing::MD,
            padding_right: spacing::MD,
            margin_bottom: spacing::SM,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
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
                border_radius: radius::MD,
                {arkit::icon("git-branch", 17.0, palette.foreground)}
            }
            column {
                layout_weight: 1.0,
                margin_left: 10.0,
                align_items: "start",
                text {
                    width: "100%",
                    content: title,
                    font_size: typography::SM,
                    font_weight: 600,
                    font_color: palette.foreground,
                    line_height: 20.0,
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
                            UiLocale::ZhCn => translate_ui(locale, tr::hard_zh_021(group.member_count)),
                            UiLocale::En => format!("{} members", group.member_count),
                        }
                    ),
                    font_size: typography::XS,
                    line_height: 16.0,
                    font_color: palette.muted_foreground,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: format!("{} · {}", translate_ui(locale, tr::page_tr_161()), selected),
                    font_size: typography::XS,
                    line_height: 16.0,
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
        .unwrap_or_else(|| translate_ui(locale, tr::proxies_untested()));
    let detail = if member.subgroup {
        format!(
            "{} · {}",
            translate_ui(locale, tr::page_tr_162()),
            member.proxy_type
        )
    } else if member.pinned {
        format!(
            "{} · {}",
            translate_ui(locale, tr::page_tr_156()),
            member.proxy_type.to_ascii_uppercase()
        )
    } else {
        member.proxy_type.to_ascii_uppercase()
    };
    let group = member.group.clone();
    let proxy = member.name.clone();
    // UiServices admits only one proxy-selection operation at a time, while
    // the core rejects a completion from an older config revision. Reading the
    // narrow pending signal here keeps unaffected virtual rows mounted without
    // retaining a stale enabled/disabled value.
    let can_select = member.selectable;
    let unfix = member.pinned;
    rsx! {
        row {
            width: "100%",
            height: 66.0,
            background_color: if selected || member.pinned {
                palette.selected_surface
            } else {
                palette.surface
            },
            padding_right: spacing::MD,
            padding_left: 22.0,
            margin_bottom: spacing::XXS,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
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
                    {virtual_loading_indicator(15.0, palette.success)}
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
                    font_size: typography::SM,
                    font_weight: if selected { 650 } else { 450 },
                    font_color: if selected { palette.success } else { palette.foreground },
                    line_height: 20.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: detail,
                    font_size: typography::XS,
                    font_color: palette.muted_foreground,
                    line_height: 16.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
            text {
                content: if !member.selectable {
                    translate_ui(locale, tr::page_tr_155())
                } else if member.pinned {
                    translate_ui(locale, tr::page_tr_163())
                } else {
                    delay
                },
                margin_left: 8.0,
                font_size: typography::XS,
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
