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
        tr(current.locale, "使用设备局域网 IP", "Use the device LAN IP")
    );
    let controller_listen_description = format!(
        "{} 0.0.0.0:{controller_port_value}",
        tr(current.locale, "开启后监听", "Listens on")
    );
    let controller_secret = current.snapshot.controller_access.secret.clone();
    let controller_secret_label = controller_secret
        .as_deref()
        .map(mask_controller_secret)
        .unwrap_or_else(|| {
            tr(
                current.locale,
                "保存后自动生成",
                "Generated automatically when saved",
            )
            .to_owned()
        });

    let body = rsx! {
        column {
            width: "100%",
            {card(
                tr(current.locale, "VPN 基础", "VPN basics"),
                Some(tr(current.locale, "运行中的 VPN 会在保存后请求重连", "A running VPN reconnects after saving").to_owned()),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: tr(current.locale, "系统代理", "System proxy").to_owned() }
                                FieldDescription { content: tr(current.locale, "同步设置系统 HTTP 代理", "Configure the system HTTP proxy").to_owned(), inset: true }
                            }
                            Switch { checked: Some(system_proxy_value), on_change: move |value| system_proxy.set(value) }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: tr(current.locale, "DNS 劫持", "DNS hijacking").to_owned() }
                                FieldDescription { content: tr(current.locale, "将 DNS 查询交给 meow-rs", "Route DNS queries through meow-rs").to_owned(), inset: true }
                            }
                            Switch { checked: Some(dns_hijacking_value), on_change: move |value| dns_hijacking.set(value) }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: tr(current.locale, "允许绕过", "Allow bypass").to_owned() }
                                FieldDescription { content: tr(current.locale, "允许应用绕过 VPN", "Allow applications to bypass VPN").to_owned(), inset: true }
                            }
                            Switch { checked: Some(allow_bypass_value), on_change: move |value| allow_bypass.set(value) }
                        }
                        row { height: 12.0 }
                        FormItem {
                            label: tr(current.locale, "网络栈", "Network stack").to_owned(),
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
                            text { content: tr(current.locale, "保存 VPN 设置", "Save VPN settings"), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                tr(current.locale, "网络端口与控制器", "Network ports and controller"),
                Some(tr(current.locale, "默认端口为 7890 和 9090；端口范围 1024–65535，且不能相同", "Defaults are 7890 and 9090; ports must be 1024–65535 and different").to_owned()),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        FormItem {
                            label: tr(current.locale, "混合代理端口", "Mixed proxy port").to_owned(),
                            Input {
                                value: Some(mixed_port_value.clone()),
                                placeholder: Some("7890".to_owned()),
                                width: Some("100%".into()),
                                on_change: move |value| mixed_port.set(value),
                            }
                        }
                        FieldDescription {
                            content: tr(current.locale, "VPN 扩展的本地 HTTP/SOCKS 监听；出口 IP 查询也会使用此端口", "Local HTTP/SOCKS listener for the VPN extension; exit IP queries use it too").to_owned(),
                            inset: true,
                        }
                        FormItem {
                            label: tr(current.locale, "控制器端口", "Controller port").to_owned(),
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
                                FieldTitle { content: tr(current.locale, "允许局域网访问", "Allow LAN access").to_owned() }
                                FieldDescription { content: controller_listen_description.clone(), inset: true }
                            }
                            Switch { checked: Some(controller_allow_lan_value), on_change: move |value| controller_allow_lan.set(value) }
                        }
                        row { height: 12.0 }
                        column {
                            width: "100%",
                            text { content: tr(current.locale, "应用内部地址", "In-app address"), font_size: typography::XS, font_color: subtle() }
                            text { content: controller_loopback_addr.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                        if controller_allow_lan_value {
                            column {
                                width: "100%",
                                margin_top: 12.0,
                                text { content: tr(current.locale, "局域网访问", "LAN endpoint"), font_size: typography::XS, font_color: subtle() }
                                text { content: controller_lan_description.clone(), margin_top: 4.0, font_size: typography::SM, font_color: text_color() }
                                text { content: "Authorization: Bearer <secret>", margin_top: 4.0, font_size: typography::XS, font_color: subtle() }
                            }
                            row {
                                width: "100%",
                                margin_top: 12.0,
                                align_items: "center",
                                column {
                                    width: "72%",
                                    text { content: tr(current.locale, "访问密钥", "Access secret"), font_size: typography::XS, font_color: subtle() }
                                    text { content: controller_secret_label.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                                }
                                if let Some(secret) = controller_secret.clone() {
                                    FlatButton {
                                        onclick: move |_| copy_controller_secret(state, secret.clone()),
                                        {arkit::icon("copy", 16.0, text_color())}
                                        text { content: tr(current.locale, "复制", "Copy"), margin_left: 6.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
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
                            text { content: tr(current.locale, "保存网络设置", "Save network settings"), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                "DNS",
                Some(tr(current.locale, "每行一个地址，策略格式为 matcher = upstream", "One address per line; policy uses matcher = upstream").to_owned()),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        FormItem {
                            label: tr(current.locale, "上游 DNS", "Upstream DNS").to_owned(),
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
                            label: tr(current.locale, "分流策略", "Nameserver policy").to_owned(),
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
                            text { content: tr(current.locale, "保存 DNS 设置", "Save DNS settings"), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
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
            Ok(Ok(())) => tr(
                state.read().locale,
                "访问密钥已复制",
                "Access secret copied",
            )
            .to_owned(),
            Ok(Err(error)) => format!(
                "{}{}",
                tr(state.read().locale, "复制失败：", "Copy failed: "),
                error
            ),
            Err(error) => format!(
                "{}{}",
                tr(state.read().locale, "复制任务失败：", "Copy task failed: "),
                error
            ),
        };
        state.read().notifications.publish(message);
    });
}
