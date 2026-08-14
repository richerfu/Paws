use super::super::*;

pub(crate) fn traffic_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let snapshot = current.snapshot;
    let navigator = use_navigator();
    let history = snapshot
        .traffic_history
        .iter()
        .map(|point| (point.download_speed, point.upload_speed))
        .collect::<Vec<_>>();
    let summary = summarize_traffic_history(&history);
    let samples = summary.map(|value| value.samples).unwrap_or(0);
    let peak_download = summary.map(|value| value.peak_download).unwrap_or(0);
    let peak_upload = summary.map(|value| value.peak_upload).unwrap_or(0);
    let active_profile = snapshot.profiles.iter().find(|profile| profile.active);
    let profile_upload = active_profile
        .map(|profile| profile.upload_bytes)
        .unwrap_or(0);
    let profile_download = active_profile
        .map(|profile| profile.download_bytes)
        .unwrap_or(0);
    let connected = snapshot.vpn_running;
    let connection_upload = snapshot
        .connections
        .iter()
        .map(|connection| connection.upload_bytes)
        .sum::<u64>();
    let connection_download = snapshot
        .connections
        .iter()
        .map(|connection| connection.download_bytes)
        .sum::<u64>();
    let active_connection_count = snapshot.connections.len();
    let connection_rows = snapshot.connections.iter().take(5).map(|connection| {
        rsx! {
            {info_row(
                truncate_text(&connection.host, 28),
                format!("↓ {} · ↑ {}", format_total(connection.download_bytes), format_total(connection.upload_bytes)),
            )}
        }
    }).collect::<Vec<_>>();
    let connections_navigator = navigator.clone();
    let dns_upstreams = if snapshot.dns.upstreams.is_empty() {
        "—".to_owned()
    } else {
        truncate_text(&snapshot.dns.upstreams.join(", "), 52)
    };
    let dns_fallbacks = if snapshot.dns.fallbacks.is_empty() {
        "—".to_owned()
    } else {
        truncate_text(&snapshot.dns.fallbacks.join(", "), 52)
    };
    let dns_tun_addresses = if snapshot.dns.tun_addresses.is_empty() {
        "—".to_owned()
    } else {
        snapshot.dns.tun_addresses.join(", ")
    };
    let recent_dns = snapshot.dns.recent_queries.iter().map(|query| {
        rsx! { {info_row(format!("{} {}", query.record_type, query.name), query.count.to_string())} }
    }).collect::<Vec<_>>();
    let diagnostic_pending = current.controller_diagnostic_pending.is_some();
    let memory_in_use = format_total(snapshot.controller_diagnostics.memory_in_use_bytes);
    let memory_limit = if snapshot.controller_diagnostics.memory_limit_bytes > 0 {
        format_total(snapshot.controller_diagnostics.memory_limit_bytes)
    } else {
        "—".to_owned()
    };
    let last_config_sync = snapshot
        .controller_diagnostics
        .last_config_sync_at
        .as_deref()
        .and_then(time_format::format_unix_seconds)
        .unwrap_or_else(|| translate_ui(current.locale, tr::page_tr_124()));
    let body = rsx! {
        column {
            width: "100%",
            align_items: "start",
            row {
                width: "100%",
                align_items: "center",
                row {
                    width: 8.0,
                    height: 8.0,
                    border_radius: 4.0,
                    background_color: if connected { success() } else { subtle() },
                }
                text {
                    content: if connected {
                        translate_ui(current.locale, tr::page_tr_125())
                    } else {
                        translate_ui(current.locale, tr::page_tr_126())
                    },
                    margin_left: 8.0,
                    font_size: typography::SM,
                    font_weight: 650,
                    font_color: if connected { success() } else { subtle() },
                }
            }
            row { height: 18.0 }
            text { content: translate_ui(current.locale, tr::page_tr_129()), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            row {
                width: "100%",
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        translate_ui(current.locale, tr::page_tr_130()),
                        profile_upload,
                        profile_download,
                    )}
                }
                row { width: 10.0 }
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        translate_ui(current.locale, tr::page_tr_131()),
                        snapshot.traffic.upload_bytes,
                        snapshot.traffic.download_bytes,
                    )}
                }
            }
            row { height: 18.0 }
            text { content: translate_ui(current.locale, tr::page_tr_132()), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            {traffic_metrics(
                translate_ui(current.locale, tr::traffic_download()),
                format_speed(snapshot.traffic.download_speed),
                translate_ui(current.locale, tr::traffic_upload()),
                format_speed(snapshot.traffic.upload_speed),
            )}
            row { height: 18.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_133()),
                Some(format!("{} {}", samples, translate_ui(current.locale, tr::traffic_sample_unit()))),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(current.locale, tr::traffic_peak_download()), format_speed(peak_download))}
                        {info_row(translate_ui(current.locale, tr::traffic_peak_upload()), format_speed(peak_upload))}
                        if peak_download == 0 && peak_upload == 0 {
                            column {
                                width: "100%",
                                height: 72.0,
                                margin_top: 10.0,
                                padding: 12.0,
                                align_items: "center",
                                justify_content: "center",
                                background_color: muted(),
                                border_radius: 8.0,
                                text {
                                    content: translate_ui(current.locale, tr::page_tr_274()),
                                    font_size: typography::XS,
                                    font_weight: 600,
                                    font_color: text_color(),
                                }
                                text {
                                    content: translate_ui(current.locale, tr::page_tr_275()),
                                    margin_top: 4.0,
                                    font_size: typography::XS,
                                    font_color: subtle(),
                                    text_align: "center",
                                }
                            }
                        } else {
                            {speed_bars(&snapshot.traffic_history)}
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_134()),
                Some(format!("{} {}", active_connection_count, translate_ui(current.locale, tr::page_tr_135()))),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(current.locale, tr::page_tr_136()), format_total(connection_download))}
                        {info_row(translate_ui(current.locale, tr::page_tr_137()), format_total(connection_upload))}
                        if !connection_rows.is_empty() {
                            Separator {}
                            {connection_rows.into_iter()}
                        }
                        row { height: 6.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            width: Some("100%".into()),
                            onclick: move |_| {
                                connections_navigator.push(Route::Connections { query: String::new() });
                            },
                            text { content: translate_ui(current.locale, tr::page_tr_138()), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            {arkit::icon("chevron-right", 14.0, subtle())}
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::traffic_dns_title()),
                Some(snapshot.dns.model.clone()),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(current.locale, tr::page_tr_139()), if snapshot.dns.hijacking { translate_ui(current.locale, tr::page_tr_140()) } else { translate_ui(current.locale, tr::page_tr_141()) })}
                        {info_row(translate_ui(current.locale, tr::page_tr_142()), snapshot.dns.listen.clone())}
                        {info_row(translate_ui(current.locale, tr::page_tr_143()), dns_tun_addresses)}
                        {info_row(translate_ui(current.locale, tr::page_tr_144()), dns_upstreams)}
                        {info_row(translate_ui(current.locale, tr::page_tr_145()), dns_fallbacks)}
                        {info_row(translate_ui(current.locale, tr::page_tr_146()), snapshot.dns.nameserver_policy.len().to_string())}
                        {info_row(translate_ui(current.locale, tr::traffic_dns_handled()), snapshot.dns.handled_packets.to_string())}
                        {info_row(translate_ui(current.locale, tr::dns_cache_hits()), snapshot.dns.cache_hits.to_string())}
                        {info_row(translate_ui(current.locale, tr::dns_cache_misses()), snapshot.dns.cache_misses.to_string())}
                        if !recent_dns.is_empty() {
                            row { height: 8.0 }
                            {recent_dns.into_iter()}
                        }
                        row { height: 8.0 }
                        row {
                            width: "100%",
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: Some(diagnostic_pending),
                                onclick: move |_| dispatch(state, Action::FlushDnsCache),
                                text { content: translate_ui(current.locale, tr::page_tr_147()), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                            row { width: 8.0 }
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: Some(diagnostic_pending),
                                onclick: move |_| dispatch(state, Action::FlushFakeIpCache),
                                text { content: translate_ui(current.locale, tr::page_tr_148()), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_149()),
                snapshot.controller_addr.clone(),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(current.locale, tr::page_tr_150()), memory_in_use)}
                        {info_row(translate_ui(current.locale, tr::page_tr_151()), memory_limit)}
                        {info_row(translate_ui(current.locale, tr::page_tr_152()), snapshot.controller_diagnostics.config_sync_count.to_string())}
                        {info_row(translate_ui(current.locale, tr::page_tr_153()), last_config_sync)}
                        if let Some(error) = snapshot.controller_diagnostics.last_config_sync_error.clone() {
                            text { content: compact(&error), margin_top: 6.0, font_size: 12.0, font_color: danger(), max_lines: 3 }
                        }
                    }
                }
            )}
        }
    };
    scaffold(state, Route::Traffic {}, rsx! {}, body)
}
