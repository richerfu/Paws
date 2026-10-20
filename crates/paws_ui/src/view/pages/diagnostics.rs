use super::super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiagnosticAction {
    Probe,
    Watch,
}

pub(crate) fn diagnostics_page() -> Element {
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let session = stores.session.read().clone();
    let telemetry = stores.telemetry.read().clone();
    let preferred = stores.settings.read().vpn.engine;
    let groups = stores.proxies.read().groups.clone();
    let tasks = use_page_tasks();
    let probe_tasks = tasks.clone();
    let initial = use_hook(|| {
        paws_core::shared_core()
            .service_watch_policy()
            .map_err(|e| e.to_string())
    });
    let mut watch_state = use_signal(|| initial.clone().unwrap_or_default());
    let initial_policy = watch_state.peek().clone();
    let mut selected = use_signal(|| initial_policy.group.clone());
    let mut target = use_signal(|| {
        if initial_policy.url.is_empty() {
            "https://www.gstatic.com/generate_204".to_owned()
        } else {
            initial_policy.url.clone()
        }
    });
    let mut pending = use_signal(|| None::<DiagnosticAction>);
    let mut probe_result = use_signal(|| None::<Result<String, String>>);
    let mut watch_result = use_signal(|| initial.err().map(Err));
    let busy = pending().is_some();
    let options = groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<Vec<_>>();
    let group = if options.contains(&selected()) {
        selected()
    } else {
        options.first().cloned().unwrap_or_default()
    };
    let selector = groups
        .iter()
        .any(|entry| entry.name == group && entry.group_type.eq_ignore_ascii_case("selector"));
    let watch = watch_state();
    let invalid_url = !valid_probe_url(&target());
    let changed = watch.group != group || watch.url != target();
    let probe_group = group.clone();
    let toggle_group = group.clone();
    let save_group = group.clone();
    let expected_session = session.vpn_session_id.clone();
    let engine = engine_status_label(&session, preferred, locale);

    let save_watch = move |policy: paws_core::ServiceWatchPolicy| {
        if pending.peek().is_some() {
            return;
        }
        pending.set(Some(DiagnosticAction::Watch));
        watch_result.set(None);
        tasks.mutate(
            async move {
                paws_core::shared_core()
                    .save_service_watch_policy(policy.clone())
                    .map_err(|e| e.to_string())?;
                Ok(policy)
            },
            move |result| {
                pending.set(None);
                match result {
                    Ok(policy) => {
                        watch_result.set(Some(Ok(translate_ui(
                            locale,
                            if policy.enabled {
                                tr::watch_saved()
                            } else {
                                tr::watch_disabled()
                            },
                        ))));
                        watch_state.set(policy);
                    }
                    Err(error) => watch_result.set(Some(Err(error))),
                }
            },
        );
    };
    let mut toggle_watch = save_watch.clone();
    let mut update_watch = save_watch;
    let presets = [
        (
            "Gstatic 204".to_owned(),
            "https://www.gstatic.com/generate_204",
        ),
        (
            "Cloudflare".to_owned(),
            "https://cp.cloudflare.com/generate_204",
        ),
        ("GitHub".to_owned(), "https://github.com/robots.txt"),
        (translate_ui(locale, tr::probe_custom()), ""),
    ];
    let selected_preset = presets
        .iter()
        .find(|(_, url)| *url == target())
        .unwrap_or(&presets[3])
        .0
        .clone();
    let preset_options = presets
        .iter()
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();

    let body = rsx! {
        column {
            width: "100%",
            {card(translate_ui(locale, tr::diagnostics_status_title()), Some(translate_ui(locale, tr::connection_probe_note())), rsx! {
                FieldGroup {
                    {info_row(translate_ui(locale, tr::diagnostics_tunnel()), translate_ui(locale, if session.vpn_running { tr::diagnostics_connected() } else { tr::diagnostics_disconnected() }))}
                    {info_row(translate_ui(locale, tr::diagnostics_engine()), engine)}
                    {info_row("DNS", translate_ui(locale, tr::diagnostics_dns_counts(telemetry.dns.handled_packets.to_string(), telemetry.dns.cache_hits.to_string(), telemetry.dns.cache_misses.to_string())))}
                    Separator {}
                    ConnectionChecks { session: session.clone(), health: telemetry.startup_health.clone(), locale }
                    if let Some(error) = session.network_protect_error.clone() {
                        OperationNotice { result: Some(Err(error)), locale }
                    }
                }
            })}
            row { height: spacing::LG }
            {card(translate_ui(locale, tr::probe_title()), Some(translate_ui(locale, tr::probe_description())), rsx! {
                FieldGroup {
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::diagnostics_group()) }
                        Select {
                            options,
                            selected: Some(group.clone()),
                            default_selected: group.clone(),
                            default_open: false,
                            disabled: busy || group.is_empty(),
                            on_select: move |value| selected.set(value),
                        }
                        if group.is_empty() {
                            FieldDescription { content: translate_ui(locale, tr::diagnostics_empty_groups()), inset: true }
                        }
                    }
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::probe_preset()) }
                        Select {
                            options: preset_options,
                            selected: Some(selected_preset.clone()),
                            default_selected: selected_preset,
                            default_open: false,
                            disabled: busy,
                            on_select: move |value| {
                                if let Some((_, url)) = presets.iter().find(|(label, _)| *label == value) {
                                    if !url.is_empty() { target.set((*url).to_owned()); }
                                }
                            },
                        }
                    }
                    Field {
                        FieldLabel { content: translate_ui(locale, tr::probe_url()) }
                        Input {
                            click_to_focus: true,
                            value: Some(target()),
                            width: Some("100%".into()),
                            disabled: busy,
                            on_change: move |value| target.set(value),
                        }
                        FieldDescription { content: translate_ui(locale, tr::probe_url_hint()), inset: true }
                        if invalid_url {
                            FieldError { errors: vec![translate_ui(locale, tr::probe_url_invalid())] }
                        }
                    }
                    OperationButton {
                        label: translate_ui(locale, if session.vpn_running { tr::probe_run() } else { tr::probe_connect_first() }),
                        icon: "radar",
                        disabled: busy || invalid_url || group.is_empty() || !session.vpn_running,
                        loading: pending() == Some(DiagnosticAction::Probe),
                        onclick: move |_| {
                            if pending.peek().is_some() { return; }
                            let url = target();
                            let group = probe_group.clone();
                            let expected = expected_session.clone();
                            pending.set(Some(DiagnosticAction::Probe));
                            probe_result.set(None);
                            probe_tasks.query(async move {
                                let core = paws_core::shared_core();
                                let before = core.runtime_status_projection().map_err(|e| e.to_string())?;
                                if !before.vpn_running || expected.is_none() || before.vpn_session_id != expected {
                                    return Err(translate_ui(locale, tr::probe_session_changed()));
                                }
                                let delay = tokio::time::timeout(std::time::Duration::from_secs(10), core.test_proxy_delay_via_controller(&group, Some(&url), Some(8000)))
                                    .await.map_err(|_| translate_ui(locale, tr::probe_timeout()))?.map_err(|e| e.to_string())?;
                                let after = core.runtime_status_projection().map_err(|e| e.to_string())?;
                                if !after.vpn_running || after.vpn_session_id != expected {
                                    return Err(translate_ui(locale, tr::probe_session_changed()));
                                }
                                Ok(translate_ui(locale, tr::probe_success(delay.to_string(), group)))
                            }, move |result| { pending.set(None); probe_result.set(Some(result)); });
                        },
                    }
                    OperationNotice { result: probe_result(), locale }
                }
            })}
            row { height: spacing::LG }
            {card(translate_ui(locale, tr::watch_title()), Some(translate_ui(locale, tr::watch_description())), rsx! {
                FieldGroup {
                    Field {
                        orientation: FieldOrientation::Horizontal,
                        FieldContent {
                            FieldTitle { content: translate_ui(locale, tr::watch_enable()) }
                            FieldDescription { content: translate_ui(locale, tr::watch_switch_hint()), inset: true }
                        }
                        Switch {
                            checked: Some(watch.enabled),
                            disabled: busy || (!watch.enabled && (!selector || invalid_url)),
                            on_change: move |enabled| {
                                toggle_watch(if enabled { paws_core::ServiceWatchPolicy { enabled, group: toggle_group.clone(), url: target() } } else { paws_core::ServiceWatchPolicy::default() });
                            },
                        }
                        if pending() == Some(DiagnosticAction::Watch) {
                            Spinner { size: spacing::LG }
                        }
                    }
                    if watch.enabled {
                        FieldDescription { content: translate_ui(locale, tr::watch_current(watch.group.clone(), watch.url.clone())), inset: true }
                        OperationButton {
                            label: translate_ui(locale, tr::watch_save_target()),
                            icon: "save",
                            variant: ButtonVariant::Outline,
                            disabled: busy || !changed || !selector || invalid_url,
                            loading: pending() == Some(DiagnosticAction::Watch),
                            onclick: move |_| update_watch(paws_core::ServiceWatchPolicy { enabled: true, group: save_group.clone(), url: target() }),
                        }
                    }
                    if !selector {
                        FieldDescription { content: translate_ui(locale, tr::diagnostics_selector_only()), inset: true }
                    }
                    OperationNotice { result: watch_result(), locale }
                }
            })}
        }
    };
    scaffold(Route::Diagnostics {}, rsx! {}, body)
}

fn valid_probe_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}
