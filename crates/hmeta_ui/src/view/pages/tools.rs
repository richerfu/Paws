use super::super::*;

pub(crate) fn tools_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let about = current.snapshot.about.clone();
    let body = rsx! {
        column {
            width: "100%",
            {settings_section(
                translate_ui(current.locale, tr::page_tr_019()),
                vec![
                    settings_value_row("package", translate_ui(current.locale, tr::page_tr_020()), about.app_version),
                    settings_value_row("cpu", translate_ui(current.locale, tr::page_tr_021()), format!("meow-rs {}", about.meow_rs_version)),
                    settings_route_row(
                        Route::SubscriptionConverter {},
                        current.locale,
                        tr(
                            current.locale,
                            "在订阅链接、客户端格式与远程规则之间转换",
                            "Convert subscription links, client formats, and remote rules",
                        ),
                    ),
                ],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(current.locale, tr::page_tr_022()),
                vec![settings_route_row(
                    Route::Appearance {},
                    current.locale,
                    translate_ui(current.locale, tr::page_tr_023()),
                )],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(current.locale, tr::page_tr_024()),
                vec![settings_route_row(
                    Route::Settings {},
                    current.locale,
                    translate_ui(current.locale, tr::page_tr_025()),
                )],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(current.locale, tr::page_tr_026()),
                vec![
                    settings_route_row(Route::Requests {}, current.locale, translate_ui(current.locale, tr::page_tr_027())),
                    settings_route_row(Route::Connections { query: String::new() }, current.locale, translate_ui(current.locale, tr::page_tr_028())),
                    settings_route_row(Route::Resources {}, current.locale, translate_ui(current.locale, tr::page_tr_029())),
                    settings_route_row(Route::Logs {}, current.locale, translate_ui(current.locale, tr::page_tr_030())),
                ],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(current.locale, tr::page_tr_031()),
                vec![settings_route_row(
                    Route::About {},
                    current.locale,
                    translate_ui(current.locale, tr::page_tr_032()),
                )],
            )}
        }
    };
    scaffold(state, Route::Tools {}, rsx! {}, body)
}

fn settings_section(title: impl Into<String>, rows: Vec<Element>) -> Element {
    let title = title.into();
    let theme = use_theme();
    let count = rows.len();
    let rows = rows.into_iter().enumerate().map(|(index, row)| {
        rsx! {
            {row}
            if index + 1 < count { Separator {} }
        }
    });
    rsx! {
        column {
            width: "100%",
            align_items: "start",
            text {
                content: title,
                margin_left: spacing::XXS,
                margin_bottom: spacing::SM,
                font_size: typography::SM,
                font_weight: 600,
                font_color: theme.colors.muted_foreground,
            }
            Card {
                shadow: Some(false),
                column {
                    width: "100%",
                    padding_left: spacing::MD,
                    padding_right: spacing::SM,
                    {rows}
                }
            }
        }
    }
}

fn settings_route_row(page: Route, locale: UiLocale, subtitle: impl Into<String>) -> Element {
    let theme = use_theme();
    let navigator = use_navigator();
    let target = page.clone();
    let subtitle = subtitle.into();
    rsx! {
        button {
            button_type: "normal",
            width: "100%",
            height: 68.0,
            padding: 0.0,
            background_color: 0x00000000,
            border_width: 0.0,
            onclick: move |_| {
                navigator.push(target.clone());
            },
            row {
                width: "100%",
                padding_right: spacing::XS,
                align_items: "center",
                row {
                    width: 34.0,
                    height: 34.0,
                    align_items: "center",
                    justify_content: "center",
                    background_color: theme.colors.muted,
                    border_radius: theme.radii.md,
                    {arkit::icon(page.icon(), 17.0, theme.colors.foreground)}
                }
                column {
                    layout_weight: 1.0,
                    margin_left: spacing::MD,
                    align_items: "start",
                    text {
                        content: page.title(locale),
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: theme.colors.foreground,
                    }
                    text {
                        content: subtitle,
                        margin_top: 3.0,
                        font_size: typography::XS,
                        font_color: theme.colors.muted_foreground,
                        max_lines: 1,
                        text_overflow: "ellipsis",
                    }
                }
                {arkit::icon("chevron-right", 16.0, theme.colors.muted_foreground)}
            }
        }
    }
}

fn settings_value_row(
    icon: &'static str,
    title: impl Into<String>,
    value: impl Into<String>,
) -> Element {
    let theme = use_theme();
    let title = title.into();
    let value = value.into();
    rsx! {
        row {
            width: "100%",
            height: 58.0,
            padding_right: spacing::SM,
            align_items: "center",
            row {
                width: 34.0,
                height: 34.0,
                align_items: "center",
                justify_content: "center",
                background_color: theme.colors.muted,
                border_radius: theme.radii.md,
                {arkit::icon(icon, 17.0, theme.colors.foreground)}
            }
            text {
                content: title,
                margin_left: spacing::MD,
                font_size: typography::SM,
                font_weight: 600,
                font_color: theme.colors.foreground,
            }
            row { layout_weight: 1.0 }
            text {
                content: truncate_text(&value, 24),
                margin_left: spacing::MD,
                font_size: typography::XS,
                font_color: theme.colors.muted_foreground,
                max_lines: 1,
                text_align: "end",
            }
        }
    }
}

pub(crate) fn about_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let about = current.snapshot.about;
    let arkit_revision = middle_truncate_text(&about.arkit_rev, 18);
    let body = rsx! {
        column {
            width: "100%",
            {card(
                "Paws",
                Some(translate_ui(current.locale, tr::page_tr_033())),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(current.locale, tr::page_tr_034()), about.app_version)}
                        {info_row(translate_ui(current.locale, tr::page_tr_035()), about.core_version)}
                        {info_row("meow-rs", about.meow_rs_version)}
                        {info_row("arkit", arkit_revision)}
                        {info_row("Rust", about.rust_version)}
                    }
                }
            )}
            row { height: 12.0 }
            {settings_section(
                translate_ui(current.locale, tr::page_tr_036()),
                vec![settings_route_row(
                    Route::Privacy {},
                    current.locale,
                    tr(
                        current.locale,
                        "隐私策略、出口 IP 查询和第三方服务说明",
                        "Privacy policy, exit IP lookup and third-party services",
                    ),
                )],
            )}
            row { height: 10.0 }
            row {
                width: "100%",
                justify_content: "center",
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    width: Some("46%".into()),
                    onclick: move |_| dispatch(state, Action::OpenExternalUrl("https://github.com/madeye/meow-rs".to_owned())),
                    row {
                        width: 18.0,
                        height: 20.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("github", 16.0, text_color())}
                    }
                    text { content: "meow-rs", margin_left: spacing::SM, font_size: typography::SM, line_height: 20.0, font_weight: 600, font_color: text_color() }
                }
                row { width: spacing::SM }
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    width: Some("46%".into()),
                    onclick: move |_| dispatch(state, Action::OpenExternalUrl("https://github.com/richerfu/arkit".to_owned())),
                    row {
                        width: 18.0,
                        height: 20.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("github", 16.0, text_color())}
                    }
                    text { content: "arkit", margin_left: spacing::SM, font_size: typography::SM, line_height: 20.0, font_weight: 600, font_color: text_color() }
                }
            }
        }
    };
    scaffold(state, Route::About {}, rsx! {}, body)
}

pub(crate) fn privacy_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let about = current.snapshot.about;
    let disclosures = about
        .privacy_summary
        .into_iter()
        .map(|item| {
            rsx! {
                row {
                    width: "100%",
                    margin_bottom: spacing::MD,
                    align_items: "start",
                    row {
                        width: 22.0,
                        height: 22.0,
                        margin_top: 1.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("shield-check", 16.0, success())}
                    }
                    row {
                        layout_weight: 1.0,
                        margin_left: spacing::SM,
                        text {
                            width: "100%",
                            content: item,
                            font_size: typography::SM,
                            line_height: 21.0,
                            font_color: text_color(),
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let exit_ip_services = about
        .exit_ip_services
        .into_iter()
        .map(|service| {
            let documentation_url = service.documentation_url;
            rsx! {
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    width: Some("100%".into()),
                    onclick: move |_| dispatch(state, Action::OpenExternalUrl(documentation_url.clone())),
                    row {
                        width: 18.0,
                        height: 20.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("external-link", 16.0, text_color())}
                    }
                    text {
                        content: service.name,
                        margin_left: spacing::SM,
                        font_size: typography::SM,
                        line_height: 20.0,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let body = rsx! {
        column {
            width: "100%",
            {card(
                translate_ui(current.locale, tr::page_tr_037()),
                Some(tr(
                    current.locale,
                    "以下说明覆盖 Paws 自身处理的数据；用户配置的订阅、规则提供方和外部网站由各自运营方负责",
                    "This covers data handled by Paws; configured subscriptions, rule providers and external sites are governed by their operators",
                ).to_owned()),
                rsx! { column { width: "100%", {disclosures.into_iter()} } },
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_038()),
                Some(tr(
                    current.locale,
                    "仅在 VPN 已连接时查询，取首个有效结果。点击服务名称可查看其文档和隐私规则",
                    "Queried only while the VPN is connected; the first valid result is used. Open a service to review its documentation and privacy terms",
                ).to_owned()),
                rsx! { column { width: "100%", {exit_ip_services.into_iter()} } },
            )}
        }
    };
    scaffold(state, Route::Privacy {}, rsx! {}, body)
}
