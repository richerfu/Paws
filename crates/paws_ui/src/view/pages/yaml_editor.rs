use super::super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct YamlEditorDraft {
    pub profile_id: String,
    pub profile_name: String,
    pub text: String,
    pub original: String,
    pub base_config_revision: u64,
    pub error: Option<String>,
    pub saving: bool,
    pub testing: bool,
}

pub(super) fn load_yaml_editor_draft(
    profile_id: String,
    profile_name: String,
    base_config_revision: u64,
) -> Result<YamlEditorDraft, String> {
    let raw_yaml = paws_core::shared_core()
        .profile_raw_yaml(&profile_id)
        .map_err(|error| error.to_string())?;
    Ok(YamlEditorDraft {
        profile_id,
        profile_name,
        text: raw_yaml.clone(),
        original: raw_yaml,
        base_config_revision,
        error: None,
        saving: false,
        testing: false,
    })
}

#[component]
pub(super) fn YamlEditorDialog(mut editor: Signal<Option<YamlEditorDraft>>) -> Element {
    let services = use_context::<UiServices>();
    let tasks = use_page_tasks();
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let Some(current) = editor.read().clone() else {
        return rsx! {};
    };
    let summary = summarize_yaml_edit(&current.text, &current.original);
    let busy = current.testing || current.saving;
    let mut close_editor = move || {
        if editor
            .peek()
            .as_ref()
            .is_some_and(|draft| !draft.saving && !draft.testing)
        {
            editor.set(None);
        }
    };
    let mut text_editor = editor;
    let mut reset_editor = editor;
    let mut test_editor = editor;
    let mut save_editor = editor;
    let test_services = services.clone();
    let save_services = services.clone();
    let test_tasks = tasks.clone();
    let save_tasks = tasks;
    let test_profile_id = current.profile_id.clone();
    let test_text = current.text.clone();
    let save_profile_id = current.profile_id.clone();
    let save_profile_name = current.profile_name.clone();
    let save_text = current.text.clone();
    let expected_revision = current.base_config_revision;
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| close_editor(),
            DialogHeader {
                title: translate_ui(locale, tr::profiles_yaml_editor_title()),
                description: Some(current.profile_name.clone()),
            }
            row { height: 16.0 }
            column {
                width: "100%",
                text {
                    content: format!("{} {} · {} {} · {}", summary.lines, translate_ui(locale, tr::profiles_yaml_lines_unit()), summary.characters, translate_ui(locale, tr::profiles_yaml_chars_unit()), if summary.changed { translate_ui(locale, tr::profiles_yaml_changed()) } else { translate_ui(locale, tr::profiles_yaml_unchanged()) }),
                    font_size: typography::XS,
                    font_color: subtle(),
                }
                row { height: 8.0 }
                Textarea {
                    value: Some(current.text.clone()),
                    placeholder: Some(translate_ui(locale, tr::profiles_yaml_content())),
                    height: Some(260.0),
                    width: Some("100%".into()),
                    disabled: busy,
                    on_change: move |value| {
                        if let Some(draft) = text_editor.write().as_mut().filter(|draft| !draft.saving && !draft.testing) {
                            draft.text = value;
                            draft.error = None;
                        }
                    },
                }
                if let Some(error) = current.error.clone() {
                    text { content: error, margin_top: 8.0, font_size: 12.0, font_color: danger() }
                }
            }
            DialogFooter {
                row {
                    width: "100%",
                    FlatButton {
                        variant: FlatButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        disabled: Some(busy),
                        onclick: move |_| {
                            if let Some(draft) = reset_editor.write().as_mut().filter(|draft| !draft.saving && !draft.testing) {
                                draft.text = draft.original.clone();
                                draft.error = None;
                            }
                        },
                        {arkit::icon("rotate-ccw", 14.0, text_color())}
                        text { content: translate_ui(locale, tr::profiles_yaml_reset()), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                    }
                    row { layout_weight: 1.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Outline,
                        size: ButtonSize::Sm,
                        disabled: Some(busy),
                        onclick: move |_| {
                            if test_text.trim().is_empty() {
                                test_services.notify(translate_ui(locale, tr::profiles_yaml_empty()));
                                return;
                            }
                            {
                                let mut draft_slot = test_editor.write();
                                let Some(draft) = draft_slot.as_mut().filter(|draft| !draft.saving && !draft.testing) else {
                                    return;
                                };
                                draft.testing = true;
                                draft.error = None;
                            }
                            let profile_id = test_profile_id.clone();
                            let text = test_text.clone();
                            let services = test_services.clone();
                            test_tasks.query(
                                async move {
                                    paws_core::shared_core()
                                        .validate_profile_content(&text)
                                        .await
                                        .map_err(|error| error.to_string())
                                },
                                move |result| {
                                    let mut draft_slot = test_editor.write();
                                    let Some(draft) = draft_slot.as_mut().filter(|draft| draft.profile_id == profile_id) else {
                                        return;
                                    };
                                    draft.testing = false;
                                    match result {
                                        Ok(()) => services.notify(translate_ui(locale, tr::profiles_yaml_valid())),
                                        Err(error) => draft.error = Some(error),
                                    }
                                },
                            );
                        },
                        if current.testing {
                            Spinner { size: 14.0, color: Some(text_color()) }
                        } else {
                            {arkit::icon("check", 14.0, text_color())}
                        }
                        text { content: if current.testing { translate_ui(locale, tr::profiles_yaml_testing()) } else { translate_ui(locale, tr::profiles_yaml_test()) }, margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                    }
                    row { width: 8.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        disabled: Some(busy),
                        onclick: move |_| {
                            {
                                let mut draft_slot = save_editor.write();
                                let Some(draft) = draft_slot.as_mut().filter(|draft| !draft.saving && !draft.testing) else {
                                    return;
                                };
                                draft.saving = true;
                                draft.error = None;
                            }
                            let profile_id = save_profile_id.clone();
                            let callback_profile_id = profile_id.clone();
                            let profile_name = save_profile_name.clone();
                            let raw_yaml = save_text.clone();
                            let services = save_services.clone();
                            let save_scope = save_tasks.clone();
                            let (owner, vpn_operation_id) = services.begin_owned_vpn_operation(
                                services.connected_session_owner(),
                                VpnCommandAction::Restart,
                            );
                            services.run_durable_mutation(
                                async move {
                                    paws_core::shared_core()
                                        .update_profile_content_checked(&profile_id, expected_revision, &raw_yaml)
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    let config = paws_core::shared_core()
                                        .config_projection()
                                        .map_err(|error| error.to_string())?;
                                    let restart = UiServices::restart_config_projection(owner, &config).await;
                                    Ok((config, restart))
                                },
                                move |services, result| match result {
                                    Ok((config, restart)) => {
                                        services.stores.apply_config_projection(config);
                                        services.refresh_status();
                                        let (requested, error, unconfirmed) =
                                            services.finish_vpn_followup(vpn_operation_id, restart);
                                        if !save_scope.is_alive() {
                                            return;
                                        }
                                        save_editor.set(None);
                                        if let Some(message) = unconfirmed {
                                            services.notify(message);
                                        }
                                        services.notify(settings_saved_message(
                                            &profile_name,
                                            requested,
                                            error.as_deref(),
                                            locale,
                                        ));
                                    }
                                    Err(error) => {
                                        if let Some(operation_id) = vpn_operation_id {
                                            services.finish_vpn_failure(operation_id);
                                        }
                                        if !save_scope.is_alive() {
                                            return;
                                        }
                                        let mut draft_slot = save_editor.write();
                                        if let Some(draft) = draft_slot.as_mut().filter(|draft| draft.profile_id == callback_profile_id) {
                                            draft.saving = false;
                                            draft.error = Some(error);
                                        }
                                    }
                                },
                            );
                        },
                        if current.saving {
                            Spinner { size: 14.0, color: Some(primary_text()) }
                        } else {
                            {arkit::icon("save", 14.0, primary_text())}
                        }
                        text { content: if current.saving { translate_ui(locale, tr::profiles_yaml_saving()) } else { translate_ui(locale, tr::profiles_yaml_save()) }, margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: primary_text() }
                    }
                }
            }
        }
    }
}
