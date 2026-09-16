use super::super::*;
use super::{VirtualProxyGroupList, VirtualProxyPalette};

pub(crate) fn dashboard_page() -> Element {
    let services = use_context::<UiServices>();
    let vpn_services = services.clone();
    let proxy_services = services.clone();
    let stores = use_context::<UiStores>();
    let operations = use_context::<UiOperationStores>();
    let mut quick_expanded_group = use_signal(|| None::<String>);
    let quick_proxy_projection = use_memo(move || {
        let proxies = stores.proxies.read();
        (
            grouped_proxy_rows(&proxies.groups, "", quick_expanded_group().as_deref()),
            proxy_group_summary(&proxies.groups),
        )
    });
    let preferences = stores.preferences.read();
    let session = stores.session.read();
    let profiles = stores.profiles.read();
    let proxies = stores.proxies.read();
    let activity = stores.activity.read();
    let telemetry = stores.telemetry.read();
    let vpn_operation = operations.vpn.read().active.clone();
    let s = preferences.locale;
    let navigator = use_navigator();
    let vpn_starting = vpn_operation.as_ref().map_or_else(
        || matches!(session.lifecycle, VpnLifecycle::Starting),
        |operation| {
            operation.phase != VpnOperationPhase::Unconfirmed
                && matches!(
                    operation.action,
                    VpnCommandAction::Start | VpnCommandAction::Restart
                )
        },
    );
    let vpn_stopping = vpn_operation.as_ref().is_some_and(|operation| {
        operation.phase != VpnOperationPhase::Unconfirmed
            && matches!(
                operation.action,
                VpnCommandAction::Stop | VpnCommandAction::OwnedStop
            )
    });
    let vpn_unconfirmed = vpn_operation.as_ref().is_some_and(|operation| {
        matches!(
            operation.phase,
            VpnOperationPhase::Unconfirmed | VpnOperationPhase::Confirming
        )
    });
    let vpn_confirming = vpn_operation
        .as_ref()
        .is_some_and(|operation| operation.phase == VpnOperationPhase::Confirming);
    let transitioning = vpn_starting || vpn_stopping;
    let disabled = vpn_operation.is_some() || matches!(session.lifecycle, VpnLifecycle::Starting);
    let connected = session.vpn_running && !transitioning;
    let status_label = if vpn_confirming {
        translate_ui(s, tr::vpn_operation_confirming())
    } else if vpn_unconfirmed {
        translate_ui(s, tr::vpn_operation_unconfirmed())
    } else if vpn_starting {
        translate_ui(s, tr::page_tr_248())
    } else if vpn_stopping {
        translate_ui(s, tr::page_tr_249())
    } else {
        match session.lifecycle {
            VpnLifecycle::Stopped | VpnLifecycle::EngineLoaded => {
                translate_ui(s, tr::dashboard_disconnected())
            }
            VpnLifecycle::Starting => translate_ui(s, tr::lifecycle_starting()),
            VpnLifecycle::Connected => translate_ui(s, tr::dashboard_connected()),
            VpnLifecycle::ProtectFailed => translate_ui(s, tr::lifecycle_protect_failed()),
            VpnLifecycle::Failed => translate_ui(s, tr::page_tr_251()),
        }
    };
    let profile = profiles
        .profiles
        .iter()
        .find(|profile| profiles.active_profile.as_deref() == Some(profile.id.as_str()))
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| translate_ui(s, tr::dashboard_profile_empty()));
    let status_color = if transitioning {
        subtle()
    } else if matches!(
        session.lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        danger()
    } else if connected {
        success()
    } else {
        subtle()
    };
    let (quick_rows, quick_summary) = {
        let projection = quick_proxy_projection.read();
        (projection.0.clone(), projection.1)
    };
    let global_node_count = quick_rows
        .iter()
        .find_map(|row| match row {
            ProxyGroupRow::Group(group) if group.name.eq_ignore_ascii_case("GLOBAL") => {
                Some(group.member_count)
            }
            _ => None,
        })
        .unwrap_or(0);
    let current_node = match proxies.mode {
        RuntimeMode::Direct => translate_ui(s, tr::proxies_direct()),
        RuntimeMode::Global => effective_group_leaf(&proxies.groups, "GLOBAL")
            .unwrap_or_else(|| translate_ui(s, tr::page_tr_154())),
        RuntimeMode::Rule => primary_selected_group_leaf(&proxies.groups)
            .or_else(|| latest_active_rule_node(&activity.connections))
            .unwrap_or_else(|| translate_ui(s, tr::page_tr_154())),
    };
    let quick_count = quick_summary.members;
    let quick_group_count = quick_summary.groups;
    let proxy_group_context = match s {
        UiLocale::ZhCn => translate_ui(
            s,
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
    let subscriptions_navigator = navigator;
    let all_nodes_navigator = navigator;
    let confirm_services = services.clone();
    let recover_services = services.clone();
    let exit_location = exit_location_label(&telemetry.exit_location, connected, s);
    let status_icon = if vpn_unconfirmed {
        "triangle-alert"
    } else if connected {
        "shield-check"
    } else if matches!(
        session.lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        "triangle-alert"
    } else {
        "power"
    };

    let theme = use_theme();
    let body = rsx! {
        column {
            width: "100%",
            layout_weight: 1.0,
            column {
                width: "100%",
                button {
                    button_type: "normal",
                    width: "100%",
                    height: 52.0,
                    padding: 0.0,
                    background_color: 0x00000000,
                    border_width: 0.0,
                    enabled: !disabled,
                    onclick: move |_| vpn_services.toggle_vpn(),
                    row {
                        width: "100%",
                        height: 52.0,
                        align_items: "center",
                        row {
                            width: 40.0,
                            height: 40.0,
                            align_items: "center",
                            justify_content: "center",
                            background_color: muted(),
                            border_radius: theme.radii.lg,
                            if transitioning {
                                Spinner { size: 18.0, color: Some(status_color) }
                            } else {
                                {arkit::icon(status_icon, 18.0, status_color)}
                            }
                        }
                        column {
                            layout_weight: 1.0,
                            margin_left: spacing::MD,
                            align_items: "start",
                            text {
                                content: status_label,
                                font_size: typography::XL,
                                line_height: 24.0,
                                font_weight: 600,
                                font_color: status_color,
                            }
                            text {
                                width: "100%",
                                content: profile,
                                margin_top: 1.0,
                                font_size: typography::XS,
                                line_height: 16.0,
                                font_color: subtle(),
                                max_lines: 1,
                                text_overflow: "ellipsis",
                            }
                        }
                    }
                }
                row { height: 14.0 }
                if vpn_unconfirmed {
                    row {
                        width: "100%",
                        justify_content: "end",
                        Button {
                            variant: ButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            disabled: Some(vpn_confirming),
                            onclick: move |_| confirm_services.confirm_vpn_operation(),
                            {translate_ui(s, tr::vpn_operation_confirm())}
                        }
                        row { width: spacing::SM }
                        Button {
                            variant: ButtonVariant::Destructive,
                            size: ButtonSize::Sm,
                            onclick: move |_| recover_services.recover_vpn_operation(),
                            {translate_ui(s, tr::vpn_operation_stop_resync())}
                        }
                    }
                    row { height: 14.0 }
                }
                {mode_picker(proxies.mode, s)}
                row { height: 14.0 }
                column {
                    width: "100%",
                    padding_left: spacing::MD,
                    padding_right: spacing::MD,
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: theme.radii.lg,
                    background_color: surface(),
                    column {
                        width: "100%",
                        height: 89.0,
                        {dashboard_connection_row(
                            "git-branch",
                            translate_ui(s, tr::page_tr_252()),
                            current_node,
                        )}
                        Separator {}
                        {dashboard_connection_row(
                            "network",
                            translate_ui(s, tr::page_tr_253()),
                            exit_location,
                        )}
                    }
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
                        content: translate_ui(s, tr::page_tr_254()),
                        font_size: typography::SM,
                        line_height: 20.0,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                    text { content: proxy_group_context, margin_top: 1.0, font_size: typography::XS, line_height: 16.0, font_color: subtle(), max_lines: 1 }
                }
                if quick_count > 0 {
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            all_nodes_navigator.push(Route::Proxies {});
                        },
                        text { content: translate_ui(s, tr::page_tr_273()), font_size: typography::XS, font_weight: 500, font_color: subtle() }
                        {arkit::icon("chevron-right", 14.0, subtle())}
                    }
                }
            }
            row { height: 6.0 }
            if quick_group_count == 0 {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    padding_top: 36.0,
                    align_items: "center",
                    justify_content: "start",
                    row {
                        width: 48.0,
                        height: 48.0,
                        align_items: "center",
                        justify_content: "center",
                        background_color: muted(),
                        border_radius: theme.radii.xl,
                        {arkit::icon("rss", 20.0, subtle())}
                    }
                    text { content: translate_ui(s, tr::page_tr_256()), margin_top: 12.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                    text { content: translate_ui(s, tr::page_tr_257()), margin_top: 6.0, font_size: typography::XS, line_height: 18.0, font_color: subtle(), text_align: "center" }
                    row { height: 16.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Primary,
                        onclick: move |_| {
                            subscriptions_navigator.push(Route::Profiles {});
                        },
                        {arkit::icon("plus", 14.0, primary_text())}
                        text { content: translate_ui(s, tr::page_tr_098()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: primary_text() }
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
                        locale: s,
                        palette: quick_palette,
                        on_toggle: move |group: String| {
                            let next = (quick_expanded_group().as_deref() != Some(group.as_str()))
                                .then_some(group);
                            quick_expanded_group.set(next);
                        },
                        on_select: move |(group, proxy): (String, String)| {
                            let proxy = (!proxy.is_empty()).then_some(proxy);
                            proxy_services.select_proxy(group, proxy);
                        },
                    }
                }
            }
        }
    };
    fixed_scaffold_flush_bottom(Route::Dashboard {}, rsx! {}, body)
}

fn dashboard_connection_row(icon_name: &'static str, label: String, value: String) -> Element {
    rsx! {
        row {
            width: "100%",
            height: 44.0,
            align_items: "center",
            clip: true,
            {arkit::icon(icon_name, 15.0, subtle())}
            text {
                width: 68.0,
                content: label,
                margin_left: 8.0,
                font_size: typography::XS,
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
                    font_size: typography::SM,
                    line_height: 20.0,
                    font_weight: 500,
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
            translate_ui(UiLocale::ZhCn, tr::page_tr_126())
        );
    }
}

fn mode_picker(selected: RuntimeMode, locale: UiLocale) -> Element {
    let services = use_context::<UiServices>();
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
                services.set_mode(mode);
            },
        }
    }
}
