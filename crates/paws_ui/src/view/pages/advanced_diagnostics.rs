use super::super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdvancedAction {
    Report,
    Export,
    Qualification,
}

pub(crate) fn advanced_diagnostics_page() -> Element {
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let services = use_context::<UiServices>();
    let operations = use_context::<UiOperationStores>();
    let tasks = use_page_tasks();
    let export_tasks = tasks.clone();
    let qualification_tasks = tasks.clone();
    let groups = stores.proxies.read().groups.clone();
    let options = groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<Vec<_>>();
    let mut selected = use_signal(String::new);
    let group = if options.contains(&selected()) {
        selected()
    } else {
        options.first().cloned().unwrap_or_default()
    };
    let mut domains = use_signal(String::new);
    let mut report = use_signal(String::new);
    let mut pending = use_signal(|| None::<AdvancedAction>);
    let mut last_action = use_signal(|| None::<AdvancedAction>);
    let mut result = use_signal(|| None::<Result<String, String>>);
    let dns_pending = operations.diagnostics.read().pending.is_some();
    let busy = pending().is_some() || dns_pending;

    let body = rsx! {
        column {
            width: "100%",
            {card(translate_ui(locale, tr::service_dns_title()), Some(translate_ui(locale, tr::service_dns_description())), rsx! {
                FieldGroup {
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::diagnostics_group()) }
                        if group.is_empty() {
                            FieldDescription { content: translate_ui(locale, tr::diagnostics_empty_groups()), inset: false }
                        } else {
                            Select {
                                options,
                                accessibility_label: Some(translate_ui(locale, tr::diagnostics_group())),
                                selected: Some(group.clone()),
                                default_selected: group.clone(),
                                default_open: false,
                                disabled: busy,
                                on_select: move |value| selected.set(value),
                            }
                        }
                    }
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::service_dns_domains()) }
                        Input {
                            click_to_focus: true,
                            accessibility_label: Some(translate_ui(locale, tr::service_dns_domains())),
                            value: Some(domains()),
                            placeholder: Some("example.com, example.net".into()),
                            width: Some("100%".into()),
                            disabled: busy,
                            on_change: move |value| domains.set(value),
                        }
                        FieldDescription { content: translate_ui(locale, tr::service_dns_hint()), inset: true }
                    }
                    OperationButton {
                        label: translate_ui(locale, tr::service_dns_apply()),
                        icon: "save",
                        disabled: busy || group.is_empty() || domains().trim().is_empty(),
                        loading: dns_pending,
                        onclick: move |_| services.apply_service_dns_policy(domains(), group.clone()),
                    }
                }
            })}
            row { height: spacing::LG }
            {card(translate_ui(locale, tr::report_title()), Some(translate_ui(locale, tr::report_description())), rsx! {
                FieldGroup {
                    OperationButton {
                        label: translate_ui(locale, tr::report_generate()),
                        icon: "file-text",
                        variant: ButtonVariant::Outline,
                        disabled: busy,
                        loading: pending() == Some(AdvancedAction::Report),
                        onclick: move |_| {
                            if pending.peek().is_some() { return; }
                            last_action.set(Some(AdvancedAction::Report));
                                pending.set(Some(AdvancedAction::Report)); report.set(String::new()); result.set(None);
                            tasks.query(async move { paws_core::shared_core().diagnostic_report().map_err(|e| e.to_string()) }, move |outcome| {
                                pending.set(None);
                                match outcome {
                                    Ok(content) => { report.set(content); result.set(Some(Ok(translate_ui(locale, tr::report_ready())))); }
                                    Err(error) => result.set(Some(Err(error))),
                                }
                            });
                        },
                    }
                    if !report().is_empty() {
                        Field {
                            FieldLabel { content: translate_ui(locale, tr::report_preview()) }
                            Textarea {
                                click_to_focus: true,
                                value: Some(report()),
                                width: Some("100%".into()),
                                height: Some(260.0),
                                disabled: true,
                            }
                        }
                        OperationButton {
                            label: translate_ui(locale, tr::report_export()),
                            icon: "download",
                            variant: ButtonVariant::Outline,
                            disabled: busy,
                            loading: pending() == Some(AdvancedAction::Export),
                            onclick: move |_| {
                                if pending.peek().is_some() { return; }
                                let content = report();
                                last_action.set(Some(AdvancedAction::Export));
                                pending.set(Some(AdvancedAction::Export)); result.set(None);
                                export_tasks.mutate(async move {
                                    crate::bridge::export_text("diagnostic", "Paws-diagnostics.json".into(), content).await?;
                                    Ok(translate_ui(locale, tr::report_exported()))
                                }, move |outcome| { pending.set(None); result.set(Some(outcome)); });
                            },
                        }
                    }
                    if matches!(last_action(), Some(AdvancedAction::Report | AdvancedAction::Export)) {
                        OperationNotice { result: result(), locale }
                    }
                }
            })}
            row { height: spacing::LG }
            {card(translate_ui(locale, tr::qualification_title()), Some(translate_ui(locale, tr::qualification_description())), rsx! {
                FieldGroup {
                    OperationButton {
                        label: translate_ui(locale, tr::qualification_import()),
                        icon: "file-up",
                        variant: ButtonVariant::Outline,
                        disabled: busy,
                        loading: pending() == Some(AdvancedAction::Qualification),
                        onclick: move |_| {
                            if pending.peek().is_some() { return; }
                            last_action.set(Some(AdvancedAction::Qualification));
                                pending.set(Some(AdvancedAction::Qualification)); result.set(None);
                            qualification_tasks.mutate(async move {
                                let Some((_, text)) = crate::bridge::pick_profile_text().await? else { return Ok(None); };
                                paws_core::shared_core().record_engine_qualification(&text).map_err(|e| e.to_string())?;
                                Ok(Some(translate_ui(locale, tr::qualification_imported())))
                            }, move |outcome| {
                                pending.set(None);
                                result.set(match outcome { Ok(Some(message)) => Some(Ok(message)), Ok(None) => None, Err(error) => Some(Err(error)) });
                            });
                        },
                    }
                    if last_action() == Some(AdvancedAction::Qualification) {
                        OperationNotice { result: result(), locale }
                    }
                }
            })}
        }
    };
    scaffold(Route::AdvancedDiagnostics {}, rsx! {}, body)
}
