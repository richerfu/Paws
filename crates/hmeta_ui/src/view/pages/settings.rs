use super::super::*;

pub(crate) fn settings_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let (initial_dns_servers, initial_dns_fallbacks, initial_dns_policy) =
        dns_draft_from_snapshot(&current.snapshot);
    let (initial_system_proxy, initial_dns_hijacking, initial_allow_bypass, initial_stack) =
        vpn_draft_from_snapshot(&current.snapshot);

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
            percent_width: 1.0,
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
                            Input {
                                value: Some(vpn_stack_value.clone()),
                                percent_width: Some(1.0),
                                on_change: move |value| vpn_stack.set(value),
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            percent_width: Some(1.0),
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
                                percent_width: Some(1.0),
                                on_change: move |value| dns_servers.set(value),
                            }
                        }
                        FormItem {
                            label: "Fallback".to_owned(),
                            Textarea {
                                value: Some(dns_fallbacks_value.clone()),
                                height: Some(76.0),
                                percent_width: Some(1.0),
                                on_change: move |value| dns_fallbacks.set(value),
                            }
                        }
                        FormItem {
                            label: tr(current.locale, "分流策略", "Nameserver policy").to_owned(),
                            Textarea {
                                value: Some(dns_policy_value.clone()),
                                height: Some(104.0),
                                percent_width: Some(1.0),
                                on_change: move |value| dns_policy.set(value),
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            percent_width: Some(1.0),
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

pub(crate) fn per_app_settings_page(state: Signal<State>) -> Element {
    let mut installed_query = use_signal(String::new);
    let mut load_requested = use_signal(|| false);
    let current = state.read().clone();
    let (initial_mode, initial_trusted, initial_blocked) =
        per_app_draft_from_snapshot(&current.snapshot);
    let initial_picker_mode = if initial_mode == PerAppMode::Bypass {
        PerAppMode::Bypass
    } else {
        PerAppMode::Proxy
    };
    let mut selected_mode = use_signal(move || initial_picker_mode);
    let mut trusted_text = use_signal({
        let value = initial_trusted.clone();
        move || value
    });
    let mut blocked_text = use_signal({
        let value = initial_blocked.clone();
        move || value
    });

    use_effect(move || {
        if !load_requested() {
            load_requested.set(true);
            dispatch(state, Action::RefreshInstalledApplications);
        }
    });

    let query_value = installed_query();
    let mode_value = selected_mode();
    let trusted_value = trusted_text();
    let blocked_value = blocked_text();
    let trusted = parse_applications_text(&trusted_value);
    let blocked = parse_applications_text(&blocked_value);
    let selected_count = match mode_value {
        PerAppMode::Bypass => blocked.len(),
        _ => trusted.len(),
    };
    let save_mode = if selected_count == 0 {
        PerAppMode::Off
    } else {
        mode_value
    };
    let dirty = save_mode != initial_mode
        || trusted_value != initial_trusted
        || blocked_value != initial_blocked;

    let visible_apps = current
        .installed_applications
        .iter()
        .filter(|app| matches_installed_application_query(app, &query_value))
        .take(80)
        .cloned()
        .collect::<Vec<_>>();
    let visible_names = visible_apps
        .iter()
        .map(|app| app.bundle_name.clone())
        .collect::<Vec<_>>();
    let select_names = visible_names.clone();
    let deselect_names = visible_names;
    let select_source = if mode_value == PerAppMode::Bypass {
        blocked_value.clone()
    } else {
        trusted_value.clone()
    };
    let deselect_source = select_source.clone();
    let app_rows = visible_apps
        .into_iter()
        .enumerate()
        .map(|(index, app)| {
            let bundle = app.bundle_name.clone();
            let is_selected = if mode_value == PerAppMode::Bypass {
                blocked.contains(&bundle)
            } else {
                trusted.contains(&bundle)
            };
            let trusted_source = trusted_value.clone();
            let blocked_source = blocked_value.clone();
            rsx! {
                column {
                    key: "{bundle}",
                    percent_width: 1.0,
                    row {
                        percent_width: 1.0,
                        height: 64.0,
                        padding_left: 2.0,
                        padding_right: 2.0,
                        align_items: "center",
                        row {
                            width: 36.0,
                            height: 36.0,
                            align_items: "center",
                            justify_content: "center",
                            background_color: muted(),
                            border_radius: 9.0,
                            {arkit::icon("app-window", 17.0, subtle())}
                        }
                        column {
                            layout_weight: 1.0,
                            margin_left: 10.0,
                            text { content: truncate_text(&app.name, 32), font_size: 14.0, font_weight: 600, font_color: text_color(), max_lines: 1 }
                            text { content: truncate_text(&app.bundle_name, 44), margin_top: 3.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
                        }
                        Checkbox {
                            checked: Some(is_selected),
                            on_change: move |checked| {
                                if mode_value == PerAppMode::Bypass {
                                    let next = if checked {
                                        add_application_to_text(&blocked_source, &bundle)
                                    } else {
                                        remove_application_from_text(&blocked_source, &bundle)
                                    };
                                    blocked_text.set(next);
                                    if checked {
                                        trusted_text.set(remove_application_from_text(&trusted_source, &bundle));
                                    }
                                } else {
                                    let next = if checked {
                                        add_application_to_text(&trusted_source, &bundle)
                                    } else {
                                        remove_application_from_text(&trusted_source, &bundle)
                                    };
                                    trusted_text.set(next);
                                    if checked {
                                        blocked_text.set(remove_application_from_text(&blocked_source, &bundle));
                                    }
                                }
                            },
                        }
                    }
                    if index + 1 < current.installed_applications.len() {
                        Separator {}
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let proxy = tr(current.locale, "代理所选", "Proxy selected").to_owned();
    let bypass = tr(current.locale, "绕过所选", "Bypass selected").to_owned();
    let selected_label = if mode_value == PerAppMode::Bypass {
        bypass.clone()
    } else {
        proxy.clone()
    };
    let body = rsx! {
        column {
            percent_width: 1.0,
            FlatSegmented {
                options: vec![proxy.clone(), bypass.clone()],
                selected: selected_label,
                on_change: move |value: String| {
                    selected_mode.set(if value == bypass { PerAppMode::Bypass } else { PerAppMode::Proxy });
                },
            }
            text {
                content: if selected_count == 0 {
                    tr(current.locale, "未选择应用时自动关闭分应用代理", "Per-app proxy is disabled when no apps are selected")
                } else {
                    tr(current.locale, "保存后运行中的 VPN 会自动请求重连", "A running VPN reconnects after saving")
                },
                margin_top: 8.0,
                font_size: 12.0,
                line_height: 18.0,
                font_color: subtle(),
            }
            row { height: 14.0 }
            Input {
                value: Some(query_value.clone()),
                placeholder: Some(tr(current.locale, "搜索应用或包名", "Search apps or bundles").to_owned()),
                percent_width: Some(1.0),
                on_change: move |value| installed_query.set(value),
            }
            row {
                percent_width: 1.0,
                height: 48.0,
                align_items: "center",
                text {
                    content: format!("{} {}", selected_count, tr(current.locale, "个应用", "selected")),
                    font_size: 12.0,
                    font_color: subtle(),
                }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(select_names.is_empty()),
                    onclick: move |_| {
                        let mut next = select_source.clone();
                        for bundle in &select_names {
                            next = add_application_to_text(&next, bundle);
                        }
                        if mode_value == PerAppMode::Bypass { blocked_text.set(next); } else { trusted_text.set(next); }
                    },
                    text { content: tr(current.locale, "全选", "Select all"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(deselect_names.is_empty()),
                    onclick: move |_| {
                        let mut next = deselect_source.clone();
                        for bundle in &deselect_names {
                            next = remove_application_from_text(&next, bundle);
                        }
                        if mode_value == PerAppMode::Bypass { blocked_text.set(next); } else { trusted_text.set(next); }
                    },
                    text { content: tr(current.locale, "取消全选", "Deselect all"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                }
            }
            if current.installed_applications_loading {
                {empty_state("loader-circle", tr(current.locale, "正在读取应用", "Loading applications"), tr(current.locale, "请稍候…", "Please wait…"))}
            } else if let Some(error) = current.installed_applications_error.clone() {
                {empty_state("triangle-alert", tr(current.locale, "无法读取应用", "Unable to load apps"), compact(&error))}
            } else if app_rows.is_empty() {
                if query_value.trim().is_empty() {
                    {empty_state("layout-grid", tr(current.locale, "未发现可选择的应用", "No applications available"), tr(current.locale, "刷新列表或检查应用读取权限", "Refresh or check application access"))}
                } else {
                    {empty_state("layout-grid", tr(current.locale, "没有匹配的应用", "No matching apps"), tr(current.locale, "调整搜索词后重试", "Try a different search"))}
                }
            } else {
                column {
                    percent_width: 1.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 10.0,
                    clip: true,
                    {app_rows.into_iter()}
                }
            }
        }
    };
    let trusted_submit = trusted_value;
    let blocked_submit = blocked_value;
    let actions = rsx! {
        row {
            {icon_action("refresh-cw", Action::RefreshInstalledApplications, state)}
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                disabled: Some(!dirty),
                onclick: move |_| dispatch(state, Action::SavePerAppSettings {
                    mode: save_mode,
                    trusted_applications_text: trusted_submit.clone(),
                    blocked_applications_text: blocked_submit.clone(),
                }),
                {arkit::icon("check", 18.0, if dirty { text_color() } else { subtle() })}
            }
        }
    };
    scaffold(state, Route::PerApp {}, actions, body)
}
