use super::super::*;
use super::{VirtualProxyGroupList, VirtualProxyPalette};

pub(crate) fn dashboard_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let snapshot = current.snapshot;
    let s = current.locale;
    let navigator = use_navigator();
    let mut quick_expanded_group = use_signal(|| None::<String>);
    let vpn_starting = current.vpn_command_pending == Some(VpnCommandAction::Start)
        || matches!(snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    let vpn_stopping = current.vpn_command_pending == Some(VpnCommandAction::Stop);
    let transitioning = vpn_starting || vpn_stopping;
    let connected = snapshot.vpn_running && !transitioning;
    let status_label = if vpn_starting {
        translate_ui(current.locale, tr::page_tr_248())
    } else if vpn_stopping {
        translate_ui(current.locale, tr::page_tr_249())
    } else {
        match snapshot.vpn_lifecycle {
            VpnLifecycle::Stopped => translate_ui(s, tr::dashboard_disconnected()),
            VpnLifecycle::EngineLoaded => translate_ui(current.locale, tr::page_tr_250()),
            VpnLifecycle::Starting => translate_ui(s, tr::lifecycle_starting()),
            VpnLifecycle::Connected => translate_ui(s, tr::dashboard_connected()),
            VpnLifecycle::ProtectFailed => translate_ui(s, tr::lifecycle_protect_failed()),
            VpnLifecycle::Failed => translate_ui(current.locale, tr::page_tr_251()),
        }
    };
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| snapshot.active_profile.as_deref() == Some(profile.id.as_str()))
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| translate_ui(s, tr::dashboard_profile_empty()));
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
        RuntimeMode::Direct => translate_ui(s, tr::proxies_direct()),
        RuntimeMode::Global => effective_group_leaf(&snapshot.proxy_groups, "GLOBAL")
            .unwrap_or_else(|| translate_ui(current.locale, tr::page_tr_154())),
        RuntimeMode::Rule => primary_selected_group_leaf(&snapshot.proxy_groups)
            .or_else(|| latest_active_rule_node(&snapshot.connections))
            .unwrap_or_else(|| translate_ui(current.locale, tr::page_tr_154())),
    };
    let quick_count = quick_summary.members;
    let quick_group_count = quick_summary.groups;
    let proxy_group_context = match current.locale {
        UiLocale::ZhCn => translate_ui(
            current.locale,
            tr::hard_zh_012(global_node_count, quick_count, quick_group_count),
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
                        translate_ui(current.locale, tr::page_tr_252()),
                        current_node,
                    )}
                    Separator {}
                    {dashboard_connection_row(
                        "network",
                        translate_ui(current.locale, tr::page_tr_253()),
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
                        content: translate_ui(current.locale, tr::page_tr_254()),
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
                        text { content: translate_ui(current.locale, tr::page_tr_255()), font_size: 12.0, font_weight: 600, font_color: text_color() }
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
                    text { content: translate_ui(current.locale, tr::page_tr_256()), margin_top: 9.0, font_size: 14.0, font_weight: 700, font_color: text_color() }
                    text { content: translate_ui(current.locale, tr::page_tr_257()), margin_top: 3.0, font_size: 11.0, line_height: 16.0, font_color: subtle(), text_align: "center" }
                    row { height: 10.0 }
                    Button {
                        variant: ButtonVariant::Default,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            subscriptions_navigator.push(Route::Profiles {});
                        },
                        {arkit::icon("plus", 14.0, primary_text())}
                        text { content: translate_ui(current.locale, tr::page_tr_098()), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: primary_text() }
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

fn dashboard_connection_row(icon_name: &'static str, label: String, value: String) -> Element {
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
    location: &paws_model::ExitLocationSnapshot,
    connected: bool,
    locale: UiLocale,
) -> String {
    if !connected {
        return translate_ui(locale, tr::page_tr_126());
    }
    if location.ip.is_empty() {
        return if location.error.is_some() {
            translate_ui(locale, tr::page_tr_258())
        } else {
            translate_ui(locale, tr::page_tr_259())
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
        let location = paws_model::ExitLocationSnapshot {
            ip: "203.0.113.9".to_owned(),
            country: "Japan".to_owned(),
            country_code: "JP".to_owned(),
            ..paws_model::ExitLocationSnapshot::default()
        };

        assert_eq!(
            exit_location_label(&location, true, UiLocale::ZhCn),
            "203.0.113.9 · 🇯🇵 Japan"
        );
    }

    #[test]
    fn exit_location_does_not_show_cached_data_while_disconnected() {
        let location = paws_model::ExitLocationSnapshot {
            ip: "203.0.113.9".to_owned(),
            country: "Japan".to_owned(),
            country_code: "JP".to_owned(),
            ..paws_model::ExitLocationSnapshot::default()
        };

        assert_eq!(
            exit_location_label(&location, false, UiLocale::ZhCn),
            translate_ui(current.locale, tr::hard_zh_013())
        );
    }
}

fn mode_picker(state: Signal<State>, selected: RuntimeMode, locale: UiLocale) -> Element {
    let rule = translate_ui(locale, tr::page_tr_164());
    let global = translate_ui(locale, tr::page_tr_165());
    let direct = translate_ui(locale, tr::page_tr_166());
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
