use super::super::*;
use crate::subscription_converter::{
    build_conversion_url, clash_install_url, client_label, client_value, fetch_backend_version,
    format_custom_params, generate_short_url, load_draft, parse_custom_params, remote_config_label,
    resolve_and_parse_conversion_url, save_draft, upload_remote_config, SubscriptionConverterDraft,
    CLIENT_TYPES, REMOTE_CONFIGS,
};

pub(crate) fn subscription_converter_page() -> Element {
    let services = use_context::<UiServices>();
    let page_tasks = use_page_tasks();
    let version_tasks = page_tasks.clone();
    let short_tasks = page_tasks.clone();
    let parse_tasks = page_tasks.clone();
    let upload_tasks = page_tasks;
    let version_services = services.clone();
    let generate_services = services.clone();
    let copy_services = services.clone();
    let input_copy_services = services.clone();
    let short_services = services.clone();
    let parse_services = services.clone();
    let upload_services = services.clone();
    let link_services = services.clone();
    let install_services = services.clone();
    let drop_services = services;
    let locale = use_context::<UiStores>().preferences.read().locale;
    let initial = use_hook(move || std::rc::Rc::new(load_draft(locale)));
    let draft_initial = initial.clone();
    let params_initial = initial.clone();
    let error_initial = initial.clone();
    let mut draft =
        use_signal(move || draft_initial.as_ref().as_ref().cloned().unwrap_or_default());
    let mut custom_params_text = use_signal(move || {
        params_initial
            .as_ref()
            .as_ref()
            .map(|draft| format_custom_params(&draft.custom_params))
            .unwrap_or_default()
    });
    let persistence_error = use_signal(move || error_initial.as_ref().as_ref().err().cloned());
    let mut generated_url = use_signal(String::new);
    let mut short_url = use_signal(String::new);
    let mut parse_url = use_signal(String::new);
    let mut upload_content = use_signal(String::new);
    let mut backend_version = use_signal(String::new);
    let mut busy = use_signal(|| None::<String>);

    use_drop(move || {
        let mut value = draft.peek().clone();
        value.custom_params = parse_custom_params(&custom_params_text.peek());
        let unchanged = initial
            .as_ref()
            .as_ref()
            .is_ok_and(|loaded| loaded == &value);
        if !unchanged {
            if let Err(error) = save_draft(locale, &value) {
                drop_services.notify(error);
            }
        }
    });

    let value = draft();
    let custom_params_value = custom_params_text();
    let generated_value = generated_url();
    let short_value = short_url();
    let parse_value = parse_url();
    let upload_value = upload_content();
    let backend_version_value = backend_version();
    let busy_value = busy();
    let persistence_error_value = persistence_error();
    let is_busy = busy_value.is_some();
    let basic_label = translate_ui(locale, tr::page_tr_039());
    let advanced_label = translate_ui(locale, tr::page_tr_040());
    let selected_mode = if value.advanced {
        advanced_label.clone()
    } else {
        basic_label.clone()
    };
    let client_options = CLIENT_TYPES
        .iter()
        .map(|item| item.label.to_owned())
        .collect::<Vec<_>>();
    let selected_client = client_label(locale, &value.client_type).to_owned();
    let remote_options = REMOTE_CONFIGS
        .iter()
        .map(|item| item.label.to_owned())
        .collect::<Vec<_>>();
    let selected_remote = remote_config_label(locale, &value.remote_config).to_owned();

    let body = rsx! {
        column {
            width: "100%",
            if let Some(error) = persistence_error_value {
                {card(
                    translate_ui(locale, tr::conv_040()),
                    Some(translate_ui(locale, tr::conv_041())),
                    rsx! { text { content: error, font_size: typography::SM, font_color: danger() } },
                )}
                row { height: spacing::LG }
            }
            {card(
                translate_ui(locale, tr::page_tr_041()),
                Some(translate_ui(locale, tr::hard_zh_040())),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            FieldLabel { content: translate_ui(locale, tr::page_tr_042()) }
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
                                content: translate_ui(locale, tr::page_tr_043()),
                                required: true,
                            }
                            Textarea {
                                value: Some(value.source_sub_url.clone()),
                                placeholder: Some(translate_ui(locale, tr::hard_zh_041())),
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
                                content: translate_ui(locale, tr::page_tr_044()),
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
                    translate_ui(locale, tr::page_tr_045()),
                    Some(translate_ui(locale, tr::hard_zh_042())),
                    rsx! {
                        Form {
                            surface: false,
                            submit_label: String::new(),
                            Field {
                                FieldLabel { content: translate_ui(locale, tr::page_tr_046()) }
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
                                                translate_ui(locale, tr::page_tr_047())
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
                                            busy.set(Some(translate_ui(locale, tr::page_tr_048())));
                                            let backend = draft().backend;
                                            let version_services = version_services.clone();
                                            version_tasks.query(
                                                async move { fetch_backend_version(locale, &backend).await },
                                                move |result| {
                                                    match result {
                                                        Ok(version) => {
                                                            backend_version.set(version);
                                                            converter_notice(&version_services, translate_ui(locale, tr::page_tr_049()));
                                                        }
                                                        Err(error) => converter_notice(&version_services, error),
                                                    }
                                                busy.set(None);
                                                },
                                            );
                                        },
                                        {arkit::icon("refresh-cw", 14.0, text_color())}
                                        text {
                                            content: translate_ui(locale, tr::page_tr_050()),
                                            margin_left: 6.0,
                                            font_size: typography::XS,
                                            font_weight: 600,
                                            font_color: text_color(),
                                        }
                                    }
                                }
                            }
                            Field {
                                FieldLabel { content: translate_ui(locale, tr::page_tr_051()) }
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
                                FieldLabel { content: translate_ui(locale, tr::page_tr_052()) }
                                Input {
                                    value: Some(value.remote_config.clone()),
                                    placeholder: Some(translate_ui(locale, tr::page_tr_053())),
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
                                    placeholder: Some(translate_ui(locale, tr::page_tr_054())),
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
                                    placeholder: Some(translate_ui(locale, tr::page_tr_055())),
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
                                    placeholder: Some(translate_ui(locale, tr::page_tr_056())),
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
                                FieldLabel { content: translate_ui(locale, tr::page_tr_057()) }
                                Textarea {
                                    value: Some(custom_params_value.clone()),
                                    placeholder: Some(translate_ui(locale, tr::hard_zh_043())),
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
                    translate_ui(locale, tr::page_tr_058()),
                    None,
                    rsx! {
                        column {
                            width: "100%",
                            {converter_switch(
                                translate_ui(locale, tr::page_tr_059()),
                                translate_ui(locale, tr::page_tr_060()),
                                value.node_list,
                                move |checked| update_draft_bool(draft, |next| next.node_list = checked),
                            )}
                            {converter_switch("Emoji", translate_ui(locale, tr::page_tr_061()), value.emoji, move |checked| update_draft_bool(draft, |next| next.emoji = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_062()), "scv", value.scv, move |checked| update_draft_bool(draft, |next| next.scv = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_063()), "udp", value.udp, move |checked| {
                                let mut next = draft();
                                next.udp = checked;
                                next.need_udp = true;
                                draft.set(next);
                            })}
                            {converter_switch("TCP Fast Open", "tfo", value.tfo, move |checked| update_draft_bool(draft, |next| next.tfo = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_064()), "append_type", value.append_type, move |checked| update_draft_bool(draft, |next| next.append_type = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_065()), "sort", value.sort, move |checked| update_draft_bool(draft, |next| next.sort = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_066()), "fdn", value.fdn, move |checked| update_draft_bool(draft, |next| next.fdn = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_067()), "expand", value.expand, move |checked| update_draft_bool(draft, |next| next.expand = checked))}
                            {converter_switch("Surge DoH", "surge.doh", value.surge_doh, move |checked| update_draft_bool(draft, |next| next.surge_doh = checked))}
                            {converter_switch("Clash DoH", "clash.doh", value.clash_doh, move |checked| update_draft_bool(draft, |next| next.clash_doh = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_068()), "new_name", value.new_name, move |checked| update_draft_bool(draft, |next| next.new_name = checked))}
                            {converter_switch(translate_ui(locale, tr::page_tr_069()), "insert / insert_url", value.insert, move |checked| update_draft_bool(draft, |next| next.insert = checked))}
                        }
                    }
                )}
            }

            row { height: spacing::LG }
            {card(
                translate_ui(locale, tr::page_tr_070()),
                Some(translate_ui(locale, tr::hard_zh_044())),
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
                                match build_conversion_url(locale, &next) {
                                    Ok(url) => {
                                        persist_converter_draft(
                                            &generate_services,
                                            persistence_error,
                                            locale,
                                            &next,
                                        );
                                        draft.set(next);
                                        generated_url.set(url.clone());
                                        short_url.set(String::new());
                                        copy_converter_text(
                                            generate_services.clone(),
                                            url,
                                            translate_ui(locale, tr::page_tr_071()),
                                        );
                                    }
                                    Err(error) => converter_notice(&generate_services, error),
                                }
                            },
                            {arkit::icon("refresh-cw", 16.0, primary_text())}
                            text {
                                content: translate_ui(locale, tr::page_tr_072()),
                                margin_left: 8.0,
                                font_size: typography::SM,
                                font_weight: 600,
                                font_color: primary_text(),
                            }
                        }
                        row { height: spacing::MD }
                        Field {
                            FieldLabel { content: translate_ui(locale, tr::page_tr_073()) }
                            Textarea {
                                value: Some(generated_value.clone()),
                                placeholder: Some(translate_ui(locale, tr::page_tr_074())),
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
                                    copy_services.clone(),
                                    generated_url(),
                                    translate_ui(locale, tr::page_tr_075()),
                                ),
                                {arkit::icon("file-text", 14.0, text_color())}
                                text { content: translate_ui(locale, tr::page_tr_076()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
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
                                    let short_services = short_services.clone();
                                    busy.set(Some(translate_ui(locale, tr::page_tr_077())));
                                    short_tasks.query(
                                        async move { generate_short_url(locale, &api, &generated).await },
                                        move |result| {
                                        match result {
                                            Ok(url) => {
                                                short_url.set(url.clone());
                                                copy_converter_text(
                                                    short_services.clone(),
                                                    url,
                                                    translate_ui(locale, tr::page_tr_078()),
                                                );
                                            }
                                            Err(error) => converter_notice(&short_services, error),
                                        }
                                        busy.set(None);
                                        },
                                    );
                                },
                                if busy_value.as_deref() == Some(translate_ui(locale, tr::page_tr_077()).as_str()) {
                                    Spinner { size: 14.0, color: Some(text_color()) }
                                } else {
                                    {arkit::icon("network", 14.0, text_color())}
                                }
                                text { content: translate_ui(locale, tr::page_tr_079()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                            }
                        }
                        row { height: spacing::MD }
                        Field {
                            FieldLabel { content: translate_ui(locale, tr::page_tr_080()) }
                            Input {
                                value: Some(short_value.clone()),
                                placeholder: Some(translate_ui(locale, tr::page_tr_081())),
                                width: Some("100%".into()),
                                read_only: true,
                                on_click: move |_| {
                                    if !short_url().is_empty() {
                                        copy_converter_text(
                                            input_copy_services.clone(),
                                            short_url(),
                                            translate_ui(locale, tr::page_tr_082()),
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
                                match clash_install_url(locale, &generated_url(), &short_url()) {
                                    Ok(url) => open_converter_url(install_services.clone(), url),
                                    Err(error) => converter_notice(&install_services, error),
                                }
                            },
                            {arkit::icon("external-link", 16.0, text_color())}
                            text { content: translate_ui(locale, tr::page_tr_083()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                    }
                }
            )}

            row { height: spacing::LG }
            {card(
                translate_ui(locale, tr::page_tr_084()),
                Some(translate_ui(locale, tr::hard_zh_045())),
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
                                let parse_services = parse_services.clone();
                                busy.set(Some(translate_ui(locale, tr::page_tr_085())));
                                parse_tasks.query(
                                    async move { resolve_and_parse_conversion_url(locale, &input).await },
                                    move |result| {
                                    match result {
                                        Ok(mut parsed) => {
                                            let previous = draft();
                                            parsed.short_url_api = previous.short_url_api;
                                            parsed.config_upload_api = previous.config_upload_api;
                                            custom_params_text.set(format_custom_params(&parsed.custom_params));
                                            persist_converter_draft(
                                                &parse_services,
                                                persistence_error,
                                                locale,
                                                &parsed,
                                            );
                                            draft.set(parsed);
                                            generated_url.set(String::new());
                                            short_url.set(String::new());
                                            converter_notice(&parse_services, translate_ui(locale, tr::page_tr_086()));
                                        }
                                        Err(error) => converter_notice(&parse_services, error),
                                    }
                                    busy.set(None);
                                    },
                                );
                            },
                            if busy_value.as_deref() == Some(translate_ui(locale, tr::page_tr_085()).as_str()) {
                                Spinner { size: 16.0, color: Some(text_color()) }
                            } else {
                                {arkit::icon("download", 16.0, text_color())}
                            }
                            text { content: translate_ui(locale, tr::page_tr_087()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                    }
                }
            )}

            row { height: spacing::LG }
            {card(
                translate_ui(locale, tr::page_tr_088()),
                Some(translate_ui(locale, tr::hard_zh_046())),
                rsx! {
                    Form {
                        surface: false,
                        submit_label: String::new(),
                        Field {
                            FieldLabel { content: translate_ui(locale, tr::page_tr_089()) }
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
                            FieldLabel { content: translate_ui(locale, tr::page_tr_090()) }
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
                            FieldLabel { content: translate_ui(locale, tr::page_tr_091()) }
                            Textarea {
                                value: Some(upload_value.clone()),
                                placeholder: Some(translate_ui(locale, tr::page_tr_092())),
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
                                let upload_services = upload_services.clone();
                                busy.set(Some(translate_ui(locale, tr::page_tr_093())));
                                upload_tasks.mutate(
                                    async move { upload_remote_config(locale, &api, &content).await },
                                    move |result| {
                                    match result {
                                        Ok(url) => {
                                            let mut next = draft();
                                            next.remote_config = url.clone();
                                            persist_converter_draft(
                                                &upload_services,
                                                persistence_error,
                                                locale,
                                                &next,
                                            );
                                            draft.set(next);
                                            upload_content.set(String::new());
                                            copy_converter_text(
                                                upload_services.clone(),
                                                url,
                                                translate_ui(locale, tr::page_tr_094()),
                                            );
                                        }
                                        Err(error) => converter_notice(&upload_services, error),
                                    }
                                    busy.set(None);
                                    },
                                );
                            },
                            if busy_value.as_deref() == Some(translate_ui(locale, tr::page_tr_093()).as_str()) {
                                Spinner { size: 16.0, color: Some(text_color()) }
                            } else {
                                {arkit::icon("file-up", 16.0, text_color())}
                            }
                            text { content: translate_ui(locale, tr::page_tr_095()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
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
        Route::SubscriptionConverter {},
        rsx! {
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                onclick: move |_| open_converter_url(
                    link_services.clone(),
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
    on_change: impl FnMut(bool) + 'static,
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
                on_change,
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

fn converter_notice(services: &UiServices, message: impl Into<String>) {
    services.notify(message);
}

fn persist_converter_draft(
    services: &UiServices,
    mut persistence_error: Signal<Option<String>>,
    locale: UiLocale,
    draft: &SubscriptionConverterDraft,
) {
    match save_draft(locale, draft) {
        Ok(()) => persistence_error.set(None),
        Err(error) => {
            persistence_error.set(Some(error.clone()));
            services.notify(error);
        }
    }
}

fn copy_converter_text(services: UiServices, text: String, success: String) {
    let locale = services.stores.preferences.peek().locale;
    let runtime = services.runtime.clone();
    arkit::dioxus_core::spawn_forever(async move {
        let result = bridge::copy_text(text).await;
        runtime.queue_ui(move || match result {
            Ok(()) => converter_notice(&services, success),
            Err(error) => converter_notice(&services, translate_ui(locale, tr::hard_zh_036(error))),
        });
    });
}

fn open_converter_url(services: UiServices, url: String) {
    let locale = services.stores.preferences.peek().locale;
    let runtime = services.runtime.clone();
    arkit::dioxus_core::spawn_forever(async move {
        let result = bridge::open_external_url(url).await;
        runtime.queue_ui(move || {
            if let Err(error) = result {
                converter_notice(&services, translate_ui(locale, tr::hard_zh_038(error)));
            }
        });
    });
}
