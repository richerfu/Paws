use super::super::*;

pub(crate) fn tools_page() -> Element {
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let about = stores.about.read().about.clone();
    let body = rsx! {
        column {
            width: "100%",
            {settings_section(
                translate_ui(locale, tr::page_tr_022()),
                vec![settings_route_row(
                    Route::Appearance {},
                    locale,
                    translate_ui(locale, tr::page_tr_023()),
                )],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(locale, tr::page_tr_024()),
                vec![settings_route_row(
                    Route::Settings {},
                    locale,
                    translate_ui(locale, tr::page_tr_025()),
                )],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(locale, tr::page_tr_272()),
                vec![settings_route_row(
                    Route::SubscriptionConverter {},
                    locale,
                    translate_ui(locale, tr::hard_zh_023()),
                )],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(locale, tr::page_tr_026()),
                vec![
                    settings_route_row(Route::Requests {}, locale, translate_ui(locale, tr::page_tr_027())),
                    settings_route_row(Route::Connections { query: String::new() }, locale, translate_ui(locale, tr::page_tr_028())),
                    settings_route_row(Route::Resources {}, locale, translate_ui(locale, tr::page_tr_029())),
                    settings_route_row(Route::Logs {}, locale, translate_ui(locale, tr::page_tr_030())),
                ],
            )}
            row { height: spacing::LG }
            {settings_section(
                translate_ui(locale, tr::page_tr_031()),
                vec![settings_route_row(
                    Route::About {},
                    locale,
                    translate_ui(locale, tr::page_tr_032()),
                )],
            )}
            row { height: spacing::XXL }
            row {
                width: "100%",
                justify_content: "center",
                text {
                    content: format!(
                        "Paws {} · meow-rs {}",
                        about.app_version,
                        about.meow_rs_version
                    ),
                    font_size: typography::XS,
                    font_color: subtle(),
                }
            }
        }
    };
    scaffold(Route::Tools {}, rsx! {}, body)
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
            height: 60.0,
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

#[allow(dead_code)]
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

pub(crate) fn about_page() -> Element {
    let services = use_context::<UiServices>();
    let meow_services = services.clone();
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let about = stores.about.read().about.clone();
    let arkit_revision = middle_truncate_text(&about.arkit_rev, 18);
    let body = rsx! {
        column {
            width: "100%",
            column {
                width: "100%",
                padding_top: spacing::SM,
                padding_bottom: spacing::LG,
                align_items: "center",
                row {
                    width: 56.0,
                    height: 56.0,
                    align_items: "center",
                    justify_content: "center",
                    background_color: muted(),
                    border_radius: radius::XXL,
                    {arkit::icon("paw-print", 26.0, text_color())}
                }
                text {
                    content: "Paws",
                    margin_top: spacing::MD,
                    font_size: typography::XL,
                    line_height: 28.0,
                    font_weight: 600,
                    font_color: text_color(),
                }
                text {
                    content: translate_ui(locale, tr::page_tr_033()),
                    margin_top: spacing::XXS,
                    font_size: typography::SM,
                    line_height: 20.0,
                    font_color: subtle(),
                    text_align: "center",
                }
            }
            {card(
                translate_ui(locale, tr::page_tr_020()),
                None,
                rsx! {
                    column {
                        width: "100%",
                        {info_row(translate_ui(locale, tr::page_tr_034()), about.app_version)}
                        {info_row(translate_ui(locale, tr::page_tr_035()), about.core_version)}
                        {info_row("meow-rs", about.meow_rs_version)}
                        {info_row("arkit", arkit_revision)}
                        {info_row("Rust", about.rust_version)}
                    }
                }
            )}
            row { height: 12.0 }
            {settings_section(
                translate_ui(locale, tr::page_tr_036()),
                vec![settings_route_row(
                    Route::Privacy {},
                    locale,
                    translate_ui(locale, tr::hard_zh_024()),
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
                    onclick: move |_| meow_services.open_external_url("https://github.com/madeye/meow-rs".to_owned()),
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
                    onclick: move |_| services.open_external_url("https://github.com/richerfu/arkit".to_owned()),
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
    scaffold(Route::About {}, rsx! {}, body)
}

pub(crate) fn privacy_page() -> Element {
    let services = use_context::<UiServices>();
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let about = stores.about.read().about.clone();
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
            let services = services.clone();
            let documentation_url = service.documentation_url;
            rsx! {
                FlatButton {
                    variant: FlatButtonVariant::Link,
                    size: ButtonSize::Sm,
                    width: Some("100%".into()),
                    onclick: move |_| services.open_external_url(documentation_url.clone()),
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
                translate_ui(locale, tr::page_tr_037()),
                Some(translate_ui(locale, tr::hard_zh_025())),
                rsx! { column { width: "100%", {disclosures.into_iter()} } },
            )}
            row { height: 12.0 }
            {card(
                translate_ui(locale, tr::page_tr_038()),
                Some(translate_ui(locale, tr::hard_zh_026())),
                rsx! { column { width: "100%", {exit_ip_services.into_iter()} } },
            )}
        }
    };
    scaffold(Route::Privacy {}, rsx! {}, body)
}
