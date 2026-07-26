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
    let all_label = strings(current.locale).requests_status_all.to_owned();
    let active_label = strings(current.locale).requests_status_active.to_owned();
    let ended_label = strings(current.locale).requests_status_ended.to_owned();
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
            rule_accessibility: tr(current.locale, "添加命中规则", "Add matching rule").to_owned(),
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
                placeholder: Some(strings(current.locale).requests_search_placeholder.to_owned()),
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
                    {empty_state("activity", strings(current.locale).requests_empty_title, strings(current.locale).requests_empty_subtitle)}
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
                close_accessibility: strings(current.locale).connections_close.to_owned(),
                rule_accessibility: tr(current.locale, "添加命中规则", "Add matching rule")
                    .to_owned(),
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
                placeholder: Some(strings(current.locale).connections_search_placeholder.to_owned()),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row { height: spacing::MD }
            row {
                layout_weight: 1.0,
                width: "100%",
                if empty {
                    {empty_state("unplug", strings(current.locale).connections_empty_title, strings(current.locale).connections_empty_subtitle)}
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

#[derive(Clone, Copy)]
enum VirtualActivityTextWidth {
    Intrinsic,
    FillRow,
    FullWidth,
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

#[derive(Clone)]
struct VirtualRequestRenderState {
    items: Vec<VirtualRequestRow>,
    palette: VirtualActivityPalette,
    on_open: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
}

#[derive(Clone)]
struct VirtualConnectionRenderState {
    items: Vec<VirtualConnectionRow>,
    palette: VirtualActivityPalette,
    on_close: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
}

#[component]
fn VirtualRequestList(
    items: Vec<VirtualRequestRow>,
    palette: VirtualActivityPalette,
    on_open: EventHandler<String>,
    on_add_rule: EventHandler<ManualRuleContext>,
) -> Element {
    let item_keys = activity_item_keys(&items, palette);
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualRequestRenderState {
            items: items.clone(),
            palette,
            on_open,
            on_add_rule,
        }))
    });
    *render_state.borrow_mut() = VirtualRequestRenderState {
        items,
        palette,
        on_open,
        on_add_rule,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_request_row(
            &state.items[index as usize],
            state.palette,
            render_state_for_adapter.clone(),
        )
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        list {
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
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualConnectionRenderState {
            items: items.clone(),
            palette,
            on_close,
            on_add_rule,
        }))
    });
    *render_state.borrow_mut() = VirtualConnectionRenderState {
        items,
        palette,
        on_close,
        on_add_rule,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_connection_row(
            &state.items[index as usize],
            state.palette,
            render_state_for_adapter.clone(),
        )
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        list {
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

fn render_virtual_request_row(
    item: &VirtualRequestRow,
    palette: VirtualActivityPalette,
    interaction_state: Rc<RefCell<VirtualRequestRenderState>>,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    let host = virtual_activity_text(
        item.host.clone(),
        typography::SM,
        6,
        palette.foreground,
        20.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let host = if item.active {
        let connection_query = item.connection_query.clone();
        let open_state = interaction_state.clone();
        // Navigation lives on the title, not inside the status badge.
        NodeBuilder::new("row")?
            .attr(ArkUINodeAttributeType::LayoutWeight, 1.0_f32)?
            .height(20.0)?
            .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
            .child(host)?
            .on_click(move || {
                let on_open = open_state.borrow().on_open;
                on_open.call(connection_query.clone());
            })?
            .build()
    } else {
        host
    };
    // shadcn Badge: status only — never embed navigation chevrons in the label.
    let status = virtual_status_badge(
        item.status.clone(),
        if item.active {
            // soft success chip (common extension of shadcn Badge)
            palette.success_soft
        } else {
            // Badge variant="secondary"
            palette.secondary
        },
        if item.active {
            palette.success
        } else {
            palette.muted_foreground
        },
    )?;
    let rule_context = ManualRuleContext {
        connection_id: item.active.then(|| item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    // shadcn Button size="icon" variant="ghost": no fill, thin stroke glyph.
    let add_rule = virtual_icon_action(
        VirtualIconGlyph::Plus,
        palette.muted_foreground,
        0x0000_0000,
        0.0,
        item.rule_accessibility.clone(),
        {
            let add_rule_state = interaction_state.clone();
            move || {
                let on_add_rule = add_rule_state.borrow().on_add_rule;
                on_add_rule.call(rule_context.clone());
            }
        },
    )?;
    let header = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(ACTIVITY_ACTION_SIZE)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(host)?
        .child(status)?
        .child(add_rule)?
        .build();
    let metadata = virtual_activity_text(
        item.metadata.clone(),
        typography::XS,
        4,
        palette.muted_foreground,
        16.0,
        VirtualActivityTextWidth::FullWidth,
    )?;
    let traffic = virtual_activity_text(
        item.traffic.clone(),
        typography::XS,
        5,
        palette.foreground,
        16.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let updated_at = virtual_activity_text(
        item.updated_at.clone(),
        typography::XS,
        4,
        palette.muted_foreground,
        16.0,
        VirtualActivityTextWidth::Intrinsic,
    )?;
    let footer = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(16.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(traffic)?
        .child(updated_at)?
        .build();
    Ok(NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(REQUEST_ROW_HEIGHT)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([12.0, 14.0, 12.0, 14.0])?
        .margin([0.0, 0.0, ACTIVITY_CARD_GAP, 0.0])?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![palette.radius; 4])?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(ArkUINodeAttributeType::ColumnJustifyContent, 2_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            format!(
                "{}，{}，{}，{}，{}",
                item.host, item.status, item.metadata, item.traffic, item.updated_at,
            ),
        )?
        .child(header)?
        .child(metadata)?
        .child(footer)?
        .build())
}

fn render_virtual_connection_row(
    item: &VirtualConnectionRow,
    palette: VirtualActivityPalette,
    interaction_state: Rc<RefCell<VirtualConnectionRenderState>>,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    // Mirror request-row layout:
    // [host                    ] [+] [×]
    // [metadata]
    // [traffic                     ] [time]
    let host = virtual_activity_text(
        item.host.clone(),
        typography::SM,
        6,
        palette.foreground,
        20.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let rule_context = ManualRuleContext {
        connection_id: Some(item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    let add_rule = virtual_icon_action(
        VirtualIconGlyph::Plus,
        palette.muted_foreground,
        0x0000_0000,
        // Gap before close matches Badge→+ spacing on the requests page.
        spacing::MD,
        item.rule_accessibility.clone(),
        {
            let add_rule_state = interaction_state.clone();
            move || {
                let on_add_rule = add_rule_state.borrow().on_add_rule;
                on_add_rule.call(rule_context.clone());
            }
        },
    )?;
    let close_id = item.id.clone();
    let close = virtual_icon_action(
        VirtualIconGlyph::Close,
        palette.danger,
        0x0000_0000,
        0.0,
        item.close_accessibility.clone(),
        {
            let close_state = interaction_state.clone();
            move || {
                let on_close = close_state.borrow().on_close;
                on_close.call(close_id.clone());
            }
        },
    )?;
    let header = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(ACTIVITY_ACTION_SIZE)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(host)?
        .child(add_rule)?
        .child(close)?
        .build();
    let metadata = virtual_activity_text(
        item.metadata.clone(),
        typography::XS,
        4,
        palette.muted_foreground,
        16.0,
        VirtualActivityTextWidth::FullWidth,
    )?;
    let traffic = virtual_activity_text(
        item.traffic.clone(),
        typography::XS,
        5,
        palette.foreground,
        16.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let started_at = virtual_activity_text(
        item.started_at.clone(),
        typography::XS,
        4,
        palette.muted_foreground,
        16.0,
        VirtualActivityTextWidth::Intrinsic,
    )?;
    let footer = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(16.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(traffic)?
        .child(started_at)?
        .build();
    Ok(NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(CONNECTION_ROW_HEIGHT)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([12.0, 14.0, 12.0, 14.0])?
        .margin([0.0, 0.0, ACTIVITY_CARD_GAP, 0.0])?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![palette.radius; 4])?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(ArkUINodeAttributeType::ColumnJustifyContent, 2_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            format!(
                "{}，{}，{}，{}",
                item.host, item.metadata, item.traffic, item.started_at,
            ),
        )?
        .child(header)?
        .child(metadata)?
        .child(footer)?
        .build())
}

/// shadcn Badge (pill): height ~22, XS text, medium weight, status label only.
fn virtual_status_badge(
    label: String,
    background: u32,
    foreground: u32,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    let text = NodeBuilder::new("text")?
        .font_size(typography::XS)?
        .font_color(format!("#{foreground:08x}"))?
        .text_content(label)?
        .attr(ArkUINodeAttributeType::FontWeight, 5_i32)?
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?
        .build();
    Ok(NodeBuilder::new("row")?
        .height(22.0)?
        // [top, right, bottom, left] — right gap keeps Badge away from the + action.
        .margin([0.0, spacing::MD, 0.0, spacing::XXS])?
        .padding([0.0, 8.0, 0.0, 8.0])?
        .background_color(format!("#{background:08x}"))?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![999.0; 4])?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .attr(ArkUINodeAttributeType::RowJustifyContent, 1_i32)?
        .child(text)?
        .build())
}

#[derive(Clone, Copy)]
enum VirtualIconGlyph {
    Plus,
    Close,
}

/// shadcn Button size="icon" variant="ghost".
///
/// + and × use the *same* fixed text box (size / weight / line-height / align)
/// so a pair of trailing actions stays level and optically centered.
///
/// `margin_right` is the trailing gap after this action (space before the next
/// icon, matching Badge→+ spacing on request rows).
fn virtual_icon_action(
    glyph: VirtualIconGlyph,
    foreground: u32,
    background: u32,
    margin_right: f32,
    accessibility: String,
    on_click: impl Fn() + 'static,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    // Same hit box for every action. Slightly smaller close glyph so the wider
    // "×" glyph matches the optical weight/center of "+".
    let (content, font_size) = match glyph {
        VirtualIconGlyph::Plus => ("+", 18.0_f32),
        VirtualIconGlyph::Close => ("×", 16.0_f32),
    };
    Ok(NodeBuilder::new("text")?
        .width(ACTIVITY_ACTION_SIZE)?
        .height(ACTIVITY_ACTION_SIZE)?
        .margin([0.0, margin_right, 0.0, 0.0])?
        .background_color(format!("#{background:08x}"))?
        .font_size(font_size)?
        .font_color(format!("#{foreground:08x}"))?
        .text_content(content.to_owned())?
        .attr(ArkUINodeAttributeType::FontWeight, 4_i32)?
        .attr(ArkUINodeAttributeType::TextAlign, 1_i32)? // center
        .attr(ArkUINodeAttributeType::TextLineHeight, ACTIVITY_ACTION_SIZE)?
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![6.0; 4])?
        .attr(ArkUINodeAttributeType::AccessibilityText, accessibility)?
        .on_click(on_click)?
        .build())
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
    let exact_label = tr(locale, "精确域名", "Exact domain").to_owned();
    let suffix_label = tr(locale, "域名后缀", "Domain suffix").to_owned();
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
        if !targets.iter().any(|target| target == &group.name) {
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
            title: tr(locale, "添加命中规则", "Add matching rule").to_owned(),
            description: Some(tr(locale, "保存到当前配置并热更新 meow 路由", "Save to the active profile and update meow routing live").to_owned()),
        }
            row { height: 14.0 }
            text { content: tr(locale, "匹配方式", "Match type"), font_size: 11.0, font_weight: 650, font_color: subtle() }
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
                    ManualRuleMatchKind::Domain => tr(locale, "例如 api.example.com", "For example api.example.com"),
                    ManualRuleMatchKind::DomainSuffix => tr(locale, "例如 example.com", "For example example.com"),
                    ManualRuleMatchKind::IpCidr => tr(locale, "例如 192.0.2.1 或 192.0.2.0/24", "For example 192.0.2.1 or 192.0.2.0/24"),
                }.to_owned()),
                width: Some("100%".into()),
                disabled: editor.submitting,
                on_change: move |value| dispatch(state, Action::SetManualRuleValue(value)),
            }
            row { height: 10.0 }
            text { content: tr(locale, "路由策略", "Routing policy"), font_size: 11.0, font_weight: 650, font_color: subtle() }
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
                text { content: tr(locale, "规则预览", "Rule preview"), font_size: 10.0, font_weight: 650, font_color: subtle() }
                text { content: preview, margin_top: 3.0, font_size: 11.0, font_color: text_color(), max_lines: 2, text_overflow: "ellipsis" }
            }
            if let Some(message) = conflict_message {
                text { content: message, margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
            }
            if current.snapshot.mode != RuntimeMode::Rule {
                text { content: tr(locale, "当前不是规则模式：规则会保存，但切换到规则模式后才会命中。", "Rule mode is not active: the rule will be saved, but matching starts after switching to Rule mode."), margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
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
                    text { content: tr(locale, "保存后断开当前连接，使重连立即使用新规则", "Close the current connection after saving so its reconnect uses the new rule"), margin_left: 8.0, font_size: 11.0, line_height: 16.0, font_color: subtle() }
                }
            } else {
                text { content: tr(locale, "规则只影响之后建立的新连接。", "The rule applies to newly established connections."), margin_top: 8.0, font_size: 11.0, font_color: subtle() }
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
                text { content: if editor.submitting { tr(locale, "正在保存", "Saving") } else { tr(locale, "保存并应用", "Save and apply") }, margin_left: 8.0, font_size: 13.0, font_weight: 650, font_color: primary_text() }
            }
        }
    }
}

fn virtual_activity_text(
    content: String,
    size: f32,
    weight: i32,
    color: u32,
    line_height: f32,
    width: VirtualActivityTextWidth,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    let node = NodeBuilder::new("text")?
        .font_size(size)?
        .font_color(format!("#{color:08x}"))?
        .text_content(content)?
        .attr(ArkUINodeAttributeType::FontWeight, weight)?
        .attr(ArkUINodeAttributeType::TextLineHeight, line_height)?
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?
        .attr(ArkUINodeAttributeType::TextOverflow, 2_i32)?;
    let node = match width {
        VirtualActivityTextWidth::Intrinsic => node,
        VirtualActivityTextWidth::FillRow => {
            node.attr(ArkUINodeAttributeType::LayoutWeight, 1.0_f32)?
        }
        VirtualActivityTextWidth::FullWidth => node.percent_width(1.0)?,
    };
    Ok(node.build())
}
