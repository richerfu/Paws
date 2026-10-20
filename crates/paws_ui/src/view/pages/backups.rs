use super::super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackupAction {
    Export,
    Restore,
    Upload,
    Download,
}

pub(crate) fn backups_page() -> Element {
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let services = use_context::<UiServices>();
    let restore_services = services.clone();
    let download_services = services.clone();
    let tasks = use_page_tasks();
    let restore_tasks = tasks.clone();
    let upload_tasks = tasks.clone();
    let download_tasks = tasks.clone();
    let mut pending = use_signal(|| None::<BackupAction>);
    let mut last_action = use_signal(|| None::<BackupAction>);
    let mut result = use_signal(|| None::<Result<String, String>>);
    let mut dav_url = use_signal(String::new);
    let mut dav_user = use_signal(String::new);
    let mut dav_password = use_signal(String::new);
    let busy = pending().is_some();
    let invalid_url = !url::Url::parse(&dav_url()).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });

    let body = rsx! {
        column {
            width: "100%",
            {card(translate_ui(locale, tr::backup_local_title()), Some(translate_ui(locale, tr::backup_local_description())), rsx! {
                FieldGroup {
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::backup_export_label()) }
                        FieldDescription { content: translate_ui(locale, tr::backup_sensitive_hint()), inset: true }
                        OperationButton {
                            label: translate_ui(locale, tr::backup_export()),
                            icon: "download",
                            variant: ButtonVariant::Outline,
                            disabled: busy,
                            loading: pending() == Some(BackupAction::Export),
                            onclick: move |_| {
                                if pending.peek().is_some() { return; }
                                last_action.set(Some(BackupAction::Export));
                                pending.set(Some(BackupAction::Export)); result.set(None);
                                tasks.mutate(async move {
                                    let content = paws_core::shared_core().export_portable_backup().map_err(|e| e.to_string())?;
                                    crate::bridge::export_text("backup", "Paws-backup.json".into(), content).await?;
                                    Ok(translate_ui(locale, tr::backup_exported()))
                                }, move |outcome| { pending.set(None); result.set(Some(outcome)); });
                            },
                        }
                    }
                    Separator {}
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::backup_restore_label()) }
                        FieldDescription { content: translate_ui(locale, tr::backup_restore_hint()), inset: true }
                        OperationButton {
                            label: translate_ui(locale, tr::backup_restore()),
                            icon: "file-up",
                            variant: ButtonVariant::Outline,
                            disabled: busy,
                            loading: pending() == Some(BackupAction::Restore),
                            onclick: move |_| {
                                if pending.peek().is_some() { return; }
                                last_action.set(Some(BackupAction::Restore));
                                pending.set(Some(BackupAction::Restore)); result.set(None);
                                let services = restore_services.clone();
                                restore_tasks.mutate(async move {
                                    let Some((_, content)) = crate::bridge::pick_profile_text().await? else { return Ok(None); };
                                    paws_core::shared_core().restore_portable_backup(&content).await.map(Some).map_err(|e| e.to_string())
                                }, move |outcome| {
                                    pending.set(None);
                                    match outcome {
                                        Ok(Some(count)) => {
                                            services.refresh_config();
                                            result.set(Some(Ok(translate_ui(locale, tr::backup_restored(count.to_string())))));
                                        }
                                        Ok(None) => result.set(None),
                                        Err(error) => result.set(Some(Err(error))),
                                    }
                                });
                            },
                        }
                    }
                    if matches!(last_action(), Some(BackupAction::Export | BackupAction::Restore)) {
                        OperationNotice { result: result(), locale }
                    }
                }
            })}
            row { height: spacing::LG }
            {card(translate_ui(locale, tr::backup_webdav_title()), Some(translate_ui(locale, tr::backup_webdav_description())), rsx! {
                FieldGroup {
                    Field {
                        invalid: !dav_url().is_empty() && invalid_url,
                        FieldLabel { content: translate_ui(locale, tr::backup_webdav_url()) }
                        Input {
                            click_to_focus: true,
                            accessibility_label: Some(translate_ui(locale, tr::backup_webdav_url())),
                            invalid: !dav_url().is_empty() && invalid_url,
                            value: Some(dav_url()),
                            placeholder: Some("https://example.com/backups/Paws-backup.json".into()),
                            disabled: busy,
                            width: Some("100%".into()),
                            on_change: move |value| dav_url.set(value),
                        }
                        FieldDescription { content: translate_ui(locale, tr::backup_webdav_url_hint()), inset: true }
                        if !dav_url().is_empty() && invalid_url {
                            FieldError { errors: vec![translate_ui(locale, tr::probe_url_invalid())] }
                        }
                    }
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::backup_webdav_user()) }
                        Input {
                            click_to_focus: true,
                            accessibility_label: Some(translate_ui(locale, tr::backup_webdav_user())),
                            value: Some(dav_user()),
                            disabled: busy,
                            width: Some("100%".into()),
                            on_change: move |value| dav_user.set(value),
                        }
                    }
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::backup_webdav_password()) }
                        Input {
                            click_to_focus: true,
                            accessibility_label: Some(translate_ui(locale, tr::backup_webdav_password())),
                            password_toggle_label: translate_ui(locale, tr::backup_password_toggle()),
                            value: Some(dav_password()),
                            mode: arkit::shadcn::components::InputMode::Password,
                            disabled: busy,
                            width: Some("100%".into()),
                            on_change: move |value| dav_password.set(value),
                        }
                    }
                    OperationButton {
                        label: translate_ui(locale, tr::backup_upload()),
                        icon: "upload",
                        disabled: busy || invalid_url,
                        loading: pending() == Some(BackupAction::Upload),
                        onclick: move |_| {
                            if pending.peek().is_some() { return; }
                            let (url, user, password) = (dav_url(), dav_user(), dav_password());
                            last_action.set(Some(BackupAction::Upload));
                                pending.set(Some(BackupAction::Upload)); result.set(None);
                            upload_tasks.mutate(async move {
                                paws_core::shared_core().upload_webdav_backup(&url, &user, &password).await.map_err(|e| e.to_string())?;
                                Ok(translate_ui(locale, tr::backup_uploaded()))
                            }, move |outcome| { pending.set(None); result.set(Some(outcome)); });
                        },
                    }
                    OperationButton {
                        label: translate_ui(locale, tr::backup_download()),
                        icon: "download",
                        variant: ButtonVariant::Outline,
                        disabled: busy || invalid_url,
                        loading: pending() == Some(BackupAction::Download),
                        onclick: move |_| {
                            if pending.peek().is_some() { return; }
                            let (url, user, password) = (dav_url(), dav_user(), dav_password());
                            let services = download_services.clone();
                            last_action.set(Some(BackupAction::Download));
                                pending.set(Some(BackupAction::Download)); result.set(None);
                            download_tasks.mutate(async move {
                                paws_core::shared_core().restore_webdav_backup(&url, &user, &password).await.map_err(|e| e.to_string())
                            }, move |outcome| {
                                pending.set(None);
                                match outcome {
                                    Ok(count) => { services.refresh_config(); result.set(Some(Ok(translate_ui(locale, tr::backup_restored(count.to_string()))))); }
                                    Err(error) => result.set(Some(Err(error))),
                                }
                            });
                        },
                    }
                    if matches!(last_action(), Some(BackupAction::Upload | BackupAction::Download)) {
                        OperationNotice { result: result(), locale }
                    }
                }
            })}
        }
    };
    scaffold(Route::Backups {}, rsx! {}, body)
}
