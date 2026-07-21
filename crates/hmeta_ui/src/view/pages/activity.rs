use super::super::*;

const REQUEST_ROW_HEIGHT: f32 = 72.0;
const CONNECTION_ROW_HEIGHT: f32 = 72.0;

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
                "↓ {}   ↑ {}",
                format_total(request.download_bytes),
                format_total(request.upload_bytes),
            ),
            updated_at: request.updated_at.clone(),
            status: if request.active {
                format!("{}  ›", active_label)
            } else {
                ended_label.clone()
            },
            active: request.active,
            connection_query: request_connection_query(request),
            rule_accessibility: tr(current.locale, "添加命中规则", "Add matching rule").to_owned(),
        })
        .collect::<Vec<_>>();
    let empty = rows.is_empty();
    let palette = VirtualActivityPalette {
        surface: surface(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
        danger: danger(),
    };
    let body = rsx! {
        column {
            percent_width: 1.0,
            percent_height: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).requests_search_placeholder.to_owned()),
                percent_width: Some(1.0),
                on_change: move |value| request_query.set(value),
            }
            row { height: 10.0 }
            row {
                percent_width: 1.0,
                justify_content: "center",
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
            }
            row { height: 10.0 }
            row {
                layout_weight: 1.0,
                percent_width: 1.0,
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
                metadata: format!(
                    "{} · {}",
                    connection.network.to_ascii_uppercase(),
                    truncate_text(&connection.proxy, 32),
                ),
                routing: format!(
                    "{} · {}",
                    truncate_text(&connection.rule, 32),
                    truncate_text(&chain, 30),
                ),
                traffic: format!(
                    "↓ {}   ↑ {}",
                    format_total(connection.download_bytes),
                    format_total(connection.upload_bytes),
                ),
                started_at: format!(
                    "{} {}",
                    tr(current.locale, "开始", "Started"),
                    truncate_text(&connection.started_at, 18),
                ),
                close_accessibility: strings(current.locale).connections_close.to_owned(),
                rule_accessibility: tr(current.locale, "添加命中规则", "Add matching rule")
                    .to_owned(),
            }
        })
        .collect::<Vec<_>>();
    let empty = rows.is_empty();
    let palette = VirtualActivityPalette {
        surface: surface(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
        danger: danger(),
    };
    let body = rsx! {
        column {
            percent_width: 1.0,
            percent_height: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).connections_search_placeholder.to_owned()),
                percent_width: Some(1.0),
                on_change: move |value| query.set(value),
            }
            row { height: 10.0 }
            row {
                layout_weight: 1.0,
                percent_width: 1.0,
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
    routing: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VirtualActivityPalette {
    surface: u32,
    foreground: u32,
    muted_foreground: u32,
    border: u32,
    success: u32,
    danger: u32,
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
            percent_width: 1.0,
            percent_height: 1.0,
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
            percent_width: 1.0,
            percent_height: 1.0,
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
        13.0,
        6,
        palette.foreground,
        18.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let status_builder = NodeBuilder::new("text")?
        .font_size(10.0)?
        .font_color(format!(
            "#{:08x}",
            if item.active {
                palette.success
            } else {
                palette.muted_foreground
            }
        ))?
        .text_content(item.status.clone())?
        .margin([0.0, 0.0, 0.0, 8.0])?
        .attr(ArkUINodeAttributeType::FontWeight, 5_i32)?
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?;
    let status = if item.active {
        let connection_query = item.connection_query.clone();
        let open_state = interaction_state.clone();
        status_builder
            .on_click(move || {
                let on_open = open_state.borrow().on_open;
                on_open.call(connection_query.clone());
            })?
            .build()
    } else {
        status_builder.build()
    };
    let rule_context = ManualRuleContext {
        connection_id: item.active.then(|| item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    let rule_accessibility = item.rule_accessibility.clone();
    let add_rule_state = interaction_state.clone();
    let add_rule = NodeBuilder::new("text")?
        .width(28.0)?
        .height(24.0)?
        .font_size(14.0)?
        .font_color(format!("#{:08x}", palette.foreground))?
        .text_content("＋")?
        .margin([0.0, 0.0, 0.0, 6.0])?
        .attr(ArkUINodeAttributeType::TextAlign, 1_i32)?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![6.0; 4])?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            rule_accessibility,
        )?
        .on_click(move || {
            let on_add_rule = add_rule_state.borrow().on_add_rule;
            on_add_rule.call(rule_context.clone());
        })?
        .build();
    let header = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(18.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(host)?
        .child(status)?
        .child(add_rule)?
        .build();
    let metadata = virtual_activity_text(
        item.metadata.clone(),
        10.0,
        3,
        palette.muted_foreground,
        15.0,
        VirtualActivityTextWidth::FullWidth,
    )?;
    let traffic = virtual_activity_text(
        item.traffic.clone(),
        10.5,
        5,
        palette.foreground,
        15.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let updated_at = virtual_activity_text(
        item.updated_at.clone(),
        9.5,
        3,
        palette.muted_foreground,
        15.0,
        VirtualActivityTextWidth::Intrinsic,
    )?;
    let footer = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(15.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(traffic)?
        .child(updated_at)?
        .build();
    Ok(NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(REQUEST_ROW_HEIGHT)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([7.0, 10.0, 7.0, 10.0])?
        .margin([0.0, 0.0, 5.0, 0.0])?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![8.0; 4])?
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
    let host = virtual_activity_text(
        item.host.clone(),
        13.0,
        6,
        palette.foreground,
        18.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let rule_context = ManualRuleContext {
        connection_id: Some(item.id.clone()),
        domain: item.domain.clone(),
        destination_ip: item.destination_ip.clone(),
    };
    let rule_accessibility = item.rule_accessibility.clone();
    let add_rule_state = interaction_state.clone();
    let add_rule = NodeBuilder::new("text")?
        .width(30.0)?
        .height(30.0)?
        .font_size(14.0)?
        .font_color(format!("#{:08x}", palette.foreground))?
        .text_content("＋")?
        .margin([0.0, 0.0, 0.0, 6.0])?
        .attr(ArkUINodeAttributeType::TextAlign, 1_i32)?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![7.0; 4])?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            rule_accessibility,
        )?
        .on_click(move || {
            let on_add_rule = add_rule_state.borrow().on_add_rule;
            on_add_rule.call(rule_context.clone());
        })?
        .build();
    let close_id = item.id.clone();
    let close = NodeBuilder::new("text")?
        .width(30.0)?
        .height(30.0)?
        .font_size(18.0)?
        .font_color(format!("#{:08x}", palette.danger))?
        .text_content("×")?
        .margin([0.0, 0.0, 0.0, 8.0])?
        .attr(ArkUINodeAttributeType::TextAlign, 1_i32)?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![7.0; 4])?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            item.close_accessibility.clone(),
        )?
        .on_click(move || {
            let on_close = interaction_state.borrow().on_close;
            on_close.call(close_id.clone());
        })?
        .build();
    let header = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(30.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(host)?
        .child(add_rule)?
        .child(close)?
        .build();
    let metadata = virtual_activity_text(
        item.metadata.clone(),
        10.0,
        3,
        palette.muted_foreground,
        14.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let traffic = virtual_activity_text(
        item.traffic.clone(),
        10.0,
        5,
        palette.foreground,
        14.0,
        VirtualActivityTextWidth::Intrinsic,
    )?;
    let detail = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(14.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(metadata)?
        .child(traffic)?
        .build();
    let routing = virtual_activity_text(
        item.routing.clone(),
        9.5,
        3,
        palette.muted_foreground,
        14.0,
        VirtualActivityTextWidth::FillRow,
    )?;
    let started_at = virtual_activity_text(
        item.started_at.clone(),
        9.5,
        3,
        palette.muted_foreground,
        14.0,
        VirtualActivityTextWidth::Intrinsic,
    )?;
    let footer = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(14.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(routing)?
        .child(started_at)?
        .build();
    Ok(NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(CONNECTION_ROW_HEIGHT)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([5.0, 9.0, 5.0, 10.0])?
        .margin([0.0, 0.0, 5.0, 0.0])?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![8.0; 4])?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(ArkUINodeAttributeType::ColumnJustifyContent, 2_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            format!(
                "{}，{}，{}，{}，{}",
                item.host, item.metadata, item.routing, item.traffic, item.started_at,
            ),
        )?
        .child(header)?
        .child(detail)?
        .child(footer)?
        .build())
}

#[derive(Props, Clone, PartialEq)]
struct ManualRuleTargetSelectProps {
    options: Vec<String>,
    selected: String,
    disabled: bool,
    state: Signal<State>,
}

#[derive(Props, Clone, PartialEq)]
struct ManualRuleTargetOptionProps {
    option: String,
    selected: bool,
    open: Signal<bool>,
    state: Signal<State>,
}

#[component]
fn ManualRuleTargetOption(props: ManualRuleTargetOptionProps) -> Element {
    let value = props.option.clone();
    let state = props.state;
    let mut open = props.open;
    rsx! {
        button {
            percent_width: 1.0,
            height: 36.0,
            padding_left: 12.0,
            padding_right: 12.0,
            background_color: if props.selected { muted() } else { surface() },
            border_width: 0.0,
            border_radius: 0.0,
            onclick: move |_| {
                open.set(false);
                dispatch(state, Action::SetManualRuleTarget(value.clone()));
            },
            row {
                percent_width: 1.0,
                align_items: "center",
                row {
                    layout_weight: 1.0,
                    clip: true,
                    text { content: props.option.clone(), percent_width: 1.0, font_size: 13.0, font_weight: if props.selected { 650 } else { 500 }, font_color: text_color(), max_lines: 1, text_overflow: 2 }
                }
                if props.selected {
                    {arkit::icon("check", 15.0, text_color())}
                }
            }
        }
    }
}

#[component]
fn ManualRuleTargetSelect(props: ManualRuleTargetSelectProps) -> Element {
    let mut open = use_signal(|| false);
    let list_height = (props.options.len().min(5) as f32 * 36.0).max(36.0);

    rsx! {
        column {
            percent_width: 1.0,
            button {
                percent_width: 1.0,
                height: 40.0,
                padding_left: 12.0,
                padding_right: 12.0,
                background_color: surface(),
                border_width: 1.0,
                border_color: line(),
                border_radius: 8.0,
                onclick: move |_| {
                    if !props.disabled {
                        open.set(!open());
                    }
                },
                row {
                    percent_width: 1.0,
                    align_items: "center",
                    row {
                        layout_weight: 1.0,
                        clip: true,
                        text { content: props.selected.clone(), percent_width: 1.0, font_size: 13.0, font_color: if props.disabled { subtle() } else { text_color() }, max_lines: 1, text_overflow: 2 }
                    }
                    {arkit::icon(if open() { "chevron-up" } else { "chevron-down" }, 16.0, subtle())}
                }
            }
            if open() {
                scroll {
                    percent_width: 1.0,
                    height: list_height,
                    margin_top: 4.0,
                    scroll_enabled: props.options.len() > 5,
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 8.0,
                    clip: true,
                    column {
                        percent_width: 1.0,
                        for option in props.options.iter() {
                            ManualRuleTargetOption {
                                key: "{option}",
                                option: option.clone(),
                                selected: option == &props.selected,
                                open,
                                state: props.state,
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn manual_rule_dialog(state: Signal<State>, current: &State) -> Element {
    let open = current.manual_rule_editor.is_some();
    rsx! {
        FlatDialog {
            open,
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
                percent_width: Some(1.0),
                disabled: editor.submitting,
                on_change: move |value| dispatch(state, Action::SetManualRuleValue(value)),
            }
            row { height: 10.0 }
            text { content: tr(locale, "路由策略", "Routing policy"), font_size: 11.0, font_weight: 650, font_color: subtle() }
            row { height: 6.0 }
            ManualRuleTargetSelect {
                options: targets,
                selected: editor.target.clone(),
                disabled: editor.submitting,
                state,
            }
            column {
                percent_width: 1.0,
                margin_top: 10.0,
                padding: 9.0,
                border_width: 1.0,
                border_color: line(),
                border_radius: 7.0,
                background_color: muted(),
                text { content: tr(locale, "规则预览", "Rule preview"), font_size: 10.0, font_weight: 650, font_color: subtle() }
                text { content: preview, margin_top: 3.0, font_size: 11.0, font_color: text_color(), max_lines: 2, text_overflow: 2 }
            }
            if let Some(message) = conflict_message {
                text { content: message, margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
            }
            if current.snapshot.mode != RuntimeMode::Rule {
                text { content: tr(locale, "当前不是规则模式：规则会保存，但切换到规则模式后才会命中。", "Rule mode is not active: the rule will be saved, but matching starts after switching to Rule mode."), margin_top: 8.0, font_size: 11.0, line_height: 16.0, font_color: warning() }
            }
            if editor.connection_id.is_some() {
                row {
                    percent_width: 1.0,
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
                percent_width: 1.0,
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
