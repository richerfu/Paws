use super::super::*;

pub(crate) fn resources_page(state: Signal<State>) -> Element {
    let mut query = use_signal(String::new);
    let mut geodata_detail = use_signal(|| None::<hmeta_model::GeodataFileSummary>);
    let mut provider_detail = use_signal(|| None::<String>);
    let current = state.read().clone();
    let query_value = query();
    let active_profile_name = current
        .snapshot
        .profiles
        .iter()
        .find(|profile| profile.active)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| tr(current.locale, "未选择", "Unselected").to_owned());
    let enabled_rule_count = current
        .snapshot
        .rules
        .iter()
        .filter(|rule| rule.enabled)
        .count();
    let total_rule_count = current.snapshot.rules.len();
    let total_provider_count = current.snapshot.providers.len();
    let ready_geodata_count = current
        .snapshot
        .geodata
        .iter()
        .filter(|file| file.exists)
        .count();
    let total_geodata_count = current.snapshot.geodata.len();
    let mode_label = match current.snapshot.mode {
        RuntimeMode::Rule => tr(current.locale, "规则", "Rule"),
        RuntimeMode::Global => tr(current.locale, "全局", "Global"),
        RuntimeMode::Direct => tr(current.locale, "直连", "Direct"),
    };
    let providers = current.snapshot.providers.iter()
        .filter(|provider| matches_provider_query(provider, &query_value))
        .cloned()
        .map(|provider| {
            let refresh_provider_type = provider.provider_type.clone();
            let refresh_provider_name = provider.name.clone();
            let health_provider_name = provider.name.clone();
            let detail_provider_name = provider.name.clone();
            let member_count = provider.members.len();
            let alive_count = provider.members.iter().filter(|member| member.alive).count();
            let can_healthcheck = provider.provider_type == "proxy"
                && provider.health_check_enabled;
            let provider_status = if provider.last_refresh_error.is_some() {
                tr(current.locale, "刷新失败", "Refresh failed")
            } else if provider.vehicle_type.as_deref().is_some_and(|kind| kind.eq_ignore_ascii_case("inline")) {
                tr(current.locale, "内置已加载", "Inline loaded")
            } else if provider.cache_exists {
                tr(current.locale, "缓存已加载", "Cache loaded")
            } else {
                tr(current.locale, "等待缓存", "Cache pending")
            };
            rsx! {
                {card(
                    truncate_text(&provider.name, 38),
                    Some(format!("{} · {}", provider.provider_type, provider.vehicle_type.clone().unwrap_or_default())),
                    rsx! {
                        column {
                            width: "100%",
                            {info_row(tr(current.locale, "状态", "Status"), provider_status)}
                            {info_row(tr(current.locale, "缓存", "Cache"), if provider.cache_exists { format_total(provider.cache_bytes.unwrap_or(0)) } else { tr(current.locale, "无", "None").to_owned() })}
                            {info_row(tr(current.locale, "刷新间隔", "Interval"), provider.interval_seconds.map(|value| format!("{value}s")).unwrap_or_else(|| "-".to_owned()))}
                            if provider.provider_type == "proxy" {
                                {info_row(tr(current.locale, "成员健康", "Member health"), format!("{alive_count}/{member_count}"))}
                            }
                            if let Some(error) = provider.last_refresh_error.clone() {
                                text { content: compact(&error), margin_top: 6.0, font_size: 12.0, font_color: danger(), max_lines: 2 }
                            }
                            row { height: 4.0 }
                            row {
                                width: "100%",
                                justify_content: "end",
                                FlatButton {
                                    variant: FlatButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| provider_detail.set(Some(detail_provider_name.clone())),
                                    {arkit::icon("list", 14.0, text_color())}
                                    text { content: tr(current.locale, "详情", "Details"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                }
                                if can_healthcheck {
                                    FlatButton {
                                        variant: FlatButtonVariant::Ghost,
                                        size: ButtonSize::Sm,
                                        disabled: Some(current.controller_diagnostic_pending.is_some()),
                                        onclick: move |_| dispatch(state, Action::HealthcheckProxyProvider {
                                            provider_name: health_provider_name.clone(),
                                        }),
                                        {arkit::icon("heart-pulse", 14.0, text_color())}
                                        text { content: tr(current.locale, "检查", "Check"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                    }
                                }
                                FlatButton {
                                    variant: FlatButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| dispatch(state, Action::RefreshProvider {
                                        provider_type: refresh_provider_type.clone(),
                                        provider_name: refresh_provider_name.clone(),
                                    }),
                                    {arkit::icon("refresh-cw", 14.0, text_color())}
                                    text { content: tr(current.locale, "刷新", "Refresh"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                }
                            }
                        }
                    }
                )}
            }
        }).collect::<Vec<_>>();
    let rules = current
        .snapshot
        .rules
        .iter()
        .filter(|rule| matches_rule_query(rule, &query_value))
        .cloned()
        .map(|rule| rule_view(state, &current, rule))
        .collect::<Vec<_>>();
    let geodata = current.snapshot.geodata.iter()
        .filter(|file| matches_geodata_query(file, &query_value))
        .cloned()
        .enumerate()
        .map(|(index, file)| {
            let detail = file.clone();
            let status = if file.exists {
                tr(current.locale, "可用", "Available")
            } else {
                tr(current.locale, "缺失", "Missing")
            };
            let metadata = if file.exists {
                format!("{status} · {}", format_total(file.bytes.unwrap_or(0)))
            } else {
                status.to_owned()
            };
            rsx! {
                if index > 0 {
                    Separator {}
                }
                button {
                    width: "100%",
                    height: 68.0,
                    padding_left: 14.0,
                    padding_right: 12.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| geodata_detail.set(Some(detail.clone())),
                    row {
                        width: "100%",
                        align_items: "center",
                        row {
                            width: 36.0,
                            height: 36.0,
                            align_items: "center",
                            justify_content: "center",
                            background_color: muted(),
                            border_radius: 9.0,
                            {arkit::icon("file-text", 17.0, if file.exists { success() } else { danger() })}
                        }
                        column {
                            layout_weight: 1.0,
                            margin_left: 11.0,
                            align_items: "start",
                            text { content: file.name, width: "100%", font_size: 13.0, font_weight: 650, font_color: text_color(), max_lines: 1 }
                            text { content: metadata, width: "100%", margin_top: 3.0, font_size: 11.0, font_color: if file.exists { success() } else { danger() }, max_lines: 1 }
                        }
                        {arkit::icon("chevron-right", 15.0, subtle())}
                    }
                }
            }
        }).collect::<Vec<_>>();
    let visible_geodata_count = geodata.len();
    let selected_geodata = geodata_detail();
    let selected_provider = provider_detail().and_then(|name| {
        current
            .snapshot
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .cloned()
    });
    let body = rsx! {
        column {
            width: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).resources_search_placeholder.to_owned()),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row { height: 12.0 }
            {card(
                tr(current.locale, "规则运行状态", "Rules runtime"),
                Some(active_profile_name),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "引擎配置", "Engine config"), if current.snapshot.engine_loaded { tr(current.locale, "已加载", "Loaded") } else { tr(current.locale, "未加载", "Not loaded") })}
                        {info_row(tr(current.locale, "当前模式", "Current mode"), mode_label)}
                        {info_row(tr(current.locale, "生效规则", "Effective rules"), format!("{enabled_rule_count}/{total_rule_count}"))}
                        {info_row("Provider", total_provider_count.to_string())}
                        {info_row("GeoData", format!("{ready_geodata_count}/{total_geodata_count}"))}
                    }
                }
            )}
            row { height: 12.0 }
            column {
                width: "100%",
                background_color: surface(),
                border_width: 1.0,
                border_color: line(),
                border_radius: 10.0,
                clip: true,
                row {
                    width: "100%",
                    height: 56.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    align_items: "center",
                    text { content: "GeoData", font_size: 14.0, font_weight: 700, font_color: text_color() }
                    row { layout_weight: 1.0 }
                    text {
                        content: format!("{ready_geodata_count}/{total_geodata_count} {}", tr(current.locale, "可用", "ready")),
                        font_size: 11.0,
                        font_weight: 600,
                        font_color: if ready_geodata_count == total_geodata_count && total_geodata_count > 0 { success() } else { warning() },
                    }
                }
                Separator {}
                if visible_geodata_count == 0 {
                    row {
                        width: "100%",
                        height: 66.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        align_items: "center",
                        text { content: tr(current.locale, "没有匹配的 GeoData 文件", "No matching GeoData files"), font_size: 12.0, font_color: subtle() }
                    }
                } else {
                    {geodata.into_iter()}
                }
            }
            row { height: 12.0 }
            {section_label(tr(current.locale, "Provider", "Providers"))}
            if providers.is_empty() {
                {empty_state("database", tr(current.locale, "当前订阅没有 Provider", "No providers in this profile"), tr(current.locale, "分享链接订阅通常只包含节点；Provider 需由 Clash YAML 显式声明", "Share-link subscriptions usually contain nodes only; providers must be declared by Clash YAML"))}
            } else {
                {spaced(providers)}
            }
            row { height: 14.0 }
            row {
                width: "100%",
                height: 34.0,
                margin_bottom: 8.0,
                align_items: "center",
                text { content: strings(current.locale).resources_rules_title, font_size: 15.0, font_weight: 750, font_color: text_color() }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(current.rule_import_loading || current.snapshot.active_profile.is_none()),
                    onclick: move |_| {
                        if !state.read().rule_import_loading {
                            dispatch(state, Action::ImportRules);
                        }
                    },
                    if current.rule_import_loading {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("file-up", 14.0, text_color())}
                    }
                    text {
                        content: strings(current.locale).resources_import_rules,
                        margin_left: 5.0,
                        font_size: 12.0,
                        font_weight: 650,
                        font_color: text_color(),
                    }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(current.snapshot.active_profile.is_none()),
                    onclick: move |_| dispatch(state, Action::OpenManualRuleEditor {
                        connection_id: None,
                        domain: String::new(),
                        destination_ip: String::new(),
                    }),
                    {arkit::icon("plus", 14.0, text_color())}
                    text { content: tr(current.locale, "添加", "Add"), margin_left: 5.0, font_size: 12.0, font_weight: 650, font_color: text_color() }
                }
            }
            if rules.is_empty() {
                {empty_state("list-checks", tr(current.locale, "当前配置没有可编辑规则", "No editable rules"), tr(current.locale, "请确认已选择订阅并完成配置加载", "Select a profile and wait for configuration loading"))}
            } else {
                {compact_rule_list(rules)}
            }
        }
    };
    let actions = rsx! {
        row {
            {icon_action("route", Action::OpenRuleLookup, state)}
            {icon_action("refresh-cw", Action::RefreshAllProviders, state)}
        }
    };
    let page = scaffold(state, Route::Resources {}, actions, body);
    rsx! {
        {page}
        if let Some(file) = selected_geodata {
            {geodata_detail_dialog(current.locale, file, geodata_detail)}
        }
        if let Some(provider) = selected_provider {
            {provider_detail_dialog(
                state,
                current.locale,
                provider,
                provider_detail,
                current.controller_diagnostic_pending.is_some(),
            )}
        }
        if current.manual_rule_editor.is_some() {
            {manual_rule_dialog(state, &current)}
        }
        if current.rule_lookup.is_some() {
            {rule_lookup_dialog(state, &current)}
        }
    }
}

fn rule_lookup_dialog(state: Signal<State>, current: &State) -> Element {
    let open = current.rule_lookup.is_some();
    let content_key = current
        .rule_lookup
        .as_ref()
        .map(|lookup| {
            dialog_content_key(&[
                &lookup.id.to_string(),
                &lookup.query,
                &lookup.submitting.to_string(),
                &format!("{:?}", lookup.result),
                lookup.error.as_deref().unwrap_or(""),
            ])
        })
        .unwrap_or(0);
    rsx! {
        FlatDialog {
            open,
            content_key,
            on_close: move |_| dispatch(state, Action::CloseRuleLookup),
            RuleLookupDialogContent { state }
        }
    }
}

#[component]
fn RuleLookupDialogContent(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let Some(lookup) = current.rule_lookup.clone() else {
        return rsx! {};
    };
    let locale = current.locale;
    let can_lookup = !lookup.submitting
        && !lookup.query.trim().is_empty()
        && current.snapshot.active_profile.is_some();

    rsx! {
        DialogHeader {
            title: tr(locale, "规则查询", "Rule lookup").to_owned(),
            description: Some(tr(locale, "输入域名或 IP，查看当前配置的首条命中规则", "Enter a domain or IP to inspect the first matching rule in the active profile").to_owned()),
        }
        row { height: 14.0 }
        Input {
            value: Some(lookup.query.clone()),
            placeholder: Some("example.com / 203.0.113.1".to_owned()),
            width: Some("100%".into()),
            disabled: lookup.submitting,
            on_change: move |value| dispatch(state, Action::SetRuleLookupQuery(value)),
        }
        if current.snapshot.active_profile.is_none() {
            text {
                content: tr(locale, "请先启用一个订阅配置，再查询规则。", "Activate a profile before querying rules."),
                margin_top: 9.0,
                font_size: 11.0,
                line_height: 16.0,
                font_color: warning(),
            }
        }
        if current.snapshot.mode != RuntimeMode::Rule {
            text {
                content: tr(locale, "这里展示规则模式下的匹配结果；当前 Global / Direct 模式不会采用该规则。", "This shows the Rule-mode result; the current Global / Direct mode does not use this rule."),
                margin_top: 9.0,
                font_size: 11.0,
                line_height: 16.0,
                font_color: warning(),
            }
        }
        if let Some(error) = lookup.error {
            text {
                content: error,
                margin_top: 9.0,
                font_size: 11.0,
                line_height: 16.0,
                font_color: danger(),
            }
        }
        if let Some(result) = lookup.result {
            column {
                width: "100%",
                margin_top: 12.0,
                padding: 12.0,
                border_width: 1.0,
                border_color: if result.matched { success() } else { line() },
                border_radius: 8.0,
                background_color: muted(),
                row {
                    width: "100%",
                    align_items: "center",
                    {arkit::icon(if result.matched { "route" } else { "x" }, 16.0, if result.matched { success() } else { subtle() })}
                    text {
                        content: if result.matched { tr(locale, "命中规则", "Rule matched") } else { tr(locale, "未命中规则", "No rule matched") },
                        margin_left: 7.0,
                        font_size: 13.0,
                        font_weight: 700,
                        font_color: if result.matched { success() } else { text_color() },
                    }
                }
                if let Some(rule_line) = result.rule_line {
                    text {
                        content: rule_line,
                        width: "100%",
                        margin_top: 8.0,
                        font_size: 12.0,
                        line_height: 18.0,
                        font_weight: 600,
                        font_color: text_color(),
                        max_lines: 3,
                        text_overflow: "ellipsis",
                    }
                } else {
                    text {
                        content: tr(locale, "没有规则命中，默认使用 DIRECT。", "No rule matched; DIRECT is used by default."),
                        width: "100%",
                        margin_top: 8.0,
                        font_size: 12.0,
                        line_height: 18.0,
                        font_color: subtle(),
                    }
                }
                row { height: 7.0 }
                {info_row(
                    tr(locale, "输入类型", "Input type"),
                    match result.input_kind {
                        hmeta_core::RuleLookupInputKind::Domain => tr(locale, "域名", "Domain"),
                        hmeta_core::RuleLookupInputKind::Ip => "IP",
                    },
                )}
                if result.resolution_attempted {
                    {info_row(
                        tr(locale, "解析 IP", "Resolved IP"),
                        result.resolved_ip.unwrap_or_else(|| tr(locale, "未解析", "Unresolved").to_owned()),
                    )}
                }
                {info_row(tr(locale, "目标策略", "Target policy"), result.target)}
                row { height: 8.0 }
                FlatButton {
                    variant: FlatButtonVariant::Outline,
                    width: "100%",
                    onclick: move |_| dispatch(state, Action::AddRuleFromLookup),
                    {arkit::icon("plus", 15.0, text_color())}
                    text {
                        content: tr(locale, "新增当前输入规则", "Add rule for this input"),
                        margin_left: 7.0,
                        font_size: 12.0,
                        font_weight: 650,
                        font_color: text_color(),
                    }
                }
            }
        }
        DialogFooter {
            FlatButton {
                variant: FlatButtonVariant::Primary,
                width: "100%",
                disabled: Some(!can_lookup),
                onclick: move |_| dispatch(state, Action::LookupRule),
                if lookup.submitting {
                    Spinner { size: 16.0, color: Some(primary_text()) }
                } else {
                    {arkit::icon("search", 16.0, primary_text())}
                }
                text {
                    content: if lookup.submitting { tr(locale, "正在查询", "Looking up") } else { tr(locale, "查询规则", "Look up rule") },
                    margin_left: 8.0,
                    font_size: 13.0,
                    font_weight: 650,
                    font_color: primary_text(),
                }
            }
        }
    }
}

fn provider_detail_dialog(
    state: Signal<State>,
    locale: UiLocale,
    provider: hmeta_model::ProviderSummary,
    mut selected: Signal<Option<String>>,
    pending: bool,
) -> Element {
    let provider_name = provider.name.clone();
    let health_url = provider
        .health_check_url
        .clone()
        .unwrap_or_else(|| "https://www.gstatic.com/generate_204".to_owned());
    let expected_status = provider.expected_status.clone();
    let members = provider.members.into_iter().map(|member| {
        let check_provider = provider_name.clone();
        let check_proxy = member.name.clone();
        let check_url = health_url.clone();
        let check_expected = expected_status.clone();
        let status = if member.alive {
            tr(locale, "可用", "Alive")
        } else {
            tr(locale, "不可用", "Unavailable")
        };
        let delay = member
            .delay_ms
            .map(|delay| format!("{delay} ms"))
            .unwrap_or_else(|| tr(locale, "未测试", "Untested").to_owned());
        rsx! {
            row {
                width: "100%",
                height: 50.0,
                padding_left: 10.0,
                padding_right: 8.0,
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text { content: truncate_text(&member.name, 34), width: "100%", font_size: 12.0, font_weight: 650, font_color: text_color(), max_lines: 1 }
                    text { content: format!("{} · {} · {}", member.proxy_type, status, delay), margin_top: 3.0, width: "100%", font_size: 10.0, font_color: if member.alive { success() } else { danger() }, max_lines: 1 }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Icon,
                    disabled: Some(pending),
                    onclick: move |_| dispatch(state, Action::HealthcheckProviderProxy {
                        provider_name: check_provider.clone(),
                        proxy_name: check_proxy.clone(),
                        url: check_url.clone(),
                        expected_status: check_expected.clone(),
                    }),
                    {arkit::icon("gauge", 15.0, text_color())}
                }
            }
            Separator {}
        }
    }).collect::<Vec<_>>();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: truncate_text(&provider_name, 42),
                description: Some(format!("{} {}", members.len(), tr(locale, "个成员", "members"))),
            }
            row { height: 12.0 }
            if members.is_empty() {
                text { content: tr(locale, "当前 Provider 没有可展示的成员", "No provider members available"), font_size: 12.0, font_color: subtle() }
            } else {
                column {
                    width: "100%",
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 9.0,
                    clip: true,
                    {members.into_iter()}
                }
            }
        }
    }
}

fn geodata_detail_dialog(
    locale: UiLocale,
    file: hmeta_model::GeodataFileSummary,
    mut selected: Signal<Option<hmeta_model::GeodataFileSummary>>,
) -> Element {
    let availability = if file.exists {
        tr(locale, "文件可用", "File available")
    } else {
        tr(locale, "文件缺失", "File missing")
    };
    let size = file
        .bytes
        .map(format_total)
        .unwrap_or_else(|| "-".to_owned());
    let updated_at = file
        .updated_at
        .as_deref()
        .and_then(time_format::format_unix_seconds)
        .or(file.updated_at.clone())
        .unwrap_or_else(|| "-".to_owned());
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: file.name,
                description: Some(availability.to_owned()),
            }
            row { height: 16.0 }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: 9.0,
                padding_left: 12.0,
                padding_right: 12.0,
                {info_row(tr(locale, "状态", "Status"), availability)}
                Separator {}
                {info_row(tr(locale, "文件大小", "File size"), size)}
                Separator {}
                {info_row(tr(locale, "更新时间", "Updated"), updated_at)}
            }
            row { height: 14.0 }
            text { content: tr(locale, "文件位置", "File location"), font_size: 11.0, font_weight: 650, font_color: subtle() }
            row { height: 6.0 }
            row {
                width: "100%",
                padding: 11.0,
                background_color: muted(),
                border_radius: 8.0,
                text {
                    content: file.path,
                    width: "100%",
                    font_size: 11.0,
                    line_height: 17.0,
                    font_color: text_color(),
                    max_lines: 5,
                }
            }
        }
    }
}

fn rule_view(state: Signal<State>, current: &State, rule: hmeta_model::RuleSummary) -> Element {
    let editable = rule.source != "profile-yaml";
    let rule_source = if editable {
        rule.source.clone()
    } else {
        tr(
            current.locale,
            "订阅配置 · 已载入运行时",
            "Profile YAML · loaded at runtime",
        )
        .to_owned()
    };
    let toggle_profile = rule.profile_id.clone();
    let toggle_id = rule.id.clone();
    let delete_profile = rule.profile_id.clone();
    let delete_id = rule.id.clone();
    let enabled = rule.enabled;
    let up = reordered_rule_ids(&current.snapshot.rules, &rule.profile_id, &rule.id, -1);
    let down = reordered_rule_ids(&current.snapshot.rules, &rule.profile_id, &rule.id, 1);
    let toggle_action = Action::SetRuleEnabled {
        profile_id: toggle_profile,
        rule_id: toggle_id,
        enabled: !enabled,
    };
    let delete_action = Action::DeleteRule {
        profile_id: delete_profile,
        rule_id: delete_id,
    };
    rsx! {
        column {
            width: "100%",
            height: 88.0,
            padding_top: 8.0,
            padding_right: 8.0,
            padding_bottom: 8.0,
            padding_left: 10.0,
            background_color: surface(),
            border_width: 1.0,
            border_color: line(),
            border_radius: 8.0,
            clip: true,
            row {
                width: "100%",
                height: 32.0,
                align_items: "center",
                text {
                    content: format!("#{}", rule.order + 1),
                    font_size: 11.0,
                    font_weight: 700,
                    font_color: if enabled { success() } else { subtle() },
                    max_lines: 1,
                }
                row {
                    layout_weight: 1.0,
                    margin_left: 7.0,
                    margin_right: 4.0,
                    text {
                        content: rule_source,
                        width: "100%",
                        font_size: 10.0,
                        font_color: subtle(),
                        max_lines: 1,
                    }
                }
                if editable {
                    {compact_rule_action(if enabled { "toggle-right" } else { "toggle-left" }, if enabled { success() } else { subtle() }, toggle_action, state)}
                    if let Some(ids) = up {
                        {compact_rule_action("arrow-up", subtle(), Action::ReorderRules { profile_id: rule.profile_id.clone(), ordered_rule_ids: ids }, state)}
                    }
                    if let Some(ids) = down {
                        {compact_rule_action("arrow-down", subtle(), Action::ReorderRules { profile_id: rule.profile_id.clone(), ordered_rule_ids: ids }, state)}
                    }
                    {compact_rule_action("trash-2", danger(), delete_action, state)}
                } else {
                    {pill(tr(current.locale, "运行中", "Effective").to_owned(), success())}
                }
            }
            text {
                content: truncate_text(&rule.line, 180),
                width: "100%",
                margin_top: 5.0,
                font_size: 11.0,
                line_height: 16.0,
                font_color: text_color(),
                max_lines: 2,
            }
        }
    }
}

fn compact_rule_action(
    icon: &'static str,
    color: u32,
    action: Action,
    state: Signal<State>,
) -> Element {
    rsx! {
        button {
            width: 32.0,
            height: 32.0,
            padding: 0.0,
            background_color: surface(),
            border_width: 0.0,
            border_radius: 7.0,
            onclick: move |_| dispatch(state, action.clone()),
            row {
                width: "100%",
                height: "100%",
                align_items: "center",
                justify_content: "center",
                {arkit::icon(icon, 15.0, color)}
            }
        }
    }
}

fn compact_rule_list(items: Vec<Element>) -> Element {
    let len = items.len();
    let nodes = items.into_iter().enumerate().map(|(index, item)| {
        rsx! {
            {item}
            if index + 1 < len { row { height: 6.0 } }
        }
    });
    rsx! { column { width: "100%", {nodes} } }
}

fn reordered_rule_ids(
    rules: &[hmeta_model::RuleSummary],
    profile_id: &str,
    rule_id: &str,
    delta: isize,
) -> Option<Vec<String>> {
    let mut ordered = rules
        .iter()
        .filter(|rule| rule.profile_id == profile_id && rule.source != "profile-yaml")
        .collect::<Vec<_>>();
    ordered.sort_by_key(|rule| rule.order);
    let index = ordered.iter().position(|rule| rule.id == rule_id)?;
    let target = index.checked_add_signed(delta)?;
    if target >= ordered.len() {
        return None;
    }
    ordered.swap(index, target);
    Some(ordered.into_iter().map(|rule| rule.id.clone()).collect())
}
