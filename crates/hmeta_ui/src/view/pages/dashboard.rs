use super::super::*;
use super::{VirtualProxyGroupList, VirtualProxyPalette};

pub(crate) fn dashboard_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let snapshot = current.snapshot;
    let s = strings(current.locale);
    let navigator = use_navigator();
    let mut quick_expanded_group = use_signal(|| None::<String>);
    let vpn_starting = current.vpn_command_pending == Some(VpnCommandAction::Start)
        || matches!(snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    let vpn_stopping = current.vpn_command_pending == Some(VpnCommandAction::Stop);
    let transitioning = vpn_starting || vpn_stopping;
    let connected = snapshot.vpn_running && !transitioning;
    let status_label = if vpn_starting {
        tr(current.locale, "正在连接", "Connecting")
    } else if vpn_stopping {
        tr(current.locale, "正在断开", "Disconnecting")
    } else {
        match snapshot.vpn_lifecycle {
            VpnLifecycle::Stopped => s.dashboard_disconnected,
            VpnLifecycle::EngineLoaded => tr(current.locale, "配置已就绪", "Ready to connect"),
            VpnLifecycle::Starting => s.lifecycle_starting,
            VpnLifecycle::Connected => s.dashboard_connected,
            VpnLifecycle::ProtectFailed => s.lifecycle_protect_failed,
            VpnLifecycle::Failed => tr(current.locale, "VPN 启动失败", "VPN failed"),
        }
    };
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| snapshot.active_profile.as_deref() == Some(profile.id.as_str()))
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| s.dashboard_profile_empty.to_owned());
    let status_color = if transitioning {
        subtle()
    } else if matches!(
        snapshot.vpn_lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        danger()
    } else if connected {
        success()
    } else {
        subtle()
    };
    let quick_rows = grouped_proxy_rows(
        &snapshot.proxy_groups,
        "",
        quick_expanded_group().as_deref(),
    );
    let quick_summary = proxy_group_summary(&snapshot.proxy_groups);
    let global_node_count = quick_rows
        .iter()
        .find_map(|row| match row {
            ProxyGroupRow::Group(group) if group.name.eq_ignore_ascii_case("GLOBAL") => {
                Some(group.member_count)
            }
            _ => None,
        })
        .unwrap_or(0);
    let current_node = match snapshot.mode {
        RuntimeMode::Direct => s.proxies_direct.to_owned(),
        RuntimeMode::Global => effective_group_leaf(&snapshot.proxy_groups, "GLOBAL")
            .unwrap_or_else(|| tr(current.locale, "未选择", "Unselected").to_owned()),
        RuntimeMode::Rule => primary_selected_group_leaf(&snapshot.proxy_groups)
            .or_else(|| latest_active_rule_node(&snapshot.connections))
            .unwrap_or_else(|| tr(current.locale, "未选择", "Unselected").to_owned()),
    };
    let quick_count = quick_summary.members;
    let quick_group_count = quick_summary.groups;
    let proxy_group_context = match current.locale {
        UiLocale::ZhCn => format!(
            "{global_node_count} 个全局节点 · {quick_count} 个节点 · {quick_group_count} 个分组"
        ),
        UiLocale::En => format!(
            "{global_node_count} global nodes · {quick_count} nodes · {quick_group_count} groups"
        ),
    };
    let quick_palette = VirtualProxyPalette {
        surface: surface(),
        selected_surface: muted(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
    };
    let subscriptions_navigator = navigator.clone();
    let all_nodes_navigator = navigator.clone();
    let exit_location = exit_location_label(&snapshot.exit_location, connected, current.locale);
    let status_icon = if connected {
        "shield-check"
    } else if matches!(
        snapshot.vpn_lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        "triangle-alert"
    } else {
        "power"
    };

    let body = rsx! {
        column {
            width: "100%",
            layout_weight: 1.0,
            column {
                width: "100%",
                row {
                    width: "100%",
                    height: 52.0,
                    align_items: "center",
                    row {
                        width: 42.0,
                        height: 42.0,
                        align_items: "center",
                        justify_content: "center",
                        background_color: muted(),
                        border_radius: 10.0,
                        if transitioning {
                            Spinner { size: 20.0, color: Some(status_color) }
                        } else {
                            {arkit::icon(status_icon, 20.0, status_color)}
                        }
                    }
                    column {
                        layout_weight: 1.0,
                        margin_left: 12.0,
                        align_items: "start",
                        text {
                            content: status_label,
                            font_size: 19.0,
                            line_height: 24.0,
                            font_weight: 700,
                            font_color: status_color,
                        }
                        text {
                            width: "100%",
                            content: profile,
                            margin_top: 1.0,
                            font_size: 11.0,
                            line_height: 16.0,
                            font_color: subtle(),
                            max_lines: 1,
                            text_overflow: "ellipsis",
                        }
                    }
                }
                row { height: 14.0 }
                {mode_picker(state, snapshot.mode, current.locale)}
                row { height: 14.0 }
                column {
                    width: "100%",
                    height: 89.0,
                    padding_left: 4.0,
                    padding_right: 4.0,
                    {dashboard_connection_row(
                        "git-branch",
                        tr(current.locale, "当前节点", "Current node"),
                        current_node,
                    )}
                    Separator {}
                    {dashboard_connection_row(
                        "network",
                        tr(current.locale, "出口 IP", "Exit IP"),
                        exit_location,
                    )}
                }
            }
            row { height: 14.0 }
            row {
                width: "100%",
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text {
                        content: tr(current.locale, "全局节点与策略分组", "Global node and policy groups"),
                        font_size: 17.0,
                        line_height: 22.0,
                        font_weight: 700,
                        font_color: text_color(),
                    }
                    text { content: proxy_group_context, margin_top: 1.0, font_size: 10.0, line_height: 14.0, font_color: subtle(), max_lines: 1 }
                }
                if quick_count > 0 {
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            all_nodes_navigator.push(Route::Proxies {});
                        },
                        text { content: tr(current.locale, "搜索", "Search"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                        {arkit::icon("chevron-right", 14.0, subtle())}
                    }
                }
            }
            row { height: 6.0 }
            if quick_group_count == 0 {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    align_items: "center",
                    justify_content: "center",
                    {arkit::icon("rss", 21.0, subtle())}
                    text { content: tr(current.locale, "尚未选择订阅", "No subscription selected"), margin_top: 9.0, font_size: 14.0, font_weight: 700, font_color: text_color() }
                    text { content: tr(current.locale, "添加并启用订阅后即可选择节点", "Add and activate a subscription to choose nodes"), margin_top: 3.0, font_size: 11.0, line_height: 16.0, font_color: subtle(), text_align: "center" }
                    row { height: 10.0 }
                    Button {
                        variant: ButtonVariant::Default,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            subscriptions_navigator.push(Route::Profiles {});
                        },
                        {arkit::icon("plus", 14.0, primary_text())}
                        text { content: tr(current.locale, "添加订阅", "Add subscription"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: primary_text() }
                    }
                }
            } else {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    clip: true,
                    VirtualProxyGroupList {
                        key: "dashboard-quick-proxy-list",
                        rows: quick_rows,
                        locale: current.locale,
                        palette: quick_palette,
                        selection_pending: current.proxy_selection_pending.clone(),
                        on_toggle: move |group: String| {
                            let next = (quick_expanded_group().as_deref() != Some(group.as_str()))
                                .then_some(group);
                            quick_expanded_group.set(next);
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
    fixed_scaffold_flush_bottom(state, Route::Dashboard {}, rsx! {}, body)
}

fn dashboard_connection_row(
    icon_name: &'static str,
    label: &'static str,
    value: String,
) -> Element {
    rsx! {
        row {
            width: "100%",
            height: 44.0,
            align_items: "center",
            clip: true,
            {arkit::icon(icon_name, 15.0, text_color())}
            text {
                width: 68.0,
                content: label,
                margin_left: 8.0,
                font_size: 11.0,
                line_height: 18.0,
                font_color: subtle(),
                max_lines: 1,
            }
            row {
                layout_weight: 1.0,
                margin_left: 12.0,
                clip: true,
                text {
                    width: "100%",
                    content: value,
                    font_size: 13.0,
                    line_height: 18.0,
                    font_weight: 650,
                    font_color: text_color(),
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
            }
        }
    }
}

fn exit_location_label(
    location: &hmeta_model::ExitLocationSnapshot,
    connected: bool,
    locale: UiLocale,
) -> String {
    if !connected {
        return tr(locale, "未连接", "Disconnected").to_owned();
    }
    if location.ip.is_empty() {
        return if location.error.is_some() {
            tr(locale, "暂时无法获取", "Temporarily unavailable").to_owned()
        } else {
            tr(locale, "正在查询…", "Checking…").to_owned()
        };
    }

    let country_code = location.country_code.trim().to_ascii_uppercase();
    let country = location.country.trim();
    let country_label = match (country_flag(&country_code), country.is_empty()) {
        (Some(flag), false) => format!("{flag} {country}"),
        (Some(flag), true) => format!("{flag} {country_code}"),
        (None, false) => country.to_owned(),
        (None, true) => country_code,
    };
    if country_label.is_empty() {
        location.ip.clone()
    } else {
        format!("{} · {country_label}", location.ip)
    }
}

fn country_flag(country_code: &str) -> Option<String> {
    let bytes = country_code.as_bytes();
    if bytes.len() != 2 || !bytes.iter().all(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    let mut flag = String::with_capacity(8);
    flag.push(char::from_u32(0x1F1E6 + u32::from(bytes[0] - b'A'))?);
    flag.push(char::from_u32(0x1F1E6 + u32::from(bytes[1] - b'A'))?);
    Some(flag)
}

#[cfg(test)]
mod exit_location_tests {
    use super::*;

    #[test]
    fn exit_location_shows_public_ip_and_country() {
        let location = hmeta_model::ExitLocationSnapshot {
            ip: "203.0.113.9".to_owned(),
            country: "Japan".to_owned(),
            country_code: "JP".to_owned(),
            ..hmeta_model::ExitLocationSnapshot::default()
        };

        assert_eq!(
            exit_location_label(&location, true, UiLocale::ZhCn),
            "203.0.113.9 · 🇯🇵 Japan"
        );
    }

    #[test]
    fn exit_location_does_not_show_cached_data_while_disconnected() {
        let location = hmeta_model::ExitLocationSnapshot {
            ip: "203.0.113.9".to_owned(),
            country: "Japan".to_owned(),
            country_code: "JP".to_owned(),
            ..hmeta_model::ExitLocationSnapshot::default()
        };

        assert_eq!(
            exit_location_label(&location, false, UiLocale::ZhCn),
            "未连接"
        );
    }
}

fn mode_picker(state: Signal<State>, selected: RuntimeMode, locale: UiLocale) -> Element {
    let rule = tr(locale, "规则", "Rule").to_owned();
    let global = tr(locale, "全局", "Global").to_owned();
    let direct = tr(locale, "直连", "Direct").to_owned();
    let selected_label = match selected {
        RuntimeMode::Rule => rule.clone(),
        RuntimeMode::Global => global.clone(),
        RuntimeMode::Direct => direct.clone(),
    };
    rsx! {
        FlatSegmented {
            options: vec![rule, global.clone(), direct.clone()],
            selected: selected_label,
            on_change: move |value: String| {
                let mode = if value == global {
                    RuntimeMode::Global
                } else if value == direct {
                    RuntimeMode::Direct
                } else {
                    RuntimeMode::Rule
                };
                dispatch(state, Action::SetMode(mode));
            },
        }
    }
}
