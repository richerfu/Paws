use super::super::*;

pub(crate) fn tools_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let about = current.snapshot.about.clone();
    let body = rsx! {
        column {
            percent_width: 1.0,
            {settings_section(
                tr(current.locale, "常规", "General"),
                vec![
                    settings_value_row("package", tr(current.locale, "版本", "Version"), about.app_version),
                    settings_value_row("cpu", tr(current.locale, "引擎", "Engine"), format!("meow-rs {}", about.meow_rs_version)),
                    settings_route_row(
                        Route::PerApp {},
                        current.locale,
                        tr(current.locale, "选择需要代理或绕过 VPN 的应用", "Choose apps to proxy or bypass"),
                    ),
                ],
            )}
            row { height: 18.0 }
            {settings_section(
                tr(current.locale, "界面", "Appearance"),
                vec![settings_route_row(
                    Route::Appearance {},
                    current.locale,
                    tr(current.locale, "语言、浅色与深色主题", "Language, light and dark themes"),
                )],
            )}
            row { height: 18.0 }
            {settings_section(
                tr(current.locale, "网络", "Network"),
                vec![settings_route_row(
                    Route::Settings {},
                    current.locale,
                    tr(current.locale, "VPN、DNS 与网络栈", "VPN, DNS and network stack"),
                )],
            )}
            row { height: 18.0 }
            {settings_section(
                tr(current.locale, "诊断与管理", "Diagnostics & management"),
                vec![
                    settings_route_row(Route::Requests {}, current.locale, tr(current.locale, "检查最近的请求与规则命中", "Inspect recent requests and rule matches")),
                    settings_route_row(Route::Connections { query: String::new() }, current.locale, tr(current.locale, "查看或断开活动连接", "Inspect or close active connections")),
                    settings_route_row(Route::Resources {}, current.locale, tr(current.locale, "规则、Provider 与 GeoData", "Rules, providers and GeoData")),
                    settings_route_row(Route::Logs {}, current.locale, tr(current.locale, "查看 meow-rs 运行日志", "Inspect meow-rs runtime logs")),
                ],
            )}
            row { height: 18.0 }
            {settings_section(
                tr(current.locale, "关于", "About"),
                vec![settings_route_row(
                    Route::About {},
                    current.locale,
                    tr(current.locale, "开源信息、组件版本与隐私说明", "Open source, component versions and privacy"),
                )],
            )}
        }
    };
    scaffold(state, Route::Tools {}, rsx! {}, body)
}

fn settings_section(title: impl Into<String>, rows: Vec<Element>) -> Element {
    let title = title.into();
    let count = rows.len();
    let rows = rows.into_iter().enumerate().map(|(index, row)| {
        rsx! {
            {row}
            if index + 1 < count { Separator {} }
        }
    });
    rsx! {
        column {
            percent_width: 1.0,
            align_items: "start",
            text {
                content: title,
                margin_left: 4.0,
                margin_bottom: 8.0,
                font_size: 13.0,
                font_weight: 650,
                font_color: subtle(),
            }
            column {
                percent_width: 1.0,
                padding_left: 14.0,
                padding_right: 8.0,
                background_color: surface(),
                border_width: 1.0,
                border_color: line(),
                border_radius: 10.0,
                clip: true,
                {rows}
            }
        }
    }
}

fn settings_route_row(page: Route, locale: UiLocale, subtitle: impl Into<String>) -> Element {
    let navigator = use_navigator();
    let target = page.clone();
    let subtitle = subtitle.into();
    rsx! {
        button {
            percent_width: 1.0,
            height: 68.0,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            background_color: surface(),
            border_width: 0.0,
            onclick: move |_| {
                navigator.push(target.clone());
            },
            row {
                percent_width: 1.0,
                padding_right: 6.0,
                align_items: "center",
                row {
                    width: 34.0,
                    height: 34.0,
                    align_items: "center",
                    justify_content: "center",
                    background_color: muted(),
                    border_radius: 9.0,
                    {arkit::icon(page.icon(), 17.0, text_color())}
                }
                column {
                    layout_weight: 1.0,
                    margin_left: 11.0,
                    align_items: "start",
                    text {
                        content: page.title(locale),
                        font_size: 14.0,
                        font_weight: 650,
                        font_color: text_color(),
                    }
                    text {
                        content: subtitle,
                        margin_top: 3.0,
                        font_size: 11.0,
                        font_color: subtle(),
                        max_lines: 1,
                    }
                }
                {arkit::icon("chevron-right", 16.0, subtle())}
            }
        }
    }
}

fn settings_value_row(
    icon: &'static str,
    title: impl Into<String>,
    value: impl Into<String>,
) -> Element {
    let title = title.into();
    let value = value.into();
    rsx! {
        row {
            percent_width: 1.0,
            height: 58.0,
            padding_right: 8.0,
            align_items: "center",
            row {
                width: 34.0,
                height: 34.0,
                align_items: "center",
                justify_content: "center",
                background_color: muted(),
                border_radius: 9.0,
                {arkit::icon(icon, 17.0, text_color())}
            }
            text { content: title, margin_left: 11.0, font_size: 14.0, font_weight: 600, font_color: text_color() }
            row { layout_weight: 1.0 }
            text { content: truncate_text(&value, 24), margin_left: 12.0, font_size: 12.0, font_color: subtle(), max_lines: 1, text_align: 2 }
        }
    }
}

pub(crate) fn about_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let about = current.snapshot.about;
    let arkit_revision = middle_truncate_text(&about.arkit_rev, 18);
    let privacy = about
        .privacy_summary
        .into_iter()
        .map(|item| {
            rsx! {
                row {
                    percent_width: 1.0,
                    margin_top: 8.0,
                    align_items: "start",
                    row {
                        width: 20.0,
                        height: 20.0,
                        margin_top: 1.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("shield-check", 16.0, success())}
                    }
                    row {
                        layout_weight: 1.0,
                        margin_left: 8.0,
                        text {
                            percent_width: 1.0,
                            content: item,
                            font_size: 13.0,
                            line_height: 19.0,
                            font_color: text_color(),
                            max_lines: 3,
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let body = rsx! {
        column {
            percent_width: 1.0,
            {card(
                "Paws",
                Some(tr(current.locale, "HarmonyOS 原生 meow-rs 客户端", "Native HarmonyOS client powered by meow-rs").to_owned()),
                rsx! {
                    column {
                        percent_width: 1.0,
                        {info_row(tr(current.locale, "应用版本", "App"), about.app_version)}
                        {info_row(tr(current.locale, "核心版本", "Core"), about.core_version)}
                        {info_row("meow-rs", about.meow_rs_version)}
                        {info_row("arkit", arkit_revision)}
                        {info_row("Rust", about.rust_version)}
                    }
                }
            )}
            row { height: 12.0 }
            {card(tr(current.locale, "隐私", "Privacy"), None, rsx! { column { percent_width: 1.0, {privacy.into_iter()} } })}
            row { height: 10.0 }
            row {
                percent_width: 1.0,
                justify_content: "center",
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    percent_width: 0.46,
                    onclick: move |_| dispatch(state, Action::OpenExternalUrl("https://github.com/madeye/meow-rs".to_owned())),
                    row {
                        width: 18.0,
                        height: 20.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("github", 16.0, text_color())}
                    }
                    text { content: "meow-rs", margin_left: 8.0, font_size: 14.0, line_height: 20.0, font_weight: 600, font_color: text_color() }
                }
                row { width: 8.0 }
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    percent_width: 0.46,
                    onclick: move |_| dispatch(state, Action::OpenExternalUrl("https://github.com/richerfu/arkit".to_owned())),
                    row {
                        width: 18.0,
                        height: 20.0,
                        align_items: "center",
                        justify_content: "center",
                        {arkit::icon("github", 16.0, text_color())}
                    }
                    text { content: "arkit", margin_left: 8.0, font_size: 14.0, line_height: 20.0, font_weight: 600, font_color: text_color() }
                }
            }
        }
    };
    scaffold(state, Route::About {}, rsx! {}, body)
}
