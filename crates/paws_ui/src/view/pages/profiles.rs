use super::super::*;
use super::yaml_editor::{load_yaml_editor_draft, YamlEditorDialog, YamlEditorDraft};

pub(crate) fn profiles_page() -> Element {
    let services = use_context::<UiServices>();
    let feedback_services = services.clone();
    let empty_import_services = services.clone();
    let refresh_all_services = services.clone();
    let add_import_services = services.clone();
    let mut query = use_signal(String::new);
    let mut import_open = use_signal(|| false);
    let mut import_url = use_signal(String::new);
    let mut import_name = use_signal(String::new);
    let mut import_submitted = use_signal(|| false);
    let mut action_profile_id = use_signal(|| None::<String>);
    let edit_profile_id = use_signal(|| None::<String>);
    let edit_name = use_signal(String::new);
    let edit_url = use_signal(String::new);
    let delete_profile_id = use_signal(|| None::<String>);
    let yaml_editor = use_signal(|| None::<YamlEditorDraft>);
    let stores = use_context::<UiStores>();
    let operations = use_context::<UiOperationStores>();
    let locale = stores.preferences.read().locale;
    let current = stores.profiles.read();

    use_effect(move || {
        let (succeeded, loading) = {
            let feedback = operations.profile_import.read();
            (feedback.succeeded, feedback.loading)
        };
        if import_submitted() && succeeded {
            import_open.set(false);
            import_url.set(String::new());
            import_name.set(String::new());
            import_submitted.set(false);
            feedback_services.reset_profile_import_feedback();
        } else if import_submitted() && !loading && !succeeded {
            // Failure, validation error, or cancelled file picker.
            import_submitted.set(false);
        }
    });

    let query_value = query();
    let profiles = current
        .profiles
        .iter()
        .filter(|profile| matches_profile_query(profile, &query_value))
        .map(|profile| {
            let activate_services = services.clone();
            let activate_id = profile.id.clone();
            let menu_id = profile.id.clone();
            let source = profile
                .subscription_url
                .clone()
                .unwrap_or_else(|| profile.source.clone());
            let updated = profile
                .last_refresh_at
                .as_deref()
                .or(profile.updated_at.as_deref())
                .and_then(time_format::format_unix_nanos)
                .unwrap_or_else(|| translate_ui(locale, tr::page_tr_096()));
            let usage = profile.subscription_user_info.as_ref().and_then(|info| {
                info.total_bytes.map(|total| {
                    format!(
                        "{} / {}",
                        format_total(info.upload_bytes + info.download_bytes),
                        format_total(total)
                    )
                })
            });
            let active = profile.active;
            rsx! {
                row {
                    key: "{profile.id}",
                    width: "100%",
                    height: 108.0,
                    background_color: if active { muted() } else { surface() },
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: radius::LG,
                    clip: true,
                    row {
                        layout_weight: 1.0,
                        button {
                            width: "100%",
                            height: 106.0,
                            padding_left: spacing::LG,
                            padding_right: spacing::SM,
                            background_color: 0x00000000,
                            border_width: 0.0,
                            border_radius: 0.0,
                            onclick: move |_| {
                                if !active {
                                    activate_services.activate_profile(activate_id.clone());
                                }
                            },
                            row {
                                width: "100%",
                                align_items: "center",
                                column {
                                    width: 24.0,
                                    align_items: "start",
                                    {arkit::icon(if active { "circle-check" } else { "circle" }, 18.0, if active { success() } else { subtle() })}
                                }
                                column {
                                    layout_weight: 1.0,
                                    padding_top: spacing::MD,
                                    padding_bottom: spacing::MD,
                                    align_items: "start",
                                    text {
                                        content: truncate_text(&profile.name, 40),
                                        font_size: typography::SM,
                                        font_weight: 600,
                                        font_color: text_color(),
                                        max_lines: 1,
                                    }
                                    text {
                                        content: truncate_text(&source.replace(['\n', '\r'], " "), 54),
                                        margin_top: spacing::XXS,
                                        font_size: typography::XS,
                                        line_height: 16.0,
                                        font_color: subtle(),
                                        max_lines: 1,
                                    }
                                    if let Some(error) = profile.last_refresh_error.clone() {
                                        text {
                                            width: "100%",
                                            content: compact(&error),
                                            margin_top: spacing::XS,
                                            font_size: typography::XS,
                                            line_height: 16.0,
                                            font_color: danger(),
                                            max_lines: 1,
                                        }
                                    } else {
                                        row {
                                            width: "100%",
                                            margin_top: spacing::XS,
                                            align_items: "center",
                                            {arkit::icon("clock", 12.0, subtle())}
                                            text { content: updated, margin_left: spacing::XXS, font_size: typography::XS, font_color: subtle(), max_lines: 1 }
                                            if let Some(usage) = usage {
                                                row { layout_weight: 1.0 }
                                                text { content: usage, margin_left: spacing::SM, font_size: typography::XS, font_color: subtle(), max_lines: 1 }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    column {
                        width: 48.0,
                        height: 106.0,
                        align_items: "center",
                        justify_content: "center",
                        button {
                            width: 40.0,
                            height: 40.0,
                            padding: 0.0,
                            background_color: 0x00000000,
                            border_width: 0.0,
                            border_radius: radius::LG,
                            onclick: move |_| action_profile_id.set(Some(menu_id.clone())),
                            {arkit::icon("ellipsis-vertical", 18.0, subtle())}
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let has_profiles = !current.profiles.is_empty();
    let empty = profiles.is_empty();
    let action_profile = action_profile_id().and_then(|id| {
        current
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    });
    let delete_profile = delete_profile_id().and_then(|id| {
        current
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    });
    let body = rsx! {
        column {
            width: "100%",
            if !has_profiles {
                column {
                    width: "100%",
                    align_items: "center",
                    {empty_state("rss", translate_ui(locale, tr::profiles_empty_title()), translate_ui(locale, tr::page_tr_097()))}
                    row { height: spacing::LG }
                    FlatButton {
                        variant: FlatButtonVariant::Primary,
                        onclick: move |_| {
                            empty_import_services.reset_profile_import_feedback();
                            import_open.set(true);
                        },
                        {arkit::icon("plus", 16.0, primary_text())}
                        text { content: translate_ui(locale, tr::page_tr_098()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: primary_text() }
                    }
                }
            } else {
                Input {
                    value: Some(query_value),
                    placeholder: Some(translate_ui(locale, tr::profiles_search_placeholder())),
                    width: Some("100%".into()),
                    on_change: move |value| query.set(value),
                }
                row { height: 12.0 }
                if empty {
                    {empty_state("search", translate_ui(locale, tr::profiles_no_match_title()), translate_ui(locale, tr::profiles_no_match_subtitle()))}
                } else {
                    {spaced(profiles)}
                }
            }
        }
    };
    let actions = rsx! {
        row {
            FlatButton { variant: FlatButtonVariant::Ghost, size: ButtonSize::Icon, onclick: move |_| refresh_all_services.refresh_all_profiles(), {arkit::icon("refresh-cw", 17.0, text_color())} }
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                onclick: move |_| {
                    add_import_services.reset_profile_import_feedback();
                    import_open.set(true);
                },
                {arkit::icon("plus", 18.0, text_color())}
            }
        }
    };
    let page = scaffold(Route::Profiles {}, actions, body);
    rsx! {
        {page}
        {profile_import_dialog(
            import_open(),
            import_open,
            import_url,
            import_name,
            import_submitted,
        )}
        {profile_action_dialog(
            locale,
            action_profile,
            action_profile_id,
            edit_profile_id,
            edit_name,
            edit_url,
            delete_profile_id,
            yaml_editor,
        )}
        ProfileEditDialog {
            locale,
            profile_id: edit_profile_id,
            name: edit_name,
            url: edit_url,
        }
        {profile_delete_dialog(
            locale,
            delete_profile,
            delete_profile_id,
        )}
        YamlEditorDialog { editor: yaml_editor }
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_action_dialog(
    locale: UiLocale,
    profile: Option<paws_model::ProfileSummary>,
    mut action_profile_id: Signal<Option<String>>,
    mut edit_profile_id: Signal<Option<String>>,
    mut edit_name: Signal<String>,
    mut edit_url: Signal<String>,
    mut delete_profile_id: Signal<Option<String>>,
    mut yaml_editor: Signal<Option<YamlEditorDraft>>,
) -> Element {
    let services = use_context::<UiServices>();
    let stores = use_context::<UiStores>();
    let activate_services = services.clone();
    let yaml_services = services.clone();
    let export_services = services.clone();
    let refresh_services = services.clone();
    let restore_services = services.clone();
    let Some(profile) = profile else {
        return rsx! {};
    };
    let activate_id = profile.id.clone();
    let edit_id = profile.id.clone();
    let edit_profile_name = profile.name.clone();
    let edit_profile_url = profile.subscription_url.clone().unwrap_or_default();
    let yaml_id = profile.id.clone();
    let yaml_profile_name = profile.name.clone();
    let export_id = profile.id.clone();
    let refresh_id = profile.id.clone();
    let restore_id = profile.id.clone();
    let delete_id = profile.id.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| action_profile_id.set(None),
            DialogHeader {
                title: truncate_text(&profile.name, 42),
                description: Some(translate_ui(locale, tr::page_tr_099())),
            }
            row { height: spacing::MD }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: radius::LG,
                clip: true,
                if !profile.active {
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            activate_services.activate_profile(activate_id.clone());
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("circle-check", 16.0, text_color())}
                            text { content: translate_ui(locale, tr::page_tr_100()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                    Separator {}
                }
                if profile.subscription_url.is_some() {
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            edit_profile_id.set(Some(edit_id.clone()));
                            edit_name.set(edit_profile_name.clone());
                            edit_url.set(edit_profile_url.clone());
                            action_profile_id.set(None);
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("file-pen-line", 16.0, text_color())}
                            text { content: translate_ui(locale, tr::page_tr_101()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                    Separator {}
                }
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        action_profile_id.set(None);
                        match load_yaml_editor_draft(
                            yaml_id.clone(),
                            yaml_profile_name.clone(),
                            stores.settings.peek().config_revision,
                        ) {
                            Ok(draft) => yaml_editor.set(Some(draft)),
                            Err(error) => yaml_services.notify(format!(
                                "{}{}",
                                translate_ui(locale, tr::profiles_yaml_read_failed_prefix()),
                                error,
                            )),
                        }
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("file-pen-line", 16.0, text_color())}
                        text { content: translate_ui(locale, tr::page_tr_102()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        row { layout_weight: 1.0 }
                    }
                }
                Separator {}
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        action_profile_id.set(None);
                        export_services.export_profile(export_id.clone());
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("download", 16.0, text_color())}
                        text { content: translate_ui(locale, tr::page_tr_103()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        row { layout_weight: 1.0 }
                    }
                }
                if profile.subscription_url.is_some() {
                    Separator {}
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            refresh_services.refresh_profile(refresh_id.clone());
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("refresh-cw", 16.0, text_color())}
                            text { content: translate_ui(locale, tr::page_tr_104()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                }
                if profile.has_backup {
                    Separator {}
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            restore_services.restore_profile_backup(restore_id.clone());
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("history", 16.0, text_color())}
                            text { content: translate_ui(locale, tr::page_tr_105()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                }
                Separator {}
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        delete_profile_id.set(Some(delete_id.clone()));
                        action_profile_id.set(None);
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("trash-2", 16.0, danger())}
                        text { content: translate_ui(locale, tr::page_tr_106()), margin_left: 10.0, font_size: typography::SM, font_weight: 600, font_color: danger() }
                        row { layout_weight: 1.0 }
                    }
                }
            }
        }
    }
}

#[component]
fn ProfileEditDialog(
    locale: UiLocale,
    mut profile_id: Signal<Option<String>>,
    mut name: Signal<String>,
    mut url: Signal<String>,
) -> Element {
    let services = use_context::<UiServices>();
    let open = profile_id().is_some();
    rsx! {
        FlatDialog {
            open: open,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: translate_ui(locale, tr::page_tr_101()),
                description: Some(translate_ui(locale, tr::page_tr_107())),
            }
            row { height: spacing::LG }
            {field_label(translate_ui(locale, tr::page_tr_108()))}
            row { height: spacing::XS }
            Input {
                value: Some(name()),
                placeholder: Some(translate_ui(locale, tr::page_tr_109())),
                width: Some("100%".into()),
                on_change: move |value| name.set(value),
            }
            row { height: spacing::MD }
            {field_label(translate_ui(locale, tr::page_tr_110()))}
            row { height: spacing::XS }
            Input {
                value: Some(url()),
                placeholder: Some("https://".to_owned()),
                width: Some("100%".into()),
                on_change: move |value| url.set(value),
            }
            DialogFooter {
                FlatButton {
                    variant: FlatButtonVariant::Primary,
                    width: "100%",
                    onclick: move |_| {
                        if let Some(id) = profile_id() {
                            services.update_profile_subscription(id, name(), url());
                            profile_id.set(None);
                        }
                    },
                    text { content: translate_ui(locale, tr::page_tr_111()), font_size: typography::SM, font_weight: 600, font_color: primary_text() }
                }
            }
        }
    }
}

fn profile_delete_dialog(
    locale: UiLocale,
    profile: Option<paws_model::ProfileSummary>,
    mut profile_id: Signal<Option<String>>,
) -> Element {
    let services = use_context::<UiServices>();
    let Some(profile) = profile else {
        return rsx! {};
    };
    let delete_id = profile.id.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: translate_ui(locale, tr::page_tr_112()),
                description: Some(format!("{} · {}", truncate_text(&profile.name, 38), translate_ui(locale, tr::page_tr_113()))),
            }
            row { height: 20.0 }
            DialogFooter {
                row {
                    width: "100%",
                    FlatButton {
                        variant: FlatButtonVariant::Outline,
                        onclick: move |_| profile_id.set(None),
                        text { content: translate_ui(locale, tr::page_tr_114()), font_size: typography::SM, font_weight: 600, font_color: text_color() }
                    }
                    row { layout_weight: 1.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Destructive,
                        onclick: move |_| {
                            profile_id.set(None);
                            services.delete_profile(delete_id.clone());
                        },
                        text { content: translate_ui(locale, tr::page_tr_115()), font_size: typography::SM, font_weight: 600, font_color: destructive_text() }
                    }
                }
            }
        }
    }
}

fn profile_import_dialog(
    open: bool,
    mut open_signal: Signal<bool>,
    url: Signal<String>,
    name: Signal<String>,
    mut submitted: Signal<bool>,
) -> Element {
    let services = use_context::<UiServices>();
    rsx! {
        FlatDialog {
            open: open,
            on_close: move |_| {
                open_signal.set(false);
                submitted.set(false);
                services.cancel_profile_import();
            },
            ProfileImportDialogBody {
                open_signal,
                url,
                name,
                submitted,
            }
        }
    }
}

/// Lives inside the overlay tree and reads only the profile import store so
/// Spinner/disabled updates do not invalidate the profile list.
#[component]
fn ProfileImportDialogBody(
    mut open_signal: Signal<bool>,
    mut url: Signal<String>,
    mut name: Signal<String>,
    mut submitted: Signal<bool>,
) -> Element {
    let services = use_context::<UiServices>();
    let url_services = services.clone();
    let name_services = services.clone();
    let file_services = services.clone();
    let scan_services = services.clone();
    let cancel_services = services.clone();
    let stores = use_context::<UiStores>();
    let operations = use_context::<UiOperationStores>();
    let current = operations.profile_import.read();
    let locale = stores.preferences.read().locale;
    let import_loading = current.loading;
    let loading_label = translate_ui(locale, tr::profiles_import_loading());
    let url_value = url();
    let name_value = name();
    rsx! {
        DialogHeader {
            title: translate_ui(locale, tr::profiles_import_network()),
            description: Some(translate_ui(locale, tr::profiles_import_network_subtitle())),
        }
        row { height: 20.0 }
        column {
            width: "100%",
            Input {
                value: Some(url_value),
                placeholder: Some(translate_ui(locale, tr::profiles_import_url_label())),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    url.set(value);
                    url_services.reset_profile_import_feedback();
                },
            }
            row { height: 12.0 }
            Input {
                value: Some(name_value),
                placeholder: Some(translate_ui(locale, tr::profiles_import_name_placeholder())),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    name.set(value);
                    name_services.reset_profile_import_feedback();
                },
            }
            row { height: 8.0 }
            row {
                width: "100%",
                align_items: "center",
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(import_loading),
                    onclick: move |_| {
                        if !operations.profile_import.peek().loading {
                            submitted.set(true);
                            file_services.import_local_profile();
                        }
                    },
                    if import_loading {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("file-up", 14.0, text_color())}
                    }
                    text {
                        content: if import_loading {
                            loading_label.clone()
                        } else {
                            translate_ui(locale, tr::page_tr_116())
                        },
                        margin_left: 6.0,
                        font_size: typography::XS,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(import_loading),
                    onclick: move |_| {
                        if !operations.profile_import.peek().loading {
                            submitted.set(true);
                            scan_services.scan_profile_subscription(name());
                        }
                    },
                    if import_loading {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("scan-qr-code", 14.0, text_color())}
                    }
                    text {
                        content: if import_loading {
                            translate_ui(locale, tr::profiles_scan_loading())
                        } else {
                            translate_ui(locale, tr::profiles_scan_action())
                        },
                        margin_left: 6.0,
                        font_size: typography::XS,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
            }
            if let Some(error) = current.error.clone() {
                text { content: error, margin_top: 10.0, font_size: typography::XS, line_height: 18.0, font_color: danger() }
            }
        }
        DialogFooter {
            row {
                width: "100%",
                FlatButton {
                    variant: FlatButtonVariant::Outline,
                    onclick: move |_| {
                        open_signal.set(false);
                        submitted.set(false);
                        cancel_services.cancel_profile_import();
                    },
                    text {
                        content: translate_ui(locale, tr::profiles_import_cancel()),
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Primary,
                    disabled: Some(import_loading),
                    onclick: move |_| {
                        if !operations.profile_import.peek().loading {
                            submitted.set(true);
                            services.import_profile_from_url(url(), name());
                        }
                    },
                    if import_loading {
                        Spinner { size: 16.0, color: Some(primary_text()) }
                    } else {
                        {arkit::icon("download", 16.0, primary_text())}
                    }
                    text {
                        content: if import_loading {
                            loading_label
                        } else {
                            translate_ui(locale, tr::profiles_import_submit())
                        },
                        margin_left: 8.0,
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: primary_text(),
                    }
                }
            }
        }
    }
}
