use super::super::*;

// shadcn-style list cards: room for host / meta / traffic rows with badge + action.
const REQUEST_ROW_HEIGHT: f32 = 88.0;
const CONNECTION_ROW_HEIGHT: f32 = 88.0;
const ACTIVITY_CARD_RADIUS: f32 = 12.0;
const ACTIVITY_CARD_GAP: f32 = 8.0;
/// shadcn Button size="icon" (h-8 w-8).
const ACTIVITY_ACTION_SIZE: f32 = 28.0;

pub(crate) fn requests_page(state: Signal<State>) -> Element {
    let mut request_query = use_signal(String::new);
    let mut request_filter = use_signal(|| RequestStatusFilter::All);
    let current = state.read().clone();
    let navigator = use_navigator();
    let query_value = request_query();
    let filter_value = request_filter();
    let all_label = translate_ui(current.locale, tr::requests_status_all());
    let active_label = translate_ui(current.locale, tr::requests_status_active());
    let ended_label = translate_ui(current.locale, tr::requests_status_ended());
    let filter_options = vec![all_label.clone(), active_label.clone(), ended_label.clone()];
    let selected_filter = match filter_value {
        RequestStatusFilter::All => all_label.clone(),
        RequestStatusFilter::Active => active_label.clone(),
        RequestStatusFilter::Ended => ended_label.clone(),
    };
    let rows = current
        .snapshot
        .request_history
        .iter()
        .filter(|request| matches_request_filter(request, filter_value, &query_value))
        .map(|request| VirtualRequestRow {
            id: request.id.clone(),
            host: request.host.clone(),
            domain: request.domain.clone(),
            destination_ip: request.destination_ip.clone(),
            metadata: format!(
                "{} · {} · {}",
                request.network.to_ascii_uppercase(),
                truncate_text(&request.proxy, 28),
                truncate_text(&request.rule, 32),
            ),
            traffic: format!(
                "↓ {}  ·  ↑ {}",
                format_total(request.download_bytes),
                format_total(request.upload_bytes),
            ),
            updated_at: format_activity_timestamp(&request.updated_at),
            status: if request.active {
                active_label.clone()
            } else {
                ended_label.clone()
            },
            active: request.active,
            connection_query: request_connection_query(request),
            rule_accessibility: translate_ui(current.locale, tr::page_tr_004()),
        })
        .collect::<Vec<_>>();
    let empty = rows.is_empty();
    let theme = use_theme();
    let palette = VirtualActivityPalette {
        surface: theme.colors.card,
        foreground: theme.colors.foreground,
        muted_foreground: theme.colors.muted_foreground,
        muted: theme.colors.muted,
        secondary: theme.colors.secondary,
        border: theme.colors.border,
        success: success(),
        success_soft: match theme.mode {
            ThemeMode::Light => 0xFFDCFCE7,
            ThemeMode::Dark => 0xFF14532D,
        },
        danger: theme.colors.destructive,
        radius: ACTIVITY_CARD_RADIUS,
    };
    let body = rsx! {
        column {
            width: "100%",
            height: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(translate_ui(current.locale, tr::requests_search_placeholder())),
                width: Some("100%".into()),
                on_change: move |value| request_query.set(value),
            }
            row { height: spacing::MD }
            FlatSegmented {
                options: filter_options,
                selected: selected_filter,
                on_change: move |value: String| {
                    let filter = if value == active_label {
                        RequestStatusFilter::Active
                    } else if value == ended_label {
                        RequestStatusFilter::Ended
                    } else {
                        RequestStatusFilter::All
                    };
                    request_filter.set(filter);
                },
            }
            row { height: spacing::MD }
            row {
                layout_weight: 1.0,
                width: "100%",
                if empty {
                    {empty_state("activity", translate_ui(current.locale, tr::requests_empty_title()), translate_ui(current.locale, tr::requests_empty_subtitle()))}
                } else {
                    VirtualRequestList {
                        items: rows,
                        palette,
                        on_open: move |query: String| {
                            navigator.push(Route::Connections { query });
                        },
                        on_add_rule: move |context: ManualRuleContext| {
                            dispatch(state, Action::OpenManualRuleEditor {
                                connection_id: context.connection_id,
                                domain: context.domain,
                                destination_ip: context.destination_ip,
                            });
                        },
                    }
                }
            }
        }
    };
    let page = fixed_scaffold(
        state,
        Route::Requests {},
        destructive_icon_action("trash-2", Action::ClearRequestHistory, state),
        body,
    );
    rsx! {
        {page}
        if current.manual_rule_editor.is_some() {
            {manual_rule_dialog(state, &current)}
        }
    }
}

pub(crate) fn connections_page(state: Signal<State>, initial_query: String) -> Element {
    let mut query = use_signal(move || initial_query);
    let current = state.read().clone();
    let query_value = query();
    let rows = current
        .snapshot
        .connections
        .iter()
        .filter(|connection| matches_connection_query(connection, &query_value))
        .map(|connection| {
            let chain = if connection.chains.is_empty() {
                "DIRECT".to_owned()
            } else {
                connection.chains.join(" > ")
            };
            VirtualConnectionRow {
                id: connection.id.clone(),
                host: connection.host.clone(),
                domain: connection.domain.clone(),
                destination_ip: connection.destination_ip.clone(),
                // Same density as request rows: one muted meta line.
                metadata: format!(
                    "{} · {} · {} · {}",
                    connection.network.to_ascii_uppercase(),
                    truncate_text(&connection.proxy, 24),
                    truncate_text(&connection.rule, 24),
                    truncate_text(&chain, 24),
                ),
                traffic: format!(
                    "↓ {}  ·  ↑ {}",
                    format_total(connection.download_bytes),
                    format_total(connection.upload_bytes),
                ),
                started_at: format_activity_timestamp(&connection.started_at),
                close_accessibility: translate_ui(current.locale, tr::connections_close()),
                rule_accessibility: translate_ui(current.locale, tr::page_tr_004()).to_owned(),
            }
        })
        .collect::<Vec<_>>();
    let empty = rows.is_empty();
    let theme = use_theme();
    let palette = VirtualActivityPalette {
        surface: theme.colors.card,
        foreground: theme.colors.foreground,
        muted_foreground: theme.colors.muted_foreground,
        muted: theme.colors.muted,
        secondary: theme.colors.secondary,
        border: theme.colors.border,
        success: success(),
        success_soft: match theme.mode {
            ThemeMode::Light => 0xFFDCFCE7,
            ThemeMode::Dark => 0xFF14532D,
        },
        danger: theme.colors.destructive,
        radius: ACTIVITY_CARD_RADIUS,
    };
    let body = rsx! {
        column {
            width: "100%",
            height: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(translate_ui(current.locale, tr::connections_search_placeholder())),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row { height: spacing::MD }
            row {
                layout_weight: 1.0,
                width: "100%",
                if empty {
                    {empty_state("unplug", translate_ui(current.locale, tr::connections_empty_title()), translate_ui(current.locale, tr::connections_empty_subtitle()))}
                } else {
                    VirtualConnectionList {
                        items: rows,
                        palette,
                        on_close: move |id: String| dispatch(state, Action::CloseConnection(id)),
                        on_add_rule: move |context: ManualRuleContext| {
                            dispatch(state, Action::OpenManualRuleEditor {
                                connection_id: context.connection_id,
                                domain: context.domain,
                                destination_ip: context.destination_ip,
                            });
                        },
                    }
                }
            }
        }
    };
    let page = fixed_scaffold(
        state,
        Route::Connections {
            query: String::new(),
        },
        destructive_icon_action("circle-x", Action::CloseAllConnections, state),
        body,
    );
    rsx! {
        {page}
        if current.manual_rule_editor.is_some() {
            {manual_rule_dialog(state, &current)}
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualRequestRow {
    id: String,
    host: String,
    domain: String,
    destination_ip: String,
    metadata: String,
    traffic: String,
    updated_at: String,
    status: String,
    active: bool,
    connection_query: String,
    rule_accessibility: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualConnectionRow {
    id: String,
    host: String,
    domain: String,
    destination_ip: String,
    metadata: String,
    traffic: String,
    started_at: String,
    close_accessibility: String,
    rule_accessibility: String,
}

#[derive(Clone)]
struct ManualRuleContext {
    connection_id: Option<String>,
    domain: String,
    destination_ip: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VirtualActivityPalette {
    surface: u32,
    foreground: u32,
    muted_foreground: u32,
    muted: u32,
    secondary: u32,
    border: u32,
    success: u32,
    success_soft: u32,
    danger: u32,
    radius: f32,
}

impl Hash for VirtualActivityPalette {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.surface.hash(state);
        self.foreground.hash(state);
        self.muted_foreground.hash(state);
        self.muted.hash(state);
        self.secondary.hash(state);
        self.border.hash(state);
        self.success.hash(state);
        self.success_soft.hash(state);
        self.danger.hash(state);
        self.radius.to_bits().hash(state);
    }
}

#[component]
fn VirtualRequestList(
    items: Vec<VirtualRequestRow>,
    palette: VirtualActivityPalette,
    on_open: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
) -> Element {
    let item_keys = activity_item_keys(&items, palette);
    let render_items = items;
    let source = use_virtual_source_items_keyed(VirtualKind::List, item_keys, move |index| {
        let Some(item) = render_items.get(index as usize).cloned() else {
            return rsx! {};
        };
        rsx! {
            VirtualRequestRowView { item, palette, on_open, on_add_rule }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            list_cached_count: 18_i32,
        }
    }
}

#[component]
fn VirtualConnectionList(
    items: Vec<VirtualConnectionRow>,
    palette: VirtualActivityPalette,
    on_close: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
) -> Element {
    let item_keys = activity_item_keys(&items, palette);
    let render_items = items;
    let source = use_virtual_source_items_keyed(VirtualKind::List, item_keys, move |index| {
        let Some(item) = render_items.get(index as usize).cloned() else {
            return rsx! {};
        };
        rsx! {
            VirtualConnectionRowView { item, palette, on_close, on_add_rule }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            list_cached_count: 18_i32,
        }
    }
}

fn activity_item_keys<T: Hash>(items: &[T], palette: VirtualActivityPalette) -> Vec<u64> {
    items
        .iter()
        .map(|item| {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            palette.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

#[component]
fn VirtualRequestRowView(
    item: VirtualRequestRow,
    palette: VirtualActivityPalette,
    on_open: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
) -> Element {
    let connection_query = item.connection_query.clone();
    let rule_context = ManualRuleContext {
        connection_id: item.active.then(|| item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    let accessibility_text = format!(
        "{}，{}，{}，{}，{}",
        item.host, item.status, item.metadata, item.traffic, item.updated_at,
    );
    rsx! {
        column {
            width: "100%",
            height: REQUEST_ROW_HEIGHT,
            background_color: palette.surface,
            padding_top: 12.0,
            padding_right: 14.0,
            padding_bottom: 12.0,
            padding_left: 14.0,
            margin_bottom: ACTIVITY_CARD_GAP,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: palette.radius,
            clip: true,
            align_items: "start",
            justify_content: "center",
            row {
                width: "100%",
                height: ACTIVITY_ACTION_SIZE,
                align_items: "center",
                if item.active {
                    row {
                        layout_weight: 1.0,
                        height: 20.0,
                        align_items: "center",
                        onclick: move |_| on_open.call(connection_query.clone()),
                        text {
                            width: "100%",
                            content: item.host.clone(),
                            font_size: typography::SM,
                            font_weight: 600,
                            font_color: palette.foreground,
                            line_height: 20.0,
                            max_lines: 1,
                            text_overflow: "ellipsis",
                        }
                    }
                } else {
                    row {
                        layout_weight: 1.0,
                        height: 20.0,
                        align_items: "center",
                        text {
                            width: "100%",
                            content: item.host.clone(),
                            font_size: typography::SM,
                            font_weight: 600,
                            font_color: palette.foreground,
                            line_height: 20.0,
                            max_lines: 1,
                            text_overflow: "ellipsis",
                        }
                    }
                }
                VirtualStatusBadge {
                    label: item.status.clone(),
                    background: if item.active { palette.success_soft } else { palette.secondary },
                    foreground: if item.active { palette.success } else { palette.muted_foreground },
                }
                VirtualIconAction {
                    content: "+".to_owned(),
                    font_size: 18.0,
                    foreground: palette.muted_foreground,
                    margin_right: 0.0,
                    accessibility: item.rule_accessibility.clone(),
                    on_click: move |_| on_add_rule.call(rule_context.clone()),
                }
            }
            text {
                width: "100%",
                content: item.metadata.clone(),
                font_size: typography::XS,
                font_weight: 400,
                font_color: palette.muted_foreground,
                line_height: 16.0,
                max_lines: 1,
                text_overflow: "ellipsis",
            }
            row {
                width: "100%",
                height: 16.0,
                align_items: "center",
                row {
                    layout_weight: 1.0,
                    height: 16.0,
                    align_items: "center",
                    text {
                        width: "100%",
                        content: item.traffic.clone(),
                        font_size: typography::XS,
                        font_weight: 500,
                        font_color: palette.foreground,
                        line_height: 16.0,
                        max_lines: 1,
                        text_overflow: "ellipsis",
                    }
                }
                text {
                    content: item.updated_at.clone(),
                    font_size: typography::XS,
                    font_weight: 400,
                    font_color: palette.muted_foreground,
                    line_height: 16.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
            text { content: accessibility_text, width: 0.0, height: 0.0, opacity: 0.0 }
        }
    }
}

#[component]
fn VirtualConnectionRowView(
    item: VirtualConnectionRow,
    palette: VirtualActivityPalette,
    on_close: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
) -> Element {
    // Mirror request-row layout:
    // [host                    ] [+] [×]
    // [metadata]
    // [traffic                     ] [time]
    let rule_context = ManualRuleContext {
        connection_id: Some(item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    let close_id = item.id.clone();
    let accessibility_text = format!(
        "{}，{}，{}，{}",
        item.host, item.metadata, item.traffic, item.started_at,
    );
    rsx! {
        column {
            width: "100%",
            height: CONNECTION_ROW_HEIGHT,
            background_color: palette.surface,
            padding_top: 12.0,
            padding_right: 14.0,
            padding_bottom: 12.0,
            padding_left: 14.0,
            margin_bottom: ACTIVITY_CARD_GAP,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: palette.radius,
            clip: true,
            align_items: "start",
            justify_content: "center",
            row {
                width: "100%",
                height: ACTIVITY_ACTION_SIZE,
                align_items: "center",
                row {
                    layout_weight: 1.0,
                    height: 20.0,
                    align_items: "center",
                    text {
                        width: "100%",
                        content: item.host.clone(),
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: palette.foreground,
                        line_height: 20.0,
                        max_lines: 1,
                        text_overflow: "ellipsis",
                    }
                }
                VirtualIconAction {
                    content: "+".to_owned(),
                    font_size: 18.0,
                    foreground: palette.muted_foreground,
                    margin_right: spacing::MD,
                    accessibility: item.rule_accessibility.clone(),
                    on_click: move |_| on_add_rule.call(rule_context.clone()),
                }
                VirtualIconAction {
                    content: "×".to_owned(),
                    font_size: 16.0,
                    foreground: palette.danger,
                    margin_right: 0.0,
                    accessibility: item.close_accessibility.clone(),
                    on_click: move |_| on_close.call(close_id.clone()),
                }
            }
            text {
                width: "100%",
                content: item.metadata.clone(),
                font_size: typography::XS,
                font_weight: 400,
                font_color: palette.muted_foreground,
                line_height: 16.0,
                max_lines: 1,
                text_overflow: "ellipsis",
            }
            row {
                width: "100%",
                height: 16.0,
                align_items: "center",
                row {
                    layout_weight: 1.0,
                    height: 16.0,
                    align_items: "center",
                    text {
                        width: "100%",
                        content: item.traffic.clone(),
                        font_size: typography::XS,
                        font_weight: 500,
                        font_color: palette.foreground,
                        line_height: 16.0,
                        max_lines: 1,
                        text_overflow: "ellipsis",
                    }
                }
                text {
                    content: item.started_at.clone(),
                    font_size: typography::XS,
                    font_weight: 400,
                    font_color: palette.muted_foreground,
                    line_height: 16.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
            text { content: accessibility_text, width: 0.0, height: 0.0, opacity: 0.0 }
        }
    }
}

/// shadcn Badge (pill): height ~22, XS text, medium weight, status label only.
#[component]
fn VirtualStatusBadge(label: String, background: u32, foreground: u32) -> Element {
    rsx! {
        row {
            height: 22.0,
            margin_right: spacing::MD,
            margin_left: spacing::XXS,
            padding_right: 8.0,
            padding_left: 8.0,
            background_color: background,
            border_radius: 999.0,
            align_items: "center",
            justify_content: "center",
            text {
                content: label,
                font_size: typography::XS,
                font_color: foreground,
                font_weight: 500,
                max_lines: 1,
            }
        }
    }
}

/// shadcn Button size="icon" variant="ghost".
///
/// + and × use the *same* fixed text box (size / weight / line-height / align)
/// so a pair of trailing actions stays level and optically centered.
///
/// `margin_right` is the trailing gap after this action (space before the next
/// icon, matching Badge→+ spacing on request rows).
#[component]
fn VirtualIconAction(
    content: String,
    font_size: f32,
    foreground: u32,
    margin_right: f32,
    accessibility: String,
    on_click: EventHandler<()>,
) -> Element {
    rsx! {
        text {
            width: ACTIVITY_ACTION_SIZE,
            height: ACTIVITY_ACTION_SIZE,
            margin_right,
            background_color: 0x0000_0000,
            content,
            font_size,
            font_color: foreground,
            font_weight: 400,
            text_align: "center",
            line_height: ACTIVITY_ACTION_SIZE,
            max_lines: 1,
            border_radius: 6.0,
            onclick: move |_| on_click.call(()),
        }
        text { content: accessibility, width: 0.0, height: 0.0, opacity: 0.0 }
    }
}

fn format_activity_timestamp(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(formatted) = time_format::format_unix_seconds(trimmed) {
        return formatted.trim_end_matches(" UTC").to_owned();
    }
    // meow connection timestamps often arrive as ISO-8601 (`2026-07-23T13:27:11Z`).
    format_iso_activity_timestamp(trimmed).unwrap_or_else(|| trimmed.to_owned())
}

fn format_iso_activity_timestamp(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('Z');
    let (date, time) = value.split_once('T').or_else(|| value.split_once(' '))?;
    if date.len() < 10 || time.len() < 5 {
        return None;
    }
    // Keep `YYYY-MM-DD HH:MM` to match request-row density.
    Some(format!("{} {}", &date[..10], &time[..5]))
}

pub(crate) fn manual_rule_dialog(state: Signal<State>, current: &State) -> Element {
    let open = current.manual_rule_editor.is_some();
    // Refresh overlay body when fields change so arkit Select / inputs stay live.
    let content_key = current
        .manual_rule_editor
        .as_ref()
        .map(|editor| {
            dialog_content_key(&[
                &editor.value,
                &editor.target,
                &format!("{:?}", editor.match_kind),
                &editor.submitting.to_string(),
                editor.error.as_deref().unwrap_or(""),
                &editor.disconnect_after_save.to_string(),
            ])
        })
        .unwrap_or(0);
    rsx! {
        FlatDialog {
            open,
            content_key,
            on_close: move |_| dispatch(state, Action::CloseManualRuleEditor),
            ManualRuleDialogContent { state }
        }
    }
}

#[component]
fn ManualRuleDialogContent(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let Some(editor) = current.manual_rule_editor.clone() else {
        return rsx! {};
    };
    let locale = current.locale;
    let exact_label = translate_ui(locale, tr::page_tr_005());
    let suffix_label = translate_ui(locale, tr::page_tr_006());
    let ip_label = "IP/CIDR".to_owned();
    let selected_match = match editor.match_kind {
        ManualRuleMatchKind::Domain => exact_label.clone(),
        ManualRuleMatchKind::DomainSuffix => suffix_label.clone(),
        ManualRuleMatchKind::IpCidr => ip_label.clone(),
    };
    let suffix_option = suffix_label.clone();
    let ip_option = ip_label.clone();
    let mut targets = vec!["DIRECT".to_owned()];
    for group in &current.snapshot.proxy_groups {
        if !group.name.eq_ignore_ascii_case("GLOBAL")
            && !targets.iter().any(|target| target == &group.name)
        {
            targets.push(group.name.clone());
        }
    }
    let preview = manual_rule_preview(editor.match_kind, &editor.value, &editor.target);
    let conflict = find_manual_rule_conflict(
        &current.snapshot.rules,
        editor.match_kind,
        &editor.value,
        &editor.target,
    );
    let conflict_message = conflict.map(|conflict| {
        if conflict.same_target {
            tr(
                locale,
                "已有相同规则，保存时不会重复添加",
                "The same rule already exists and will not be duplicated",
            )
            .to_owned()
        } else if conflict.source == "profile-yaml" {
            format!(
                "{} {} → {}",
                tr(
                    locale,
                    "订阅中已有同条件规则，将创建高优先级覆盖：",
                    "The profile has the same selector; a higher-priority override will be created:",
                ),
                conflict.target,
                editor.target
            )
        } else {
            format!(
                "{} {} → {}",
                tr(
                    locale,
                    "已有手动规则，将直接更新策略：",
                    "The existing manual rule will be updated:",
                ),
                conflict.target,
                editor.target
            )
        }
    });
    let can_save = !editor.submitting
        && !editor.value.trim().is_empty()
        && !editor.target.trim().is_empty()
        && current.snapshot.active_profile.is_some();

    rsx! {
        DialogHeader {
            title: translate_ui(locale, tr::page_tr_004()),
            description: Some(translate_ui(locale, tr::page_tr_007())),
        }
            row { height: 14.0 }
            text { content: translate_ui(locale, tr::page_tr_008()), font_size: 11.0, font_weight: 650, font_color: subtle() }
            row { height: 6.0 }
            FlatSegmented {
                options: vec![exact_label, suffix_label, ip_label],
                selected: selected_match,
                on_change: move |value: String| {
                    let match_kind = if value == suffix_option {
                        ManualRuleMatchKind::DomainSuffix
                    } else if value == ip_option {
                        ManualRuleMatchKind::IpCidr
                    } else {
                        ManualRuleMatchKind::Domain
                    };
                    dispatch(state, Action::SetManualRuleMatchKind(match_kind));
                },
            }
            row { height: 10.0 }
            Input {
                value: Some(editor.value.clone()),
                placeholder: Some(match editor.match_kind {
                    ManualRuleMatchKind::Domain => translate_ui(locale, tr::page_tr_009()),
                    ManualRuleMatchKind::DomainSuffix => translate_ui(locale, tr::page_tr_010()),
                    ManualRuleMatchKind::IpCidr => translate_ui(locale, tr::page_tr_011()),
                }.to_owned()),
                width: Some("100%".into()),
                disabled: editor.submitting,
                on_change: move |value| dispatch(state, Action::SetManualRuleValue(value)),
            }
            row { height: 10.0 }
            text { content: translate_ui(locale, tr::page_tr_012()), font_size: 11.0, font_weight: 650, font_color: subtle() }
            row { height: 6.0 }
            // arkit shadcn Select (overlay panel). Ignore changes while submitting.
            column {
                width: "100%",
                opacity: if editor.submitting { 0.5 } else { 1.0 },
                Select {
                    options: targets,
                    selected: Some(editor.target.clone()),
                    default_selected: editor.target.clone(),
                    default_open: false,
                    on_select: move |value: String| {
                        if !state.read().manual_rule_editor.as_ref().is_some_and(|e| e.submitting) {
                            dispatch(state, Action::SetManualRuleTarget(value));
                        }
                    },
                }
            }
            column {
                width: "100%",
                margin_top: 10.0,
                padding: 9.0,
                border_width: 1.0,
                border_color: line(),
                border_radius: 7.0,
                background_color: muted(),
                text { content: translate_ui(locale, tr::page_tr_013()), font_size: 10.0, font_weight: 650, font_color: subtle() }
                text { content: preview, margin_top: 3.0, font_size: 11.0, font_color: text_color(), max_lines: 2, text_overflow: "ellipsis" }
            }
            if let Some(message) = conflict_message {
                text { content: message, margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
            }
            if current.snapshot.mode != RuntimeMode::Rule {
                text { content: translate_ui(locale, tr::page_tr_014()), margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
            }
            if editor.connection_id.is_some() {
                row {
                    width: "100%",
                    margin_top: 10.0,
                    align_items: "center",
                    Switch {
                        checked: Some(editor.disconnect_after_save),
                        on_change: move |value| dispatch(state, Action::SetManualRuleDisconnect(value)),
                    }
                    text { content: translate_ui(locale, tr::page_tr_015()), margin_left: 8.0, font_size: 11.0, line_height: 16.0, font_color: subtle() }
                }
            } else {
                text { content: translate_ui(locale, tr::page_tr_016()), margin_top: 8.0, font_size: 11.0, font_color: subtle() }
            }
            if let Some(error) = editor.error {
                text { content: error, margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: danger() }
            }
        DialogFooter {
            FlatButton {
                variant: FlatButtonVariant::Primary,
                width: "100%",
                disabled: Some(!can_save),
                onclick: move |_| dispatch(state, Action::SaveManualRule),
                if editor.submitting {
                    Spinner { size: 16.0, color: Some(primary_text()) }
                } else {
                    {arkit::icon("route", 16.0, primary_text())}
                }
                text { content: if editor.submitting { translate_ui(locale, tr::page_tr_017()) } else { translate_ui(locale, tr::page_tr_018()) }, margin_left: 8.0, font_size: 13.0, font_weight: 650, font_color: primary_text() }
            }
        }
    }
}
