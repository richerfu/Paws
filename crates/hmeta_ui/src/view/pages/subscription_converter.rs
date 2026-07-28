use super::super::*;
use crate::subscription_converter::{
    build_conversion_url, clash_install_url, client_label, client_value, fetch_backend_version,
    format_custom_params, generate_short_url, load_draft, parse_custom_params, remote_config_label,
    resolve_and_parse_conversion_url, save_draft, upload_remote_config, SubscriptionConverterDraft,
    CLIENT_TYPES, REMOTE_CONFIGS,
};

pub(crate) fn subscription_converter_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let initial = load_draft();
    let initial_params = format_custom_params(&initial.custom_params);
    let mut draft = use_signal(move || initial);
    let mut custom_params_text = use_signal(move || initial_params);
    let mut generated_url = use_signal(String::new);
    let mut short_url = use_signal(String::new);
    let mut parse_url = use_signal(String::new);
    let mut upload_content = use_signal(String::new);
    let mut backend_version = use_signal(String::new);
    let mut busy = use_signal(|| None::<String>);

    use_drop(move || {
        let mut value = draft.peek().clone();
        value.custom_params = parse_custom_params(&custom_params_text.peek());
        let _ = save_draft(&value);
    });

    let value = draft();
    let custom_params_value = custom_params_text();
    let generated_value = generated_url();
    let short_value = short_url();
    let parse_value = parse_url();
    let upload_value = upload_content();
    let backend_version_value = backend_version();
    let busy_value = busy();
    let is_busy = busy_value.is_some();
    let basic_label = tr(current.locale, "基础模式", "Basic").to_owned();
    let advanced_label = tr(current.locale, "进阶模式", "Advanced").to_owned();
    let selected_mode = if value.advanced {
        advanced_label.clone()
    } else {
        basic_label.clone()
    };
    let client_options = CLIENT_TYPES
        .iter()
        .map(|item| item.label.to_owned())
        .collect::<Vec<_>>();
    let selected_client = client_label(&value.client_type).to_owned();
    let remote_options = REMOTE_CONFIGS
        .iter()
        .map(|item| item.label.to_owned())
        .collect::<Vec<_>>();
    let selected_remote = remote_config_label(&value.remote_config).to_owned();

    let body = rsx! {
        column {
            width: "100%",
            {card(
                tr(current.locale, "转换规则", "Conversion rule"),
                Some(tr(
                    current.locale,
                    "兼容 sub-web / subconverter 参数；多个订阅或节点链接可每行填写一个",
                    "Compatible with sub-web / subconverter parameters; enter one subscription or node link per line",
                ).to_owned()),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            FieldLabel { content: tr(current.locale, "模式", "Mode").to_owned() }
                            FlatSegmented {
                                options: vec![basic_label.clone(), advanced_label.clone()],
                                selected: selected_mode,
                                on_change: move |selected: String| {
                                    let mut next = draft();
                                    next.advanced = selected == advanced_label;
                                    draft.set(next);
                                },
                            }
                        }
                        Field {
                            FieldLabel {
                                content: tr(current.locale, "订阅或节点链接", "Subscription or node links").to_owned(),
                                required: true,
                            }
                            Textarea {
                                value: Some(value.source_sub_url.clone()),
                                placeholder: Some(tr(
                                    current.locale,
                                    "支持订阅及 ss/ssr/vmess 等链接；多个链接每行一个或使用 | 分隔",
                                    "Supports subscriptions and ss/ssr/vmess links; use one per line or separate with |",
                                ).to_owned()),
                                height: Some(108.0),
                                width: Some("100%".into()),
                                disabled: is_busy,
                                on_change: move |text| {
                                    let mut next = draft();
                                    next.source_sub_url = text;
                                    draft.set(next);
                                },
                            }
                        }
                        Field {
                            FieldLabel {
                                content: tr(current.locale, "目标客户端", "Target client").to_owned(),
                                required: true,
                            }
                            Select {
                                options: client_options,
                                selected: Some(selected_client.clone()),
                                default_selected: selected_client,
                                default_open: false,
                                on_select: move |label: String| {
                                    let mut next = draft();
                                    next.client_type = client_value(&label).to_owned();
                                    draft.set(next);
                                },
                            }
                        }
                    }
                }
            )}

            if value.advanced {
                row { height: spacing::LG }
                {card(
                    tr(current.locale, "后端与远程配置", "Backend & remote config"),
                    Some(tr(
                        current.locale,
                        "默认服务与 sub-web 一致；建议替换为自行部署的 subconverter 及辅助服务",
                        "Defaults match sub-web; using your own subconverter and helper services is recommended",
                    ).to_owned()),
                    rsx! {
                        Form {
                            surface: false,
                            submit_label: String::new(),
                            Field {
                                FieldLabel { content: tr(current.locale, "转换后端", "Converter backend").to_owned() }
                                Input {
                                    value: Some(value.backend.clone()),
                                    placeholder: Some("http://127.0.0.1:25500/sub?".to_owned()),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text: String| {
                                        let mut next = draft();
                                        next.backend = text;
                                        draft.set(next);
                                    },
                                }
                                row {
                                    width: "100%",
                                    margin_top: spacing::SM,
                                    align_items: "center",
                                    row {
                                        layout_weight: 1.0,
                                        text {
                                            width: "100%",
                                            content: if backend_version_value.is_empty() {
                                                tr(current.locale, "尚未检测后端", "Backend not checked").to_owned()
                                            } else {
                                                format!("subconverter {backend_version_value}")
                                            },
                                            font_size: typography::XS,
                                            font_color: subtle(),
                                            max_lines: 1,
                                            text_overflow: "ellipsis",
                                        }
                                    }
                                    FlatButton {
                                        variant: FlatButtonVariant::Ghost,
                                        size: ButtonSize::Sm,
                                        disabled: Some(is_busy),
                                        onclick: move |_| {
                                            busy.set(Some(tr(current.locale, "检测后端", "Checking backend").to_owned()));
                                            let backend = draft().backend;
                                            let task = arkit::tokio_handle().spawn(async move {
                                                fetch_backend_version(&backend).await
                                            });
                                            arkit::dioxus_core::spawn_forever(async move {
                                                match task.await {
                                                    Ok(Ok(version)) => {
                                                        backend_version.set(version);
                                                        converter_notice(state, tr(current.locale, "后端可用", "Backend is available"));
                                                    }
                                                    Ok(Err(error)) => converter_notice(state, error),
                                                    Err(error) => converter_notice(state, format!("后端检测任务失败：{error}")),
                                                }
                                                busy.set(None);
                                            });
                                        },
                                        {arkit::icon("refresh-cw", 14.0, text_color())}
                                        text {
                                            content: tr(current.locale, "检测", "Check"),
                                            margin_left: 6.0,
                                            font_size: typography::XS,
                                            font_weight: 600,
                                            font_color: text_color(),
                                        }
                                    }
                                }
                            }
                            Field {
                                FieldLabel { content: tr(current.locale, "远程配置预设", "Remote config preset").to_owned() }
                                Select {
                                    options: remote_options,
                                    selected: Some(selected_remote.clone()),
                                    default_selected: selected_remote,
                                    default_open: false,
                                    on_select: move |label: String| {
                                        let selected = REMOTE_CONFIGS
                                            .iter()
                                            .find(|item| item.label == label)
                                            .map(|item| item.value)
                                            .unwrap_or_default();
                                        let mut next = draft();
                                        next.remote_config = selected.to_owned();
                                        draft.set(next);
                                    },
                                }
                            }
                            Field {
                                FieldLabel { content: tr(current.locale, "远程配置地址", "Remote config URL").to_owned() }
                                Input {
                                    value: Some(value.remote_config.clone()),
                                    placeholder: Some(tr(current.locale, "可选择预设，也可直接输入 URL", "Choose a preset or enter a URL").to_owned()),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text| {
                                        let mut next = draft();
                                        next.remote_config = text;
                                        draft.set(next);
                                    },
                                }
                            }
                            Field {
                                FieldLabel { content: "Include".to_owned() }
                                Input {
                                    value: Some(value.include_remarks.clone()),
                                    placeholder: Some(tr(current.locale, "节点名必须包含，支持正则", "Node name must match; regex supported").to_owned()),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text| {
                                        let mut next = draft();
                                        next.include_remarks = text;
                                        draft.set(next);
                                    },
                                }
                            }
                            Field {
                                FieldLabel { content: "Exclude".to_owned() }
                                Input {
                                    value: Some(value.exclude_remarks.clone()),
                                    placeholder: Some(tr(current.locale, "排除匹配的节点名，支持正则", "Exclude matching node names; regex supported").to_owned()),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text| {
                                        let mut next = draft();
                                        next.exclude_remarks = text;
                                        draft.set(next);
                                    },
                                }
                            }
                            Field {
                                FieldLabel { content: "FileName".to_owned() }
                                Input {
                                    value: Some(value.filename.clone()),
                                    placeholder: Some(tr(current.locale, "返回的订阅文件名", "Returned subscription filename").to_owned()),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text| {
                                        let mut next = draft();
                                        next.filename = text;
                                        draft.set(next);
                                    },
                                }
                            }
                            Field {
                                FieldLabel { content: tr(current.locale, "自定义参数", "Custom parameters").to_owned() }
                                Textarea {
                                    value: Some(custom_params_value.clone()),
                                    placeholder: Some(tr(
                                        current.locale,
                                        "每行一个 name=value；未知参数在反向解析时会保留",
                                        "One name=value pair per line; unknown parameters survive reverse parsing",
                                    ).to_owned()),
                                    height: Some(88.0),
                                    width: Some("100%".into()),
                                    disabled: is_busy,
                                    on_change: move |text: String| {
                                        custom_params_text.set(text.clone());
                                        let mut next = draft();
                                        next.custom_params = parse_custom_params(&text);
                                        draft.set(next);
                                    },
                                }
                            }
                        }
                    }
                )}

                row { height: spacing::LG }
                {card(
                    tr(current.locale, "输出与定制选项", "Output & customization"),
                    None,
                    rsx! {
                        column {
                            width: "100%",
                            {converter_switch(
                                tr(current.locale, "输出为 Node List", "Output as Node List"),
                                tr(current.locale, "只返回节点列表", "Return only the node list"),
                                value.node_list,
                                move |checked| update_draft_bool(draft, |next| next.node_list = checked),
                            )}
                            {converter_switch("Emoji", tr(current.locale, "为节点名添加地区 Emoji", "Add regional Emoji to node names"), value.emoji, move |checked| update_draft_bool(draft, |next| next.emoji = checked))}
                            {converter_switch(tr(current.locale, "跳过证书验证", "Skip certificate verification"), "scv", value.scv, move |checked| update_draft_bool(draft, |next| next.scv = checked))}
                            {converter_switch(tr(current.locale, "启用 UDP", "Enable UDP"), "udp", value.udp, move |checked| {
                                let mut next = draft();
                                next.udp = checked;
                                next.need_udp = true;
                                draft.set(next);
                            })}
                            {converter_switch("TCP Fast Open", "tfo", value.tfo, move |checked| update_draft_bool(draft, |next| next.tfo = checked))}
                            {converter_switch(tr(current.locale, "附加节点类型", "Append node type"), "append_type", value.append_type, move |checked| update_draft_bool(draft, |next| next.append_type = checked))}
                            {converter_switch(tr(current.locale, "排序节点", "Sort nodes"), "sort", value.sort, move |checked| update_draft_bool(draft, |next| next.sort = checked))}
                            {converter_switch(tr(current.locale, "过滤非法节点", "Filter invalid nodes"), "fdn", value.fdn, move |checked| update_draft_bool(draft, |next| next.fdn = checked))}
                            {converter_switch(tr(current.locale, "展开规则", "Expand rules"), "expand", value.expand, move |checked| update_draft_bool(draft, |next| next.expand = checked))}
                            {converter_switch("Surge DoH", "surge.doh", value.surge_doh, move |checked| update_draft_bool(draft, |next| next.surge_doh = checked))}
                            {converter_switch("Clash DoH", "clash.doh", value.clash_doh, move |checked| update_draft_bool(draft, |next| next.clash_doh = checked))}
                            {converter_switch(tr(current.locale, "Clash 新字段", "Clash new fields"), "new_name", value.new_name, move |checked| update_draft_bool(draft, |next| next.new_name = checked))}
                            {converter_switch(tr(current.locale, "插入默认节点", "Insert default nodes"), "insert / insert_url", value.insert, move |checked| update_draft_bool(draft, |next| next.insert = checked))}
                        }
                    }
                )}
            }

            row { height: spacing::LG }
            {card(
                tr(current.locale, "生成结果", "Generated result"),
                Some(tr(
                    current.locale,
                    "生成操作会保存当前转换规则并复制长链接",
                    "Generating saves the current conversion rule and copies the long URL",
                ).to_owned()),
                rsx! {
                    column {
                        width: "100%",
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(is_busy || value.source_sub_url.trim().is_empty()),
                            onclick: move |_| {
                                let mut next = draft();
                                next.custom_params = parse_custom_params(&custom_params_text());
                                match build_conversion_url(&next) {
                                    Ok(url) => {
                                        let _ = save_draft(&next);
                                        draft.set(next);
                                        generated_url.set(url.clone());
                                        short_url.set(String::new());
                                        copy_converter_text(
                                            state,
                                            url,
                                            tr(current.locale, "转换链接已生成并复制", "Conversion URL generated and copied"),
                                        );
                                    }
                                    Err(error) => converter_notice(state, error),
                                }
                            },
                            {arkit::icon("refresh-cw", 16.0, primary_text())}
                            text {
                                content: tr(current.locale, "生成订阅链接", "Generate subscription URL"),
                                margin_left: 8.0,
                                font_size: typography::SM,
                                font_weight: 600,
                                font_color: primary_text(),
                            }
                        }
                        row { height: spacing::MD }
                        Field {
                            FieldLabel { content: tr(current.locale, "定制订阅", "Converted subscription").to_owned() }
                            Textarea {
                                value: Some(generated_value.clone()),
                                placeholder: Some(tr(current.locale, "生成后显示长链接", "The long URL appears after generation").to_owned()),
                                height: Some(82.0),
                                width: Some("100%".into()),
                                disabled: false,
                                on_change: move |text| generated_url.set(text),
                            }
                        }
                        row {
                            width: "100%",
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                width: Some("48%".into()),
                                disabled: Some(generated_value.is_empty()),
                                onclick: move |_| copy_converter_text(
                                    state,
                                    generated_url(),
                                    tr(current.locale, "长链接已复制", "Long URL copied"),
                                ),
                                {arkit::icon("file-text", 14.0, text_color())}
                                text { content: tr(current.locale, "复制长链", "Copy long URL"), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                            }
                            row { layout_weight: 1.0 }
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                width: Some("48%".into()),
                                disabled: Some(is_busy || generated_value.is_empty()),
                                onclick: move |_| {
                                    let generated = generated_url();
                                    let api = draft().short_url_api;
                                    busy.set(Some(tr(current.locale, "生成短链接", "Generating short URL").to_owned()));
                                    let task = arkit::tokio_handle().spawn(async move {
                                        generate_short_url(&api, &generated).await
                                    });
                                    arkit::dioxus_core::spawn_forever(async move {
                                        match task.await {
                                            Ok(Ok(url)) => {
                                                short_url.set(url.clone());
                                                copy_converter_text(
                                                    state,
                                                    url,
                                                    tr(current.locale, "短链接已生成并复制", "Short URL generated and copied"),
                                                );
                                            }
                                            Ok(Err(error)) => converter_notice(state, error),
                                            Err(error) => converter_notice(state, format!("短链任务失败：{error}")),
                                        }
                                        busy.set(None);
                                    });
                                },
                                if busy_value.as_deref() == Some(tr(current.locale, "生成短链接", "Generating short URL")) {
                                    Spinner { size: 14.0, color: Some(text_color()) }
                                } else {
                                    {arkit::icon("network", 14.0, text_color())}
                                }
                                text { content: tr(current.locale, "生成短链", "Create short URL"), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                            }
                        }
                        row { height: spacing::MD }
                        Field {
                            FieldLabel { content: tr(current.locale, "订阅短链", "Short subscription URL").to_owned() }
                            Input {
                                value: Some(short_value.clone()),
                                placeholder: Some(tr(current.locale, "生成后显示短链接", "The short URL appears after generation").to_owned()),
                                width: Some("100%".into()),
                                read_only: true,
                                on_click: move |_| {
                                    if !short_url().is_empty() {
                                        copy_converter_text(
                                            state,
                                            short_url(),
                                            tr(current.locale, "短链接已复制", "Short URL copied"),
                                        );
                                    }
                                },
                            }
                        }
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            width: Some("100%".into()),
                            disabled: Some(is_busy || generated_value.is_empty()),
                            onclick: move |_| {
                                match clash_install_url(&generated_url(), &short_url()) {
                                    Ok(url) => open_converter_url(state, url),
                                    Err(error) => converter_notice(state, error),
                                }
                            },
                            {arkit::icon("external-link", 16.0, text_color())}
                            text { content: tr(current.locale, "一键导入 Clash", "One-click import to Clash"), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                    }
                }
            )}

            row { height: spacing::LG }
            {card(
                tr(current.locale, "从长链或短链解析", "Parse a long or short URL"),
                Some(tr(
                    current.locale,
                    "短链会跟随最多 10 次重定向，并将最终转换参数还原到上方表单",
                    "Short URLs follow up to 10 redirects and restore final conversion parameters into the form",
                ).to_owned()),
                rsx! {
                    column {
                        width: "100%",
                        Textarea {
                            value: Some(parse_value.clone()),
                            placeholder: Some("https://…/sub?target=clash&url=…".to_owned()),
                            height: Some(82.0),
                            width: Some("100%".into()),
                            disabled: is_busy,
                            on_change: move |text| parse_url.set(text),
                        }
                        row { height: spacing::MD }
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            width: Some("100%".into()),
                            disabled: Some(is_busy || parse_value.trim().is_empty()),
                            onclick: move |_| {
                                let input = parse_url();
                                busy.set(Some(tr(current.locale, "解析链接", "Parsing URL").to_owned()));
                                let task = arkit::tokio_handle().spawn(async move {
                                    resolve_and_parse_conversion_url(&input).await
                                });
                                arkit::dioxus_core::spawn_forever(async move {
                                    match task.await {
                                        Ok(Ok(mut parsed)) => {
                                            let previous = draft();
                                            parsed.short_url_api = previous.short_url_api;
                                            parsed.config_upload_api = previous.config_upload_api;
                                            custom_params_text.set(format_custom_params(&parsed.custom_params));
                                            let _ = save_draft(&parsed);
                                            draft.set(parsed);
                                            generated_url.set(String::new());
                                            short_url.set(String::new());
                                            converter_notice(state, tr(current.locale, "转换链接已解析", "Conversion URL parsed"));
                                        }
                                        Ok(Err(error)) => converter_notice(state, error),
                                        Err(error) => converter_notice(state, format!("链接解析任务失败：{error}")),
                                    }
                                    busy.set(None);
                                });
                            },
                            if busy_value.as_deref() == Some(tr(current.locale, "解析链接", "Parsing URL")) {
                                Spinner { size: 16.0, color: Some(text_color()) }
                            } else {
                                {arkit::icon("download", 16.0, text_color())}
                            }
                            text { content: tr(current.locale, "解析并载入", "Parse and load"), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                    }
                }
            )}

            row { height: spacing::LG }
            {card(
                tr(current.locale, "辅助服务", "Helper services"),
                Some(tr(
                    current.locale,
                    "以下操作会把转换链接或配置内容发送到对应第三方服务；地址可替换为自建服务",
                    "These actions send conversion URLs or config content to the configured third-party services; self-hosted endpoints are supported",
                ).to_owned()),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            FieldLabel { content: tr(current.locale, "短链服务", "Short URL service").to_owned() }
                            Input {
                                value: Some(value.short_url_api.clone()),
                                width: Some("100%".into()),
                                disabled: is_busy,
                                on_change: move |text| {
                                    let mut next = draft();
                                    next.short_url_api = text;
                                    draft.set(next);
                                },
                            }
                        }
                        Field {
                            FieldLabel { content: tr(current.locale, "配置上传服务", "Config upload service").to_owned() }
                            Input {
                                value: Some(value.config_upload_api.clone()),
                                width: Some("100%".into()),
                                disabled: is_busy,
                                on_change: move |text| {
                                    let mut next = draft();
                                    next.config_upload_api = text;
                                    draft.set(next);
                                },
                            }
                        }
                        Field {
                            FieldLabel { content: tr(current.locale, "远程配置内容", "Remote config content").to_owned() }
                            Textarea {
                                value: Some(upload_value.clone()),
                                placeholder: Some(tr(current.locale, "粘贴 subconverter INI 配置", "Paste a subconverter INI config").to_owned()),
                                height: Some(112.0),
                                width: Some("100%".into()),
                                disabled: is_busy,
                                on_change: move |text| upload_content.set(text),
                            }
                        }
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            width: Some("100%".into()),
                            disabled: Some(is_busy || upload_value.trim().is_empty()),
                            onclick: move |_| {
                                let content = upload_content();
                                let api = draft().config_upload_api;
                                busy.set(Some(tr(current.locale, "上传配置", "Uploading config").to_owned()));
                                let task = arkit::tokio_handle().spawn(async move {
                                    upload_remote_config(&api, &content).await
                                });
                                arkit::dioxus_core::spawn_forever(async move {
                                    match task.await {
                                        Ok(Ok(url)) => {
                                            let mut next = draft();
                                            next.remote_config = url.clone();
                                            let _ = save_draft(&next);
                                            draft.set(next);
                                            upload_content.set(String::new());
                                            copy_converter_text(
                                                state,
                                                url,
                                                tr(current.locale, "配置已上传，地址已填入并复制", "Config uploaded; URL filled and copied"),
                                            );
                                        }
                                        Ok(Err(error)) => converter_notice(state, error),
                                        Err(error) => converter_notice(state, format!("配置上传任务失败：{error}")),
                                    }
                                    busy.set(None);
                                });
                            },
                            if busy_value.as_deref() == Some(tr(current.locale, "上传配置", "Uploading config")) {
                                Spinner { size: 16.0, color: Some(text_color()) }
                            } else {
                                {arkit::icon("file-up", 16.0, text_color())}
                            }
                            text { content: tr(current.locale, "上传并使用配置", "Upload and use config"), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                    }
                }
            )}

            if let Some(label) = busy_value {
                row {
                    width: "100%",
                    margin_top: spacing::MD,
                    justify_content: "center",
                    align_items: "center",
                    Spinner { size: 14.0, color: Some(subtle()) }
                    text { content: label, margin_left: 8.0, font_size: typography::XS, font_color: subtle() }
                }
            }
        }
    };

    scaffold(
        state,
        Route::SubscriptionConverter {},
        rsx! {
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                onclick: move |_| open_converter_url(
                    state,
                    "https://github.com/CareyWang/sub-web".to_owned(),
                ),
                {arkit::icon("github", 17.0, text_color())}
            }
        },
        body,
    )
}

fn converter_switch(
    title: impl Into<String>,
    description: impl Into<String>,
    checked: bool,
    mut on_change: impl FnMut(bool) + 'static,
) -> Element {
    let title = title.into();
    let description = description.into();
    rsx! {
        Field {
            orientation: FieldOrientation::Horizontal,
            FieldContent {
                FieldTitle { content: title }
                FieldDescription { content: description, inset: true }
            }
            Switch {
                checked: Some(checked),
                on_change: move |value| on_change(value),
            }
        }
    }
}

fn update_draft_bool(
    mut draft: Signal<SubscriptionConverterDraft>,
    update: impl FnOnce(&mut SubscriptionConverterDraft),
) {
    let mut next = draft();
    update(&mut next);
    draft.set(next);
}

fn converter_notice(state: Signal<State>, message: impl Into<String>) {
    let notifications = state.read().notifications;
    notifications.publish(message.into());
}

fn copy_converter_text(state: Signal<State>, text: String, success: &'static str) {
    let task =
        arkit::tokio_handle().spawn(async move { platform_callbacks::copy_text(text).await });
    arkit::dioxus_core::spawn_forever(async move {
        match task.await {
            Ok(Ok(())) => converter_notice(state, success),
            Ok(Err(error)) => converter_notice(state, format!("复制失败：{error}")),
            Err(error) => converter_notice(state, format!("复制任务失败：{error}")),
        }
    });
}

fn open_converter_url(state: Signal<State>, url: String) {
    let task = arkit::tokio_handle()
        .spawn(async move { platform_callbacks::open_external_url(url).await });
    arkit::dioxus_core::spawn_forever(async move {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => converter_notice(state, format!("打开链接失败：{error}")),
            Err(error) => converter_notice(state, format!("打开链接任务失败：{error}")),
        }
    });
}
