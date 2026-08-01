use super::super::*;

pub(crate) fn profiles_page(state: Signal<State>) -> Element {
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
    let current = state.read().clone();

    use_effect(move || {
        let (succeeded, loading) = {
            let feedback = state.read();
            (
                feedback.profile_import_succeeded,
                feedback.profile_import_loading,
            )
        };
        if import_submitted() && succeeded {
            import_open.set(false);
            import_url.set(String::new());
            import_name.set(String::new());
            import_submitted.set(false);
            dispatch(state, Action::ResetProfileImportFeedback);
        } else if import_submitted() && !loading && !succeeded {
            // Failure, validation error, or cancelled file picker.
            import_submitted.set(false);
        }
    });

    let query_value = query();
    let profiles = current
        .snapshot
        .profiles
        .iter()
        .filter(|profile| matches_profile_query(profile, &query_value))
        .cloned()
        .map(|profile| {
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
                .unwrap_or_else(|| tr(current.locale, "尚未更新", "Never updated").to_owned());
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
                    background_color: surface(),
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 10.0,
                    clip: true,
                    row {
                        layout_weight: 1.0,
                        button {
                            width: "100%",
                            height: 106.0,
                            padding_left: 16.0,
                            padding_right: 8.0,
                            background_color: surface(),
                            border_width: 0.0,
                            border_radius: 0.0,
                            onclick: move |_| {
                                if !active {
                                    dispatch(state, Action::ActivateProfile(activate_id.clone()));
                                }
                            },
                            row {
                                width: "100%",
                                align_items: "center",
                                column {
                                    width: 24.0,
                                    align_items: "start",
                                    {arkit::icon(if active { "circle-check" } else { "circle" }, 20.0, if active { success() } else { subtle() })}
                                }
                                column {
                                    layout_weight: 1.0,
                                    padding_top: 12.0,
                                    padding_bottom: 12.0,
                                    align_items: "start",
                                    text {
                                        content: truncate_text(&profile.name, 40),
                                        font_size: 15.0,
                                        font_weight: 700,
                                        font_color: text_color(),
                                        max_lines: 1,
                                    }
                                    text {
                                        content: truncate_text(&source.replace(['\n', '\r'], " "), 54),
                                        margin_top: 4.0,
                                        font_size: 11.0,
                                        line_height: 16.0,
                                        font_color: subtle(),
                                        max_lines: 1,
                                    }
                                    if let Some(error) = profile.last_refresh_error.clone() {
                                        text {
                                            width: "100%",
                                            content: compact(&error),
                                            margin_top: 6.0,
                                            font_size: 11.0,
                                            line_height: 16.0,
                                            font_color: danger(),
                                            max_lines: 1,
                                        }
                                    } else {
                                        row {
                                            width: "100%",
                                            margin_top: 6.0,
                                            align_items: "center",
                                            {arkit::icon("clock", 12.0, subtle())}
                                            text { content: updated, margin_left: 5.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
                                            if let Some(usage) = usage {
                                                row { layout_weight: 1.0 }
                                                text { content: usage, margin_left: 8.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
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
                            border_radius: 8.0,
                            onclick: move |_| action_profile_id.set(Some(menu_id.clone())),
                            {arkit::icon("ellipsis-vertical", 18.0, subtle())}
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let has_profiles = !current.snapshot.profiles.is_empty();
    let empty = profiles.is_empty();
    let action_profile = action_profile_id().and_then(|id| {
        current
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    });
    let delete_profile = delete_profile_id().and_then(|id| {
        current
            .snapshot
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
                    height: 360.0,
                    align_items: "center",
                    justify_content: "center",
                    {arkit::icon("rss", 30.0, subtle())}
                    text { content: strings(current.locale).profiles_empty_title, margin_top: 16.0, font_size: 17.0, font_weight: 700, font_color: text_color() }
                    text { content: tr(current.locale, "添加订阅以获取代理节点与规则", "Add a subscription to get proxy nodes and rules"), margin_top: 6.0, font_size: 13.0, line_height: 19.0, font_color: subtle(), text_align: "center" }
                    row { height: 18.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Primary,
                        onclick: move |_| {
                            dispatch(state, Action::ResetProfileImportFeedback);
                            import_open.set(true);
                        },
                        {arkit::icon("plus", 16.0, primary_text())}
                        text { content: tr(current.locale, "添加订阅", "Add subscription"), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                    }
                }
            } else {
                Input {
                    value: Some(query_value),
                    placeholder: Some(strings(current.locale).profiles_search_placeholder.to_owned()),
                    width: Some("100%".into()),
                    on_change: move |value| query.set(value),
                }
                row { height: 12.0 }
                if empty {
                    {empty_state("search", strings(current.locale).profiles_no_match_title, strings(current.locale).profiles_no_match_subtitle)}
                } else {
                    {spaced(profiles)}
                }
            }
        }
    };
    let actions = rsx! {
        row {
            {icon_action("refresh-cw", Action::RefreshAllProfiles, state)}
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                onclick: move |_| {
                    dispatch(state, Action::ResetProfileImportFeedback);
                    import_open.set(true);
                },
                {arkit::icon("plus", 18.0, text_color())}
            }
        }
    };
    let page = scaffold(state, Route::Profiles {}, actions, body);
    rsx! {
        {page}
        {profile_import_dialog(
            state,
            &current,
            import_open(),
            import_open,
            import_url,
            import_name,
            import_submitted,
        )}
        {profile_action_dialog(
            state,
            current.locale,
            action_profile,
            action_profile_id,
            edit_profile_id,
            edit_name,
            edit_url,
            delete_profile_id,
        )}
        {profile_edit_dialog(
            state,
            current.locale,
            edit_profile_id,
            edit_name,
            edit_url,
        )}
        {profile_delete_dialog(
            state,
            current.locale,
            delete_profile,
            delete_profile_id,
        )}
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_action_dialog(
    state: Signal<State>,
    locale: UiLocale,
    profile: Option<hmeta_model::ProfileSummary>,
    mut action_profile_id: Signal<Option<String>>,
    mut edit_profile_id: Signal<Option<String>>,
    mut edit_name: Signal<String>,
    mut edit_url: Signal<String>,
    mut delete_profile_id: Signal<Option<String>>,
) -> Element {
    let Some(profile) = profile else {
        return rsx! {};
    };
    let activate_id = profile.id.clone();
    let edit_id = profile.id.clone();
    let edit_profile_name = profile.name.clone();
    let edit_profile_url = profile.subscription_url.clone().unwrap_or_default();
    let yaml_id = profile.id.clone();
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
                description: Some(tr(locale, "配置操作", "Profile actions").to_owned()),
            }
            row { height: 14.0 }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: 9.0,
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
                            dispatch(state, Action::ActivateProfile(activate_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("circle-check", 16.0, text_color())}
                            text { content: tr(locale, "设为当前配置", "Use this profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                            text { content: tr(locale, "编辑订阅", "Edit subscription"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                        dispatch(state, Action::OpenYamlEditor(yaml_id.clone()));
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("file-pen-line", 16.0, text_color())}
                        text { content: tr(locale, "编辑 YAML", "Edit YAML"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                        dispatch(state, Action::ExportProfile(export_id.clone()));
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("download", 16.0, text_color())}
                        text { content: tr(locale, "导出配置", "Export profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                            dispatch(state, Action::RefreshProfile(refresh_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("refresh-cw", 16.0, text_color())}
                            text { content: tr(locale, "刷新订阅", "Refresh subscription"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                            dispatch(state, Action::RestoreProfileBackup(restore_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("history", 16.0, text_color())}
                            text { content: tr(locale, "恢复上次备份", "Restore backup"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
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
                        text { content: tr(locale, "删除配置", "Delete profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: danger() }
                        row { layout_weight: 1.0 }
                    }
                }
            }
        }
    }
}

fn profile_edit_dialog(
    state: Signal<State>,
    locale: UiLocale,
    mut profile_id: Signal<Option<String>>,
    mut name: Signal<String>,
    mut url: Signal<String>,
) -> Element {
    let open = profile_id().is_some();
    rsx! {
        FlatDialog {
            open: open,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: tr(locale, "编辑订阅", "Edit subscription").to_owned(),
                description: Some(tr(locale, "修改名称或订阅地址，保存后可手动刷新。", "Change the name or URL, then refresh when needed.").to_owned()),
            }
            row { height: 18.0 }
            row {
                width: "100%",
                text { content: tr(locale, "名称", "Name"), font_size: 12.0, font_weight: 600, font_color: text_color() }
            }
            row { height: 6.0 }
            Input {
                value: Some(name()),
                placeholder: Some(tr(locale, "配置名称", "Profile name").to_owned()),
                width: Some("100%".into()),
                on_change: move |value| name.set(value),
            }
            row { height: 14.0 }
            row {
                width: "100%",
                text { content: tr(locale, "订阅地址", "Subscription URL"), font_size: 12.0, font_weight: 600, font_color: text_color() }
            }
            row { height: 6.0 }
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
                            dispatch(state, Action::UpdateProfileSubscription {
                                profile_id: id,
                                name: name(),
                                subscription_url: url(),
                            });
                            profile_id.set(None);
                        }
                    },
                    text { content: tr(locale, "保存修改", "Save changes"), font_size: 13.0, font_weight: 600, font_color: primary_text() }
                }
            }
        }
    }
}

fn profile_delete_dialog(
    state: Signal<State>,
    locale: UiLocale,
    profile: Option<hmeta_model::ProfileSummary>,
    mut profile_id: Signal<Option<String>>,
) -> Element {
    let Some(profile) = profile else {
        return rsx! {};
    };
    let delete_id = profile.id.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: tr(locale, "删除配置？", "Delete profile?").to_owned(),
                description: Some(format!("{} · {}", truncate_text(&profile.name, 38), tr(locale, "此操作无法撤销", "This cannot be undone"))),
            }
            row { height: 20.0 }
            DialogFooter {
                row {
                    width: "100%",
                    FlatButton {
                        variant: FlatButtonVariant::Outline,
                        onclick: move |_| profile_id.set(None),
                        text { content: tr(locale, "取消", "Cancel"), font_size: 13.0, font_weight: 600, font_color: text_color() }
                    }
                    row { layout_weight: 1.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Destructive,
                        onclick: move |_| {
                            profile_id.set(None);
                            dispatch(state, Action::DeleteProfile(delete_id.clone()));
                        },
                        text { content: tr(locale, "删除", "Delete"), font_size: 13.0, font_weight: 600, font_color: destructive_text() }
                    }
                }
            }
        }
    }
}

fn profile_import_dialog(
    state: Signal<State>,
    current: &State,
    open: bool,
    mut open_signal: Signal<bool>,
    url: Signal<String>,
    name: Signal<String>,
    submitted: Signal<bool>,
) -> Element {
    let import_loading = current.profile_import_loading;
    let error_value = current.profile_import_error.clone().unwrap_or_default();
    // Refresh the overlay shell when pending/error flips so the live body is remounted
    // with the latest loading branch. Field edits are handled by the body component.
    let content_key = dialog_content_key(&[
        if import_loading { "loading" } else { "idle" },
        &error_value,
    ]);
    rsx! {
        FlatDialog {
            open: open,
            content_key: content_key,
            on_close: move |_| {
                if !state.read().profile_import_loading {
                    open_signal.set(false);
                    dispatch(state, Action::ResetProfileImportFeedback);
                }
            },
            ProfileImportDialogBody {
                state,
                url,
                name,
                submitted,
            }
        }
    }
}

/// Lives inside the overlay tree and re-reads `state` so Spinner/disabled can
/// update while the dialog stays open.
#[component]
fn ProfileImportDialogBody(
    state: Signal<State>,
    mut url: Signal<String>,
    mut name: Signal<String>,
    mut submitted: Signal<bool>,
) -> Element {
    let current = state.read().clone();
    let locale = current.locale;
    let import_loading = current.profile_import_loading;
    let loading_label = strings(locale).profiles_import_loading;
    let url_value = url();
    let name_value = name();
    rsx! {
        DialogHeader {
            title: strings(locale).profiles_import_network.to_owned(),
            description: Some(strings(locale).profiles_import_network_subtitle.to_owned()),
        }
        row { height: 20.0 }
        column {
            width: "100%",
            Input {
                value: Some(url_value),
                placeholder: Some(strings(locale).profiles_import_url_label.to_owned()),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    url.set(value);
                    dispatch(state, Action::ResetProfileImportFeedback);
                },
            }
            row { height: 12.0 }
            Input {
                value: Some(name_value),
                placeholder: Some(strings(locale).profiles_import_name_placeholder.to_owned()),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    name.set(value);
                    dispatch(state, Action::ResetProfileImportFeedback);
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
                        if !state.read().profile_import_loading {
                            submitted.set(true);
                            dispatch(state, Action::ImportLocalProfile);
                        }
                    },
                    if import_loading {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("file-up", 14.0, text_color())}
                    }
                    text {
                        content: if import_loading {
                            loading_label
                        } else {
                            tr(locale, "从本地文件导入", "Import from local file")
                        },
                        margin_left: 6.0,
                        font_size: 12.0,
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
                        if !state.read().profile_import_loading {
                            submitted.set(true);
                            dispatch(state, Action::ScanProfileSubscription {
                                name: name(),
                            });
                        }
                    },
                    if import_loading {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("scan-qr-code", 14.0, text_color())}
                    }
                    text {
                        content: if import_loading {
                            strings(locale).profiles_scan_loading
                        } else {
                            strings(locale).profiles_scan_action
                        },
                        margin_left: 6.0,
                        font_size: 12.0,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
            }
            if let Some(error) = current.profile_import_error.clone() {
                text { content: error, margin_top: 10.0, font_size: 12.0, line_height: 18.0, font_color: danger() }
            }
        }
        DialogFooter {
            FlatButton {
                variant: FlatButtonVariant::Primary,
                width: Some("100%".into()),
                disabled: Some(import_loading),
                onclick: move |_| {
                    if !state.read().profile_import_loading {
                        submitted.set(true);
                        dispatch(state, Action::ImportProfileFromUrl {
                            url: url(),
                            name: name(),
                        });
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
                        strings(locale).profiles_import_submit
                    },
                    margin_left: 8.0,
                    font_size: 14.0,
                    font_weight: 600,
                    font_color: primary_text(),
                }
            }
        }
    }
}
