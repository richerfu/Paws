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
        .unwrap_or_else(|| tr(current.locale, "尚未同步", "Not synced yet").to_owned());
    let body = rsx! {
        column {
            width: "100%",
            align_items: "start",
            row {
                width: "100%",
                align_items: "center",
                text { content: if connected { tr(current.locale, "已连接", "Connected") } else { tr(current.locale, "未连接", "Disconnected") }, font_size: 14.0, font_weight: 650, font_color: if connected { success() } else { subtle() } }
                row { layout_weight: 1.0 }
                {pill(if connected { tr(current.locale, "VPN 运行中", "VPN running") } else { tr(current.locale, "VPN 已停止", "VPN stopped") }.to_owned(), if connected { success() } else { subtle() })}
            }
            row { height: 18.0 }
            text { content: tr(current.locale, "流量用量", "Data usage"), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            row {
                width: "100%",
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        tr(current.locale, "当前配置", "Active profile"),
                        profile_upload,
                        profile_download,
                    )}
                }
                row { width: 10.0 }
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        tr(current.locale, "本次会话", "This session"),
                        snapshot.traffic.upload_bytes,
                        snapshot.traffic.download_bytes,
                    )}
                }
            }
            row { height: 18.0 }
            text { content: tr(current.locale, "当前会话", "Current session"), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            {traffic_metrics(
                strings(current.locale).traffic_download,
                format_speed(snapshot.traffic.download_speed),
                strings(current.locale).traffic_upload,
                format_speed(snapshot.traffic.upload_speed),
            )}
            row { height: 18.0 }
            {card(
                tr(current.locale, "速率图表", "Speed chart"),
                Some(format!("{} {}", samples, strings(current.locale).traffic_sample_unit)),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(strings(current.locale).traffic_peak_download, format_speed(peak_download))}
                        {info_row(strings(current.locale).traffic_peak_upload, format_speed(peak_upload))}
                        {speed_bars(&snapshot.traffic_history)}
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                tr(current.locale, "当前连接", "Active connections"),
                Some(format!("{} {}", active_connection_count, tr(current.locale, "条", "active"))),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "连接下载", "Connection download"), format_total(connection_download))}
                        {info_row(tr(current.locale, "连接上传", "Connection upload"), format_total(connection_upload))}
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
                            text { content: tr(current.locale, "查看全部连接", "View all connections"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            {arkit::icon("chevron-right", 14.0, subtle())}
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                strings(current.locale).traffic_dns_title.to_owned(),
                Some(snapshot.dns.model.clone()),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "DNS 劫持", "DNS hijack"), if snapshot.dns.hijacking { tr(current.locale, "已启用", "Enabled") } else { tr(current.locale, "已关闭", "Disabled") })}
                        {info_row(tr(current.locale, "监听地址", "Listen"), snapshot.dns.listen.clone())}
                        {info_row(tr(current.locale, "TUN DNS", "TUN DNS"), dns_tun_addresses)}
                        {info_row(tr(current.locale, "上游 DNS", "Upstreams"), dns_upstreams)}
                        {info_row(tr(current.locale, "备用 DNS", "Fallbacks"), dns_fallbacks)}
                        {info_row(tr(current.locale, "域名策略", "Domain policies"), snapshot.dns.nameserver_policy.len().to_string())}
                        {info_row(strings(current.locale).traffic_dns_handled, snapshot.dns.handled_packets.to_string())}
                        {info_row(strings(current.locale).dns_cache_hits, snapshot.dns.cache_hits.to_string())}
                        {info_row(strings(current.locale).dns_cache_misses, snapshot.dns.cache_misses.to_string())}
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
                                text { content: tr(current.locale, "清理 DNS 缓存", "Flush DNS cache"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                            row { width: 8.0 }
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: Some(diagnostic_pending),
                                onclick: move |_| dispatch(state, Action::FlushFakeIpCache),
                                text { content: tr(current.locale, "清理 Fake-IP", "Flush Fake-IP"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                tr(current.locale, "Controller 诊断", "Controller diagnostics"),
                snapshot.controller_addr.clone(),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "内存占用", "Memory in use"), memory_in_use)}
                        {info_row(tr(current.locale, "系统内存上限", "OS memory limit"), memory_limit)}
                        {info_row(tr(current.locale, "配置同步次数", "Config sync count"), snapshot.controller_diagnostics.config_sync_count.to_string())}
                        {info_row(tr(current.locale, "最近配置同步", "Last config sync"), last_config_sync)}
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
