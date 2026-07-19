use super::super::*;

pub(crate) fn requests_page(state: Signal<State>) -> Element {
    let mut request_query = use_signal(String::new);
    let mut request_filter = use_signal(|| RequestStatusFilter::All);
    let current = state.read().clone();
    let navigator = use_navigator();
    let query_value = request_query();
    let filter_value = request_filter();
    let rows = current
        .snapshot
        .request_history
        .iter()
        .filter(|request| matches_request_filter(request, filter_value, &query_value))
        .cloned()
        .map(|request| {
            let query = request_connection_query(&request);
            rsx! {
                {card(
                    truncate_text(&request.host, 42),
                    Some(format!("{} · {}", request.network, request.proxy)),
                    rsx! {
                        column {
                            percent_width: 1.0,
                            {info_row(tr(current.locale, "规则", "Rule"), request.rule.clone())}
                            {info_row(tr(current.locale, "流量", "Traffic"), format!("↓ {}  ↑ {}", format_total(request.download_bytes), format_total(request.upload_bytes)))}
                            {info_row(tr(current.locale, "更新时间", "Updated"), request.updated_at.clone())}
                            if request.active {
                                row { height: 12.0 }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    percent_width: Some(1.0),
                                    onclick: move |_| {
                                        navigator.push(Route::Connections { query: query.clone() });
                                    },
                                    {arkit::icon("arrow-right", 16.0, text_color())}
                                    text {
                                        content: tr(current.locale, "查看连接", "View connection"),
                                        margin_left: 8.0,
                                        font_size: 14.0,
                                        font_weight: 600,
                                        font_color: text_color(),
                                    }
                                }
                            }
                        }
                    }
                )}
            }
        })
        .collect::<Vec<_>>();
    let all_label = strings(current.locale).requests_status_all.to_owned();
    let active_label = strings(current.locale).requests_status_active.to_owned();
    let ended_label = strings(current.locale).requests_status_ended.to_owned();
    let filter_options = vec![all_label.clone(), active_label.clone(), ended_label.clone()];
    let selected_filter = match filter_value {
        RequestStatusFilter::All => all_label.clone(),
        RequestStatusFilter::Active => active_label.clone(),
        RequestStatusFilter::Ended => ended_label.clone(),
    };
    let empty = rows.is_empty();
    let body = rsx! {
        column {
            percent_width: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).requests_search_placeholder.to_owned()),
                percent_width: Some(1.0),
                on_change: move |value| request_query.set(value),
            }
            row { height: 12.0 }
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
            row { height: 14.0 }
            if empty {
                {empty_state("activity", strings(current.locale).requests_empty_title, strings(current.locale).requests_empty_subtitle)}
            } else {
                {spaced(rows)}
            }
        }
    };
    scaffold(
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
        .cloned()
        .map(|connection| compact_connection_card(state, current.locale, connection))
        .collect::<Vec<_>>();
    let empty = rows.is_empty();
    let body = rsx! {
        column {
            percent_width: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).connections_search_placeholder.to_owned()),
                percent_width: Some(1.0),
                on_change: move |value| query.set(value),
            }
            row { height: 12.0 }
            if empty {
                {empty_state("unplug", strings(current.locale).connections_empty_title, strings(current.locale).connections_empty_subtitle)}
            } else {
                {spaced(rows)}
            }
        }
    };
    scaffold(
        state,
        Route::Connections {
            query: String::new(),
        },
        destructive_icon_action("circle-x", Action::CloseAllConnections, state),
        body,
    )
}

fn compact_connection_card(
    state: Signal<State>,
    locale: UiLocale,
    connection: hmeta_model::ConnectionSummary,
) -> Element {
    let id = connection.id.clone();
    let chain = if connection.chains.is_empty() {
        "DIRECT".to_owned()
    } else {
        connection.chains.join(" > ")
    };
    let routing = format!(
        "{} · {}",
        truncate_text(&connection.rule, 36),
        truncate_text(&chain, 34)
    );
    let traffic = format!(
        "↓ {}   ↑ {}",
        format_total(connection.download_bytes),
        format_total(connection.upload_bytes)
    );
    rsx! {
        column {
            percent_width: 1.0,
            padding_top: 11.0,
            padding_bottom: 11.0,
            padding_left: 13.0,
            padding_right: 10.0,
            background_color: surface(),
            border_width: 1.0,
            border_color: line(),
            border_radius: 10.0,
            clip: true,
            row {
                percent_width: 1.0,
                height: 40.0,
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text {
                        content: truncate_text(&connection.host, 52),
                        percent_width: 1.0,
                        font_size: 13.0,
                        font_weight: 700,
                        font_color: text_color(),
                        max_lines: 1,
                    }
                    text {
                        content: format!("{} · {}", connection.network.to_ascii_uppercase(), truncate_text(&connection.proxy, 30)),
                        percent_width: 1.0,
                        margin_top: 3.0,
                        font_size: 10.0,
                        font_color: subtle(),
                        max_lines: 1,
                    }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Icon,
                    onclick: move |_| dispatch(state, Action::CloseConnection(id.clone())),
                    {arkit::icon("unplug", 16.0, danger())}
                }
            }
            row {
                percent_width: 1.0,
                height: 25.0,
                padding_right: 4.0,
                align_items: "center",
                row {
                    layout_weight: 1.0,
                    text { content: routing, percent_width: 1.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
                }
            }
            row {
                percent_width: 1.0,
                height: 22.0,
                padding_right: 4.0,
                align_items: "center",
                text { content: traffic, font_size: 11.0, font_weight: 650, font_color: text_color(), max_lines: 1 }
                row { layout_weight: 1.0 }
                text {
                    content: format!("{} {}", tr(locale, "开始", "Started"), truncate_text(&connection.started_at, 18)),
                    font_size: 10.0,
                    font_color: subtle(),
                    max_lines: 1,
                }
            }
        }
    }
}
