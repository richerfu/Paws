use super::super::*;

pub(crate) fn settings_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let (initial_dns_servers, initial_dns_fallbacks, initial_dns_policy) =
        dns_draft_from_snapshot(&current.snapshot);
    let (initial_system_proxy, initial_dns_hijacking, initial_allow_bypass, initial_stack) =
        vpn_draft_from_snapshot(&current.snapshot);
    let stack_options = vec![
        FlatSelectOption {
            value: hmeta_model::VpnStack::Smoltcp.as_str().to_owned(),
            label: "smoltcp".to_owned(),
            description: tr(
                current.locale,
                "纯 Rust，兼容性优先",
                "Pure Rust, compatibility first",
            )
            .to_owned(),
        },
        FlatSelectOption {
            value: hmeta_model::VpnStack::Lwip.as_str().to_owned(),
            label: "lwIP".to_owned(),
            description: tr(
                current.locale,
                "成熟 C 协议栈，适合高并发连接",
                "Mature C stack for concurrent connections",
            )
            .to_owned(),
        },
    ];

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
    let mut vpn_stack = use_signal({
        let value = initial_stack.clone();
        move || value
    });

    let dns_servers_value = dns_servers();
    let dns_fallbacks_value = dns_fallbacks();
    let dns_policy_value = dns_policy();
    let system_proxy_value = system_proxy();
    let dns_hijacking_value = dns_hijacking();
    let allow_bypass_value = allow_bypass();
    let vpn_stack_value = vpn_stack();
    let vpn_dirty = system_proxy_value != initial_system_proxy
        || dns_hijacking_value != initial_dns_hijacking
        || allow_bypass_value != initial_allow_bypass
        || vpn_stack_value != initial_stack;
    let dns_dirty = dns_servers_value != initial_dns_servers
        || dns_fallbacks_value != initial_dns_fallbacks
        || dns_policy_value != initial_dns_policy;

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
                            FlatSelect {
                                options: stack_options,
                                selected: vpn_stack_value.clone(),
                                on_change: move |value| vpn_stack.set(value),
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
