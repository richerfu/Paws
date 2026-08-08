use super::super::*;

pub(crate) fn settings_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let (initial_dns_servers, initial_dns_fallbacks, initial_dns_policy) =
        dns_draft_from_snapshot(&current.snapshot);
    let (initial_system_proxy, initial_dns_hijacking, initial_allow_bypass, initial_stack) =
        vpn_draft_from_snapshot(&current.snapshot);
    // arkit Select shows the option string itself; map display labels ↔ stack values.
    let stack_label_smoltcp = "smoltcp".to_owned();
    let stack_label_lwip = "lwIP".to_owned();
    let stack_options = vec![stack_label_smoltcp.clone(), stack_label_lwip.clone()];
    let stack_selected_label =
        match hmeta_model::VpnStack::try_from(initial_stack.as_str()).unwrap_or_default() {
            hmeta_model::VpnStack::Lwip => stack_label_lwip.clone(),
            hmeta_model::VpnStack::Smoltcp => stack_label_smoltcp.clone(),
        };

    let mut dns_servers = use_signal({
        let value = initial_dns_servers.clone();
        move || value
    });
    let mut dns_fallbacks = use_signal({
        let value = initial_dns_fallbacks.clone();
        move || value
    });
    let mut dns_policy = use_signal({
        let value = initial_dns_policy.clone();
        move || value
    });
    let mut system_proxy = use_signal(move || initial_system_proxy);
    let mut dns_hijacking = use_signal(move || initial_dns_hijacking);
    let mut allow_bypass = use_signal(move || initial_allow_bypass);
    let initial_controller_allow_lan = current.snapshot.controller_access.allow_lan;
    let mut controller_allow_lan = use_signal(move || initial_controller_allow_lan);
    let initial_mixed_port = current.snapshot.network_ports.mixed_port.to_string();
    let initial_controller_port = current.snapshot.network_ports.controller_port.to_string();
    let mut mixed_port = use_signal({
        let value = initial_mixed_port.clone();
        move || value
    });
    let mut controller_port = use_signal({
        let value = initial_controller_port.clone();
        move || value
    });
    let mut vpn_stack = use_signal({
        let value = initial_stack.clone();
        move || value
    });
    let mut vpn_stack_label = use_signal({
        let value = stack_selected_label.clone();
        move || value
    });

    let dns_servers_value = dns_servers();
    let dns_fallbacks_value = dns_fallbacks();
    let dns_policy_value = dns_policy();
    let system_proxy_value = system_proxy();
    let dns_hijacking_value = dns_hijacking();
    let allow_bypass_value = allow_bypass();
    let controller_allow_lan_value = controller_allow_lan();
    let mixed_port_value = mixed_port();
    let controller_port_value = controller_port();
    let vpn_stack_value = vpn_stack();
    let vpn_stack_label_value = vpn_stack_label();
    let vpn_dirty = system_proxy_value != initial_system_proxy
        || dns_hijacking_value != initial_dns_hijacking
        || allow_bypass_value != initial_allow_bypass
        || vpn_stack_value != initial_stack;
    let dns_dirty = dns_servers_value != initial_dns_servers
        || dns_fallbacks_value != initial_dns_fallbacks
        || dns_policy_value != initial_dns_policy;
    let network_dirty = controller_allow_lan_value != initial_controller_allow_lan
        || mixed_port_value != initial_mixed_port
        || controller_port_value != initial_controller_port;
    let controller_loopback_addr = format!("127.0.0.1:{controller_port_value}");
    let controller_lan_description = format!(
        "{}:{controller_port_value}",
        translate_ui(current.locale, tr::page_tr_221())
    );
    let controller_listen_description = format!(
        "{} 0.0.0.0:{controller_port_value}",
        translate_ui(current.locale, tr::page_tr_222())
    );
    let controller_secret = current.snapshot.controller_access.secret.clone();
    let controller_secret_label = controller_secret
        .as_deref()
        .map(mask_controller_secret)
        .unwrap_or_else(|| translate_ui(current.locale, tr::hard_zh_027()));

    let body = rsx! {
        column {
            width: "100%",
            {card(
                translate_ui(current.locale, tr::page_tr_223()),
                Some(translate_ui(current.locale, tr::page_tr_224())),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_225()) }
                                FieldDescription { content: translate_ui(current.locale, tr::page_tr_226()), inset: true }
                            }
                            Switch { checked: Some(system_proxy_value), on_change: move |value| system_proxy.set(value) }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_227()) }
                                FieldDescription { content: translate_ui(current.locale, tr::page_tr_228()), inset: true }
                            }
                            Switch { checked: Some(dns_hijacking_value), on_change: move |value| dns_hijacking.set(value) }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_229()) }
                                FieldDescription { content: translate_ui(current.locale, tr::page_tr_230()), inset: true }
                            }
                            Switch { checked: Some(allow_bypass_value), on_change: move |value| allow_bypass.set(value) }
                        }
                        row { height: 12.0 }
                        FormItem {
                            label: translate_ui(current.locale, tr::page_tr_231()),
                            Select {
                                options: stack_options,
                                selected: Some(vpn_stack_label_value.clone()),
                                default_selected: stack_selected_label.clone(),
                                default_open: false,
                                on_select: move |label: String| {
                                    let value = match label.as_str() {
                                        "lwIP" | "lwip" => {
                                            hmeta_model::VpnStack::Lwip.as_str().to_owned()
                                        }
                                        _ => hmeta_model::VpnStack::Smoltcp.as_str().to_owned(),
                                    };
                                    vpn_stack_label.set(label);
                                    vpn_stack.set(value);
                                },
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(!vpn_dirty),
                            onclick: move |_| dispatch(state, Action::SaveVpnSettings {
                                system_proxy: system_proxy_value,
                                dns_hijacking: dns_hijacking_value,
                                allow_bypass: allow_bypass_value,
                                stack: vpn_stack_value.clone(),
                            }),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_232()), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_233()),
                Some(translate_ui(current.locale, tr::page_tr_234())),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        FormItem {
                            label: translate_ui(current.locale, tr::page_tr_235()),
                            Input {
                                value: Some(mixed_port_value.clone()),
                                placeholder: Some("7890".to_owned()),
                                width: Some("100%".into()),
                                on_change: move |value| mixed_port.set(value),
                            }
                        }
                        FieldDescription {
                            content: translate_ui(current.locale, tr::page_tr_236()),
                            inset: true,
                        }
                        FormItem {
                            label: translate_ui(current.locale, tr::page_tr_237()),
                            Input {
                                value: Some(controller_port_value.clone()),
                                placeholder: Some("9090".to_owned()),
                                width: Some("100%".into()),
                                on_change: move |value| controller_port.set(value),
                            }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_238()) }
                                FieldDescription { content: controller_listen_description.clone(), inset: true }
                            }
                            Switch { checked: Some(controller_allow_lan_value), on_change: move |value| controller_allow_lan.set(value) }
                        }
                        row { height: 12.0 }
                        column {
                            width: "100%",
                            text { content: translate_ui(current.locale, tr::page_tr_239()), font_size: typography::XS, font_color: subtle() }
                            text { content: controller_loopback_addr.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                        if controller_allow_lan_value {
                            column {
                                width: "100%",
                                margin_top: 12.0,
                                text { content: translate_ui(current.locale, tr::page_tr_240()), font_size: typography::XS, font_color: subtle() }
                                text { content: controller_lan_description.clone(), margin_top: 4.0, font_size: typography::SM, font_color: text_color() }
                                text { content: "Authorization: Bearer <secret>", margin_top: 4.0, font_size: typography::XS, font_color: subtle() }
                            }
                            row {
                                width: "100%",
                                margin_top: 12.0,
                                align_items: "center",
                                column {
                                    width: "72%",
                                    text { content: translate_ui(current.locale, tr::page_tr_241()), font_size: typography::XS, font_color: subtle() }
                                    text { content: controller_secret_label.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                                }
                                if let Some(secret) = controller_secret.clone() {
                                    FlatButton {
                                        onclick: move |_| copy_controller_secret(state, secret.clone()),
                                        {arkit::icon("copy", 16.0, text_color())}
                                        text { content: translate_ui(current.locale, tr::page_tr_242()), margin_left: 6.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                                    }
                                }
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(!network_dirty),
                            onclick: move |_| dispatch(state, Action::SaveNetworkSettings {
                                mixed_port: mixed_port_value.clone(),
                                controller_port: controller_port_value.clone(),
                                allow_lan: controller_allow_lan_value,
                            }),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_243()), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                "DNS",
                Some(translate_ui(current.locale, tr::page_tr_244())),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        FormItem {
                            label: translate_ui(current.locale, tr::page_tr_245()),
                            Textarea {
                                value: Some(dns_servers_value.clone()),
                                height: Some(92.0),
                                width: Some("100%".into()),
                                on_change: move |value| dns_servers.set(value),
                            }
                        }
                        FormItem {
                            label: "Fallback".to_owned(),
                            Textarea {
                                value: Some(dns_fallbacks_value.clone()),
                                height: Some(76.0),
                                width: Some("100%".into()),
                                on_change: move |value| dns_fallbacks.set(value),
                            }
                        }
                        FormItem {
                            label: translate_ui(current.locale, tr::page_tr_246()),
                            Textarea {
                                value: Some(dns_policy_value.clone()),
                                height: Some(104.0),
                                width: Some("100%".into()),
                                on_change: move |value| dns_policy.set(value),
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(!dns_dirty),
                            onclick: move |_| dispatch(state, Action::SaveDnsSettings {
                                servers_text: dns_servers_value.clone(),
                                fallbacks_text: dns_fallbacks_value.clone(),
                                policy_text: dns_policy_value.clone(),
                            }),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_247()), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
        }
    };
    scaffold(state, Route::Settings {}, rsx! {}, body)
}

fn mask_controller_secret(secret: &str) -> String {
    if secret.len() <= 16 {
        return secret.to_owned();
    }
    format!("{}…{}", &secret[..8], &secret[secret.len() - 8..])
}

fn copy_controller_secret(state: Signal<State>, secret: String) {
    let task = state
        .read()
        .runtime
        .tokio()
        .spawn(async move { crate::bridge::copy_text(secret).await });
    arkit::dioxus_core::spawn_forever(async move {
        let message = match task.await {
            Ok(Ok(())) => translate_ui(state.read().locale, tr::hard_zh_028()),
            Ok(Err(error)) => format!(
                "{}{}",
                translate_ui(state.read().locale, tr::hard_zh_029()),
                error
            ),
            Err(error) => format!(
                "{}{}",
                translate_ui(state.read().locale, tr::hard_zh_030()),
                error
            ),
        };
        state.read().notifications.publish(message);
    });
}
