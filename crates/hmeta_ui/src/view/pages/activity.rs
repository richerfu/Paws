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
                    }
                }
            }
        }
    };
    fixed_scaffold(
        state,
        Route::Requests {},
        destructive_icon_action("trash-2", Action::ClearRequestHistory, state),
        body,
    )
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
                    }
                }
            }
        }
    };
    fixed_scaffold(
        state,
        Route::Connections {
            query: String::new(),
        },
        destructive_icon_action("circle-x", Action::CloseAllConnections, state),
        body,
    )
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualRequestRow {
    id: String,
    host: String,
    metadata: String,
    traffic: String,
    updated_at: String,
    status: String,
    active: bool,
    connection_query: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualConnectionRow {
    id: String,
    host: String,
    metadata: String,
    routing: String,
    traffic: String,
    started_at: String,
    close_accessibility: String,
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
}

#[derive(Clone)]
struct VirtualConnectionRenderState {
    items: Vec<VirtualConnectionRow>,
    palette: VirtualActivityPalette,
    on_close: EventHandler<String>,
}

#[component]
fn VirtualRequestList(
    items: Vec<VirtualRequestRow>,
    palette: VirtualActivityPalette,
    on_open: EventHandler<String>,
) -> Element {
    let item_keys = activity_item_keys(&items, palette);
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualRequestRenderState {
            items: items.clone(),
            palette,
            on_open,
        }))
    });
    *render_state.borrow_mut() = VirtualRequestRenderState {
        items,
        palette,
        on_open,
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
) -> Element {
    let item_keys = activity_item_keys(&items, palette);
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualConnectionRenderState {
            items: items.clone(),
            palette,
            on_close,
        }))
    });
    *render_state.borrow_mut() = VirtualConnectionRenderState {
        items,
        palette,
        on_close,
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
    let status = NodeBuilder::new("text")?
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
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?
        .build();
    let header = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(18.0)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .child(host)?
        .child(status)?
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
    let node = NodeBuilder::new("column")?
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
        .child(footer)?;
    if !item.active {
        return Ok(node.build());
    }

    let connection_query = item.connection_query.clone();
    Ok(node
        .on_click(move || {
            let on_open = interaction_state.borrow().on_open;
            on_open.call(connection_query.clone());
        })?
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
