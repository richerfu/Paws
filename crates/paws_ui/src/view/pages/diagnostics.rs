use super::super::*;

pub(crate) fn diagnostics_page() -> Element {
    let tasks = use_page_tasks();
    let export_tasks = tasks.clone();
    let probe_tasks = tasks.clone();
    let backup_tasks = tasks.clone();
    let restore_tasks = tasks.clone();
    let upload_tasks = tasks.clone();
    let download_tasks = tasks.clone();
    let qualification_tasks = tasks.clone();
    let services = use_context::<UiServices>();
    let watch_services = services.clone();
    let disable_services = services.clone();
    let dns_services = services.clone();
    let export_services = services.clone();
    let backup_services = services.clone();
    let restore_services = services.clone();
    let upload_services = services.clone();
    let download_services = services.clone();
    let qualification_services = services.clone();
    let report_services = services.clone();
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let session = stores.session.read().clone();
    let telemetry = stores.telemetry.read().clone();
    let settings = stores.settings.read().clone();
    let groups = stores.proxies.read().groups.clone();
    let mut watch_state = use_signal(|| {
        paws_core::shared_core()
            .service_watch_policy()
            .unwrap_or_default()
    });
    let initial_watch = watch_state.peek().clone();
    let initial_url = if initial_watch.url.is_empty() {
        "https://www.gstatic.com/generate_204".to_owned()
    } else {
        initial_watch.url.clone()
    };
    let initial_group = initial_watch.group.clone();
    let mut dns_domains = use_signal(String::new);
    let mut dav_url = use_signal(String::new);
    let mut dav_user = use_signal(String::new);
    let mut dav_password = use_signal(String::new);
    let mut report = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut active_tab = use_signal(|| 0usize);
    let tab_labels = vec![
        translate_ui(locale, tr::diagnostics_tab_health()),
        translate_ui(locale, tr::diagnostics_tab_backup()),
        translate_ui(locale, tr::diagnostics_tab_advanced()),
    ];
    let selected_tab = tab_labels[active_tab()].clone();
    let tab_choices = tab_labels.clone();
    let operation_stores = use_context::<UiOperationStores>();
    let service_busy = operation_stores.diagnostics.read().pending.is_some();
    let controls_busy = busy() || service_busy;
    let mut target = use_signal(move || initial_url);
    let mut selected = use_signal(move || initial_group);
    let mut probe = use_signal(move || match locale {
        UiLocale::ZhCn => "尚未执行端到端探测；建立 VPN 隧道后可验证真实 HTTPS 连通性。".to_owned(),
        UiLocale::En => {
            "Not probed yet; run an end-to-end HTTPS probe while VPN is active.".to_owned()
        }
    });
    let options: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    let selected_value = if selected().is_empty() || !options.contains(&selected()) {
        options.first().cloned().unwrap_or_default()
    } else {
        selected()
    };
    let selected_probe = selected_value.clone();
    let watch_group = selected_value.clone();
    let dns_group = selected_value.clone();
    let recovery_supported = groups.iter().any(|group| {
        group.name == selected_value && group.group_type.eq_ignore_ascii_case("selector")
    });
    let no_groups = options.is_empty();
    let report_text = report();
    let current_watch = watch_state();
    let theme = use_theme();

    let engine_decision_text = engine_status_label(&session, settings.vpn.engine, locale);

    let vpn_status_text = if session.vpn_running {
        "已建立隧道"
    } else {
        "未连接"
    };
    let protect_status_text = if session.network_protected {
        "已完成（protectProcessNet）"
    } else if session.vpn_running {
        "待确认"
    } else {
        "未启动"
    };
    let dns_stats_text = format!(
        "处理 {} · 命中 {} · 未命中 {}",
        telemetry.dns.handled_packets, telemetry.dns.cache_hits, telemetry.dns.cache_misses
    );
    let watch_status_summary = if current_watch.enabled {
        format!(
            "已开启 · 监控分组 [{}] · 每 60s 检测 {}",
            current_watch.group, current_watch.url
        )
    } else {
        "未启用（可为 Selector 策略组开启自动容灾切换）".to_owned()
    };

    let body = rsx! {
        column {
            width: "100%",
            FlatSegmented {
                options: tab_labels,
                selected: selected_tab,
                on_change: move |label| {
                    if let Some(index) = tab_choices.iter().position(|tab| tab == &label) { active_tab.set(index); }
                },
            }
            row { height: spacing::MD }
            if controls_busy {
                row {
                    width: "100%",
                    margin_bottom: spacing::MD,
                    align_items: "center",
                    Spinner { size: 14.0 }
                    text { content: translate_ui(locale, tr::diagnostics_working()), margin_left: spacing::SM, font_size: typography::XS, font_color: subtle() }
                }
            }
            if active_tab() == 0 {
                {card(
                    "联网与数据面状态",
                    Some("分层展示隧道建立、套接字保护、数据面内核与真实 HTTPS 探测结果。".into()),
                    rsx! {
                        column {
                            width: "100%",
                            {info_row("VPN 隧道状态", vpn_status_text)}
                            {info_row("底层网络保护", protect_status_text)}
                            {info_row("生效数据面内核", engine_decision_text)}
                            {info_row("DNS 处理统计", dns_stats_text)}
                            {info_row("故障自动自愈", if current_watch.enabled { "已启用" } else { "未启用" })}
                            row { height: spacing::MD }
                            ConnectionStages { session: session.clone(), health: telemetry.startup_health.clone(), locale }
                            text { content: translate_ui(locale, tr::connection_probe_note()), margin_top: spacing::SM, font_size: typography::XS, font_color: subtle(), line_height: 18.0 }
                            row { height: spacing::SM }
                            row {
                                width: "100%",
                                padding: spacing::MD,
                                background_color: muted(),
                                border_radius: theme.radii.md,
                                align_items: "center",
                                {arkit::icon("activity", 16.0, if session.vpn_running { success() } else { subtle() })}
                                row {
                                    layout_weight: 1.0,
                                    margin_left: spacing::SM,
                                    text {
                                        width: "100%",
                                        content: probe(),
                                        font_color: text_color(),
                                        font_size: typography::XS,
                                        line_height: 18.0,
                                    }
                                }
                            }
                        }
                    },
                )}
                row { height: 12.0 }
                {card(
                    "目标服务检测与故障自愈",
                    Some("验证当前 VPN 隧道的真实 HTTPS 连通性；可为手动 Selector 策略组开启连续失败自动切换节点。".into()),
                    rsx! {
                        FieldGroup {
                            if no_groups {
                                FieldDescription { content: translate_ui(locale, tr::diagnostics_empty_groups()), inset: true }
                            }
                            Field {
                                FieldLabel { content: "目标策略组" }
                                FieldDescription {
                                    content: "兼容内核（mihomo / sing-box）检测系统隧道与所选分组；自动自愈仅支持手动 Selector 策略组。",
                                    inset: true,
                                }
                                Select {
                                    options: options.clone(),
                                    selected: Some(selected_value.clone()),
                                    default_selected: selected_value.clone(),
                                    default_open: false,
                                    on_select: move |v| selected.set(v),
                                }
                            }
                            Field {
                                FieldLabel { content: "探测 HTTPS 地址" }
                                FieldDescription {
                                    content: "仅支持不含账号凭证与查询参数的 HTTPS 地址。",
                                    inset: true,
                                }
                                Input {
                                    click_to_focus: true,
                                    value: Some(target()),
                                    width: Some("100%".into()),
                                    disabled: controls_busy,
                                    on_change: move |v| target.set(v),
                                }
                            }
                            row {
                                width: "100%",
                                align_items: "center",
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("32%".into()),
                                    disabled: Some(controls_busy),
                                    onclick: move |_| target.set("https://www.gstatic.com/generate_204".to_owned()),
                                    text { content: "Gstatic 204", font_size: typography::XS, font_color: text_color() }
                                }
                                row { width: spacing::XS }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("32%".into()),
                                    disabled: Some(controls_busy),
                                    onclick: move |_| target.set("https://cp.cloudflare.com/generate_204".to_owned()),
                                    text { content: "Cloudflare", font_size: typography::XS, font_color: text_color() }
                                }
                                row { width: spacing::XS }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("32%".into()),
                                    disabled: Some(controls_busy),
                                    onclick: move |_| target.set("https://github.com/robots.txt".to_owned()),
                                    text { content: "GitHub", font_size: typography::XS, font_color: text_color() }
                                }
                            }
                            FlatButton {
                                variant: FlatButtonVariant::Primary,
                                width: Some("100%".into()),
                                disabled: Some(controls_busy || selected_probe.is_empty() || !session.vpn_running),
                                onclick: move |_| {
                                    if busy() {
                                        return;
                                    }
                                    let url = target();
                                    let Ok(parsed) = url::Url::parse(&url) else {
                                        probe.set("请输入有效 HTTPS 地址".into());
                                        return;
                                    };
                                    if parsed.scheme() != "https"
                                        || !parsed.username().is_empty()
                                        || parsed.password().is_some()
                                        || parsed.query().is_some()
                                    {
                                        probe.set("检测地址只接受不带账号和查询参数的 HTTPS URL".into());
                                        return;
                                    }
                                    let group = selected_probe.clone();
                                    let expected = paws_core::shared_core()
                                        .runtime_status_projection()
                                        .ok()
                                        .and_then(|s| s.vpn_session_id);
                                    busy.set(true);
                                    probe.set("正在执行端到端 HTTPS 探测…".into());
                                    probe_tasks.query(
                                        async move {
                                            let core = paws_core::shared_core();
                                            let delay = tokio::time::timeout(
                                                std::time::Duration::from_secs(10),
                                                core.test_proxy_delay_via_controller(
                                                    &group,
                                                    Some(&url),
                                                    Some(8000),
                                                ),
                                            )
                                            .await
                                            .map_err(|_| "检测超时（10s）".to_owned())?
                                            .map_err(|error| format!("目标检测失败：{error}"))?;
                                            if core
                                                .runtime_status_projection()
                                                .map_err(|e| e.to_string())?
                                                .vpn_session_id
                                                != expected
                                            {
                                                return Err("VPN 会话已变化，请重新检测".into());
                                            }
                                            Ok(format!(
                                                "目标请求成功：{delay} ms（分组：{group}）。仅代表本次 HTTPS 探测结果。"
                                            ))
                                        },
                                        move |result| {
                                            busy.set(false);
                                            probe.set(result.unwrap_or_else(|e| e));
                                        },
                                    );
                                },
                                {arkit::icon("radar", 16.0, primary_text())}
                                text {
                                    content: if session.vpn_running { "立即检测目标服务连通性" } else { "请先开启 VPN 后再执行检测" },
                                    margin_left: spacing::SM,
                                    font_size: typography::SM,
                                    font_weight: 600,
                                    font_color: primary_text(),
                                }
                            }
                            Separator {}
                            column {
                                width: "100%",
                                text {
                                    content: "服务故障自动恢复（Watchdog）",
                                    font_size: typography::SM,
                                    font_weight: 600,
                                    font_color: text_color(),
                                }
                                text {
                                    content: watch_status_summary,
                                    margin_top: 4.0,
                                    font_size: typography::XS,
                                    line_height: 18.0,
                                    font_color: if current_watch.enabled { success() } else { subtle() },
                                }
                            }
                            row {
                                width: "100%",
                                align_items: "center",
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("58%".into()),
                                    disabled: Some(controls_busy || !recovery_supported),
                                    onclick: move |_| {
                                        let next_policy = paws_core::ServiceWatchPolicy {
                                            enabled: true,
                                            group: watch_group.clone(),
                                            url: target(),
                                        };
                                        let result = paws_core::shared_core()
                                            .save_service_watch_policy(next_policy.clone());
                                        match result {
                                            Ok(()) => {
                                                watch_state.set(next_policy);
                                                watch_services.notify("已启用服务恢复：每 60 秒检测，连续失败两次后自动切换候选节点（冷却 5 分钟）");
                                            }
                                            Err(e) => watch_services.notify(e.to_string()),
                                        }
                                    },
                                    {arkit::icon("shield-check", 14.0, text_color())}
                                    text { content: "启用此分组自动自愈", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                }
                                row { width: spacing::SM }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("39%".into()),
                                    disabled: Some(controls_busy || !current_watch.enabled),
                                    onclick: move |_| {
                                        let next_policy = paws_core::ServiceWatchPolicy::default();
                                        let result = paws_core::shared_core()
                                            .save_service_watch_policy(next_policy.clone());
                                        match result {
                                            Ok(()) => {
                                                watch_state.set(next_policy);
                                                disable_services.notify("自动恢复已关闭");
                                            }
                                            Err(e) => disable_services.notify(e.to_string()),
                                        }
                                    },
                                    text { content: "关闭自动自愈", font_size: typography::XS, font_weight: 500, font_color: text_color() }
                                }
                            }
                            if !recovery_supported && !no_groups {
                                FieldDescription { content: translate_ui(locale, tr::diagnostics_selector_only()), inset: true }
                            }
                        }
                    },
                )}
            }
            if active_tab() == 2 {
                row { height: 12.0 }
                {card(
                    "域名专属 DoH 策略",
                    Some("将指定域名及其子域名的 DNS 解析绑定到所选策略组的 Cloudflare DoH，防止特定服务遭受 DNS 污染。".into()),
                    rsx! {
                        FieldGroup {
                            Field {
                                FieldLabel { content: "目标策略组" }
                                Select {
                                    options: options.clone(),
                                    selected: Some(selected_value.clone()),
                                    default_selected: selected_value.clone(),
                                    default_open: false,
                                    on_select: move |v| selected.set(v),
                                }
                            }
                            Field {
                                FieldLabel { content: format!("使用策略组 [{}] 解析的域名列表", if dns_group.is_empty() { "未选择" } else { dns_group.as_str() }) }
                                FieldDescription {
                                    content: "多个域名请用逗号分隔（例如：openai.com, anthropic.com）。保存后保留现有其他 DNS 设置并平滑重载 VPN。",
                                    inset: true,
                                }
                                Input {
                                    click_to_focus: true,
                                    value: Some(dns_domains()),
                                    placeholder: Some("example.com, example.net".into()),
                                    width: Some("100%".into()),
                                    disabled: controls_busy,
                                    on_change: move |v| dns_domains.set(v),
                                }
                            }
                            FlatButton {
                                variant: FlatButtonVariant::Primary,
                                width: Some("100%".into()),
                                disabled: Some(controls_busy || dns_group.is_empty() || dns_domains().trim().is_empty()),
                                onclick: move |_| dns_services.apply_service_dns_policy(dns_domains(), dns_group.clone()),
                                {arkit::icon("save", 16.0, primary_text())}
                                text {
                                    content: "应用域名专属 DoH 策略",
                                    margin_left: spacing::SM,
                                    font_size: typography::SM,
                                    font_weight: 600,
                                    font_color: primary_text(),
                                }
                            }
                        }
                    },
                )}
            }
            if active_tab() == 1 {
                row { height: 12.0 }
                {card(
                    "配置备份与云端同步",
                    Some("支持本地 JSON 文件备份与 WebDAV 远端同步。恢复操作始终新建配置副本，绝不覆盖正在使用的配置。".into()),
                    rsx! {
                        FieldGroup {
                            Field {
                                FieldLabel { content: "本地完整配置备份" }
                                FieldDescription {
                                    content: "包含订阅地址、自定义覆写与节点密钥，请妥善保管导出的备份文件。",
                                    inset: true,
                                }
                                row {
                                    width: "100%",
                                    align_items: "center",
                                    FlatButton {
                                        variant: FlatButtonVariant::Primary,
                                        size: ButtonSize::Sm,
                                        width: Some("48%".into()),
                                        disabled: Some(controls_busy),
                                        onclick: move |_| {
                                            if busy() { return; }
                                            busy.set(true);
                                            let services = backup_services.clone();
                                            backup_tasks.mutate(
                                                async move {
                                                    let content = paws_core::shared_core()
                                                        .export_portable_backup()
                                                        .map_err(|e| e.to_string())?;
                                                    crate::bridge::export_text(
                                                        "backup",
                                                        "Paws-backup.json".into(),
                                                        content,
                                                    )
                                                    .await
                                                },
                                                move |r| {
                                                    busy.set(false);
                                                    services.notify(match r {
                                                        Ok(()) => "配置备份已保存".into(),
                                                        Err(e) => e,
                                                    })
                                                },
                                            );
                                        },
                                        {arkit::icon("download", 14.0, primary_text())}
                                        text { content: "导出本地备份", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: primary_text() }
                                    }
                                    row { width: spacing::SM }
                                    FlatButton {
                                        variant: FlatButtonVariant::Outline,
                                        size: ButtonSize::Sm,
                                        width: Some("48%".into()),
                                        disabled: Some(controls_busy),
                                        onclick: move |_| {
                                            if busy() { return; }
                                            busy.set(true);
                                            let services = restore_services.clone();
                                            restore_tasks.mutate(
                                                async move {
                                                    let Some((_, content)) =
                                                        crate::bridge::pick_profile_text().await?
                                                    else {
                                                        return Ok(None);
                                                    };
                                                    paws_core::shared_core()
                                                        .restore_portable_backup(&content)
                                                        .await
                                                        .map(Some)
                                                        .map_err(|e| e.to_string())
                                                },
                                                move |r| {
                                                    busy.set(false);
                                                    services.refresh_config();
                                                    services.notify(match r {
                                                        Ok(None) => "已取消恢复备份".into(),
                                                        Ok(Some(n)) => format!("已恢复 {n} 个配置副本"),
                                                        Err(e) => e,
                                                    });
                                                },
                                            );
                                        },
                                        {arkit::icon("file-up", 14.0, text_color())}
                                        text { content: "从文件恢复副本", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                    }
                                }
                            }
                            Separator {}
                            Field {
                                FieldLabel { content: "WebDAV 云端备份（HTTPS）" }
                                FieldDescription {
                                    content: "账号与密码仅保存在当前页面内存中，仅在点击上传或下载时发起请求。",
                                    inset: true,
                                }
                                Input {
                                    click_to_focus: true,
                                    value: Some(dav_url()),
                                    placeholder: Some("https://服务器/目录/Paws-backup.json".into()),
                                    width: Some("100%".into()),
                                    disabled: controls_busy,
                                    on_change: move |v| dav_url.set(v),
                                }
                            }
                            Field {
                                FieldLabel { content: "WebDAV 用户名" }
                                Input {
                                    click_to_focus: true,
                                    value: Some(dav_user()),
                                    placeholder: Some("用户名".into()),
                                    width: Some("100%".into()),
                                    disabled: controls_busy,
                                    on_change: move |v| dav_user.set(v),
                                }
                            }
                            Field {
                                FieldLabel { content: "WebDAV 密码 / 应用专用密码" }
                                Input {
                                    click_to_focus: true,
                                    value: Some(dav_password()),
                                    mode: arkit::shadcn::components::InputMode::Password,
                                    placeholder: Some("密码 / 应用密码".into()),
                                    width: Some("100%".into()),
                                    disabled: controls_busy,
                                    on_change: move |v| dav_password.set(v),
                                }
                            }
                            row {
                                width: "100%",
                                align_items: "center",
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("48%".into()),
                                    disabled: Some(controls_busy || dav_url().trim().is_empty()),
                                    onclick: move |_| {
                                        if busy() { return; }
                                        let url = dav_url();
                                        let user = dav_user();
                                        let password = dav_password();
                                        let services = upload_services.clone();
                                        busy.set(true);
                                        upload_tasks.mutate(
                                            async move {
                                                paws_core::shared_core()
                                                    .upload_webdav_backup(&url, &user, &password)
                                                    .await
                                                    .map_err(|e| e.to_string())
                                            },
                                            move |r| {
                                                busy.set(false);
                                                services.notify(match r {
                                                    Ok(()) => "WebDAV 新备份上传成功".into(),
                                                    Err(e) => e,
                                                });
                                            },
                                        );
                                    },
                                    {arkit::icon("arrow-up", 14.0, text_color())}
                                    text { content: "上传到 WebDAV", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                }
                                row { width: spacing::SM }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("48%".into()),
                                    disabled: Some(controls_busy || dav_url().trim().is_empty()),
                                    onclick: move |_| {
                                        if busy() { return; }
                                        let url = dav_url();
                                        let user = dav_user();
                                        let password = dav_password();
                                        let services = download_services.clone();
                                        busy.set(true);
                                        download_tasks.mutate(
                                            async move {
                                                paws_core::shared_core()
                                                    .restore_webdav_backup(&url, &user, &password)
                                                    .await
                                                    .map_err(|e| e.to_string())
                                            },
                                            move |r| {
                                                busy.set(false);
                                                services.refresh_config();
                                                services.notify(match r {
                                                    Ok(n) => format!("WebDAV 已恢复 {n} 个副本"),
                                                    Err(e) => e,
                                                });
                                            },
                                        );
                                    },
                                    {arkit::icon("arrow-down", 14.0, text_color())}
                                    text { content: "下载并恢复副本", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                }
                            }
                        }
                    },
                )}
            }
            if active_tab() == 2 {
                row { height: 12.0 }
                {card(
                    "脱敏诊断报告与多内核验收",
                    Some("导出包含配置覆写路径、规则命中与系统强杀证据（subReason / RSS / PSS）的脱敏诊断 JSON，自动剔除订阅链接与节点密钥。".into()),
                    rsx! {
                        FieldGroup {
                            row {
                                width: "100%",
                                align_items: "center",
                                FlatButton {
                                    variant: FlatButtonVariant::Primary,
                                    size: ButtonSize::Sm,
                                    width: Some("52%".into()),
                                    disabled: Some(controls_busy),
                                    onclick: move |_| {
                                        if busy() { return; }
                                        busy.set(true);
                                        report.set(String::new());
                                        let services = report_services.clone();
                                        tasks.query(
                                            async move {
                                                paws_core::shared_core()
                                                    .diagnostic_report()
                                                    .map_err(|e| e.to_string())
                                            },
                                            move |result| {
                                                busy.set(false);
                                                match result {
                                                    Ok(content) => report.set(content),
                                                    Err(error) => services.notify(error),
                                                }
                                            },
                                        );
                                    },
                                    {arkit::icon("file-text", 14.0, primary_text())}
                                    text { content: "生成诊断报告", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: primary_text() }
                                }
                                row { width: spacing::SM }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("44%".into()),
                                    disabled: Some(report_text.is_empty() || controls_busy),
                                    onclick: move |_| {
                                        if busy() { return; }
                                        busy.set(true);
                                        let content = report();
                                        let services = export_services.clone();
                                        export_tasks.mutate(
                                            async move {
                                                crate::bridge::export_text(
                                                    "diagnostic",
                                                    "Paws-diagnostics.json".into(),
                                                    content,
                                                )
                                                .await
                                            },
                                            move |r| {
                                                busy.set(false);
                                                services.notify(match r {
                                                    Ok(()) => "诊断报告已保存".into(),
                                                    Err(e) => e,
                                                })
                                            },
                                        );
                                    },
                                    {arkit::icon("download", 14.0, text_color())}
                                    text { content: "导出报告文件", margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                }
                            }
                            if !report_text.is_empty() {
                                Textarea {
                                    click_to_focus: true,
                                    value: Some(report_text),
                                    width: Some("100%".into()),
                                    height: Some(260.0),
                                    disabled: true,
                                }
                            }
                            Separator {}
                            Field {
                                FieldLabel { content: "候选内核长测验收记录" }
                                FieldDescription {
                                    content: "你可以在网络设置中直接手动切换 mihomo / sing-box / meow-rs；导入完整 8 小时验收记录后，自动模式也可将该候选内核纳入自动晋升。",
                                    inset: true,
                                }
                                FlatButton {
                                    variant: FlatButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    width: Some("100%".into()),
                                    disabled: Some(controls_busy),
                                    onclick: move |_| {
                                        if busy() { return; }
                                        busy.set(true);
                                        let services = qualification_services.clone();
                                        qualification_tasks.mutate(
                                            async move {
                                                let Some((_, text)) =
                                                    crate::bridge::pick_profile_text().await?
                                                else {
                                                    return Ok(false);
                                                };
                                                paws_core::shared_core()
                                                    .record_engine_qualification(&text)
                                                    .map_err(|e| e.to_string())?;
                                                Ok(true)
                                            },
                                            move |result| {
                                                busy.set(false);
                                                services.notify(match result {
                                                    Ok(true) => "验收记录已导入，下一次连接会重新评估内核".into(),
                                                    Ok(false) => "未导入验收记录".into(),
                                                    Err(e) => e,
                                                })
                                            },
                                        );
                                    },
                                    {arkit::icon("file-up", 14.0, text_color())}
                                    text { content: "导入完整内核验收记录", margin_left: 6.0, font_size: typography::XS, font_weight: 500, font_color: text_color() }
                                }
                            }
                        }
                    },
                )}
            }
        }
    };
    scaffold(Route::Diagnostics {}, rsx! {}, body)
}
