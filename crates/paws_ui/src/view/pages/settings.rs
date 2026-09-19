use super::super::*;
use crate::settings_draft::{
    DnsDraft, NetworkDraft, SettingsBaseline, SettingsDraft, SettingsSection, SettingsValues,
    VpnDraft,
};
use std::cell::Cell;

pub(crate) fn settings_page() -> Element {
    let stores = use_context::<UiStores>();
    let services = use_context::<UiServices>();
    let notifications = use_context::<NotificationCenter>();
    let runtime = arkit::use_runtime_handle();
    let current = stores.preferences.read().clone();
    let settings = stores.settings.read().clone();
    let initial = settings_baseline(&settings);
    let mut form = use_signal(move || SettingsDraft::new(initial));
    let alive = use_hook(|| Rc::new(Cell::new(true)));
    let drop_alive = alive.clone();
    use_drop(move || drop_alive.set(false));
    use_effect(move || {
        let next = settings_baseline(&stores.settings.read());
        let mut updated = form.peek().clone();
        updated.observe(next);
        if *form.peek() != updated {
            form.set(updated);
        }
    });
    let draft = form.read().clone();
    let busy = draft.pending.is_some();
    let blocked = busy || draft.incoming.is_some() || draft.baseline.profile_id.is_none();
    let dns_servers_value = draft.values.dns.servers.clone();
    let dns_fallbacks_value = draft.values.dns.fallbacks.clone();
    let dns_policy_value = draft.values.dns.policy.clone();
    let system_proxy_value = draft.values.vpn.system_proxy;
    let dns_hijacking_value = draft.values.vpn.dns_hijacking;
    let allow_bypass_value = draft.values.vpn.allow_bypass;
    let controller_allow_lan_value = draft.values.network.allow_lan;
    let mixed_enabled_value = draft.values.network.mixed_enabled;
    let controller_enabled_value = draft.values.network.controller_enabled;
    let mixed_port_value = draft.values.network.mixed_port.clone();
    let controller_port_value = draft.values.network.controller_port.clone();
    let vpn_stack_label_value =
        match paws_model::VpnStack::try_from(draft.values.vpn.stack.as_str()) {
            Ok(paws_model::VpnStack::Smoltcp) => "smoltcp".to_owned(),
            Ok(paws_model::VpnStack::Lwip) => "lwIP".to_owned(),
            Err(_) => draft.values.vpn.stack.clone(),
        };
    let stack_options = vec!["smoltcp".to_owned(), "lwIP".to_owned()];
    let stack_selected_label = vpn_stack_label_value.clone();
    let vpn_dirty = draft.dirty(SettingsSection::Vpn);
    let dns_dirty = draft.dirty(SettingsSection::Dns);
    let network_dirty = draft.dirty(SettingsSection::Network);
    let vpn_runtime = runtime.clone();
    let dns_runtime = runtime.clone();
    let network_runtime = runtime.clone();
    let vpn_alive = alive.clone();
    let dns_alive = alive.clone();
    let network_alive = alive.clone();
    let vpn_services = services.clone();
    let dns_services = services.clone();
    let network_services = services.clone();
    let copy_alive = alive.clone();
    let controller_loopback_addr = format!("127.0.0.1:{controller_port_value}");
    let controller_lan_description = format!(
        "{}:{controller_port_value}",
        translate_ui(current.locale, tr::page_tr_221())
    );
    let controller_listen_description = format!(
        "{} 0.0.0.0:{controller_port_value}",
        translate_ui(current.locale, tr::page_tr_222())
    );
    let controller_secret = settings.controller_access.secret.clone();
    let controller_secret_label = controller_secret
        .as_deref()
        .map(mask_controller_secret)
        .unwrap_or_else(|| translate_ui(current.locale, tr::hard_zh_027()));

    let body = rsx! {
        column {
            width: "100%",
            if draft.baseline.profile_id.is_none() {
                text { content: translate_ui(current.locale, tr::feedback_active_profile_required()), font_color: danger() }
            }
            if draft.incoming.is_some() {
                text { content: translate_ui(current.locale, tr::settings_draft_conflict()), font_color: warning() }
                FlatButton {
                    disabled: Some(busy),
                    onclick: move |_| form.write().reload(),
                    text { content: translate_ui(current.locale, tr::settings_reload_current()) }
                }
            }
            if let Some(error) = draft.error.clone() {
                text { content: error, font_color: danger() }
            }
            if busy {
                Spinner { size: 18.0 }
            }
            {card(
                translate_ui(current.locale, tr::page_tr_223()),
                Some(translate_ui(current.locale, tr::page_tr_224())),
                rsx! {
                    FieldGroup {
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_225()) }
                                FieldDescription { content: translate_ui(current.locale, tr::settings_system_proxy_unsupported()), inset: true }
                            }
                            text { content: if system_proxy_value { "⚠" } else { "—" }, font_color: subtle() }
                        }
                        if system_proxy_value {
                            FlatButton {
                                disabled: Some(busy),
                                onclick: move |_| form.write().values.vpn.system_proxy = false,
                                text { content: translate_ui(current.locale, tr::settings_remove_unsupported_system_proxy()) }
                            }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_227()) }
                                FieldDescription { content: translate_ui(current.locale, tr::page_tr_228()), inset: true }
                            }
                            Switch { checked: Some(dns_hijacking_value), on_change: move |value| if !busy { form.write().values.vpn.dns_hijacking = value } }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_229()) }
                                FieldDescription { content: translate_ui(current.locale, tr::settings_bypass_unsupported()), inset: true }
                            }
                            text { content: if allow_bypass_value { "⚠" } else { "—" }, font_color: subtle() }
                        }
                        if allow_bypass_value {
                            FlatButton {
                                disabled: Some(busy),
                                onclick: move |_| form.write().values.vpn.allow_bypass = false,
                                text { content: translate_ui(current.locale, tr::settings_remove_unsupported_bypass()) }
                            }
                        }
                        row { height: 12.0 }
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_231()) }
                            Select {
                                options: stack_options,
                                selected: Some(vpn_stack_label_value.clone()),
                                default_selected: stack_selected_label.clone(),
                                default_open: false,
                                on_select: move |label: String| {
                                    if !busy {
                                        if let Ok(stack) = paws_model::VpnStack::try_from(label.as_str()) {
                                            form.write().values.vpn.stack = stack.as_str().to_owned();
                                        }
                                    }
                                },
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(blocked || !vpn_dirty),
                            onclick: move |_| save_settings(form, SettingsSection::Vpn, vpn_runtime.clone(), vpn_alive.clone(), notifications, vpn_services.clone(), current.locale),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_232()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(current.locale, tr::page_tr_233()),
                Some(translate_ui(current.locale, tr::page_tr_234())),
                rsx! {
                    FieldGroup {
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: match current.locale { UiLocale::ZhCn => "启用混合代理服务", UiLocale::En => "Enable mixed proxy service" } }
                                FieldDescription { content: match current.locale { UiLocale::ZhCn => "仅在 VPN 已连接时监听代理端口", UiLocale::En => "Listen on the proxy port only while VPN is connected" }, inset: true }
                            }
                            Switch { checked: Some(mixed_enabled_value), on_change: move |value| if !busy { form.write().values.network.mixed_enabled = value } }
                        }
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_235()) }
                            Input {
                                value: Some(mixed_port_value.clone()),
                                placeholder: Some("7890".to_owned()),
                                width: Some("100%".into()),
                                disabled: busy,
                                on_change: move |value| form.write().values.network.mixed_port = value,
                            }
                        }
                        FieldDescription {
                            content: translate_ui(current.locale, tr::page_tr_236()),
                            inset: true,
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: match current.locale { UiLocale::ZhCn => "启用 Controller 服务", UiLocale::En => "Enable controller service" } }
                                FieldDescription { content: match current.locale { UiLocale::ZhCn => "仅在 VPN 已连接时监听控制端口", UiLocale::En => "Listen on the controller port only while VPN is connected" }, inset: true }
                            }
                            Switch { checked: Some(controller_enabled_value), on_change: move |value| if !busy { form.write().values.network.controller_enabled = value } }
                        }
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_237()) }
                            Input {
                                value: Some(controller_port_value.clone()),
                                placeholder: Some("9090".to_owned()),
                                width: Some("100%".into()),
                                disabled: busy,
                                on_change: move |value| form.write().values.network.controller_port = value,
                            }
                        }
                        Field {
                            orientation: FieldOrientation::Horizontal,
                            FieldContent {
                                FieldTitle { content: translate_ui(current.locale, tr::page_tr_238()) }
                                FieldDescription { content: controller_listen_description.clone(), inset: true }
                            }
                            Switch { checked: Some(controller_allow_lan_value), on_change: move |value| if !busy { form.write().values.network.allow_lan = value } }
                        }
                        row { height: 12.0 }
                        column {
                            width: "100%",
                            text { content: translate_ui(current.locale, tr::page_tr_239()), font_size: typography::XS, font_color: subtle() }
                            text { content: controller_loopback_addr.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                        }
                        if controller_allow_lan_value {
                            column {
                                width: "100%",
                                margin_top: 12.0,
                                text { content: translate_ui(current.locale, tr::page_tr_240()), font_size: typography::XS, font_color: subtle() }
                                text { content: controller_lan_description.clone(), margin_top: 4.0, font_size: typography::SM, font_color: text_color() }
                                text { content: "Authorization: Bearer <secret>", margin_top: 4.0, font_size: typography::XS, font_color: subtle() }
                            }
                            row {
                                width: "100%",
                                margin_top: 12.0,
                                align_items: "center",
                                column {
                                    width: "72%",
                                    text { content: translate_ui(current.locale, tr::page_tr_241()), font_size: typography::XS, font_color: subtle() }
                                    text { content: controller_secret_label.clone(), margin_top: 4.0, font_size: typography::SM, font_weight: 600, font_color: text_color() }
                                }
                                if let Some(secret) = controller_secret.clone() {
                                    FlatButton {
                                        variant: FlatButtonVariant::Outline,
                                        size: ButtonSize::Sm,
                                        onclick: move |_| copy_controller_secret(runtime.clone(), copy_alive.clone(), notifications, current.locale, secret.clone()),
                                        {arkit::icon("copy", 14.0, text_color())}
                                        text { content: translate_ui(current.locale, tr::page_tr_242()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: text_color() }
                                    }
                                }
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(blocked || !network_dirty),
                            onclick: move |_| save_settings(form, SettingsSection::Network, network_runtime.clone(), network_alive.clone(), notifications, network_services.clone(), current.locale),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_243()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                "DNS",
                Some(translate_ui(current.locale, tr::page_tr_244())),
                rsx! {
                    FieldGroup {
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_245()) }
                            Textarea {
                                value: Some(dns_servers_value.clone()),
                                height: Some(92.0),
                                width: Some("100%".into()),
                                disabled: busy,
                                on_change: move |value| form.write().values.dns.servers = value,
                            }
                        }
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_276()) }
                            Textarea {
                                value: Some(dns_fallbacks_value.clone()),
                                height: Some(76.0),
                                width: Some("100%".into()),
                                disabled: busy,
                                on_change: move |value| form.write().values.dns.fallbacks = value,
                            }
                        }
                        Field {
                            FieldLabel { content: translate_ui(current.locale, tr::page_tr_246()) }
                            Textarea {
                                value: Some(dns_policy_value.clone()),
                                height: Some(104.0),
                                width: Some("100%".into()),
                                disabled: busy,
                                on_change: move |value| form.write().values.dns.policy = value,
                            }
                        }
                        row { height: 12.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Primary,
                            width: Some("100%".into()),
                            disabled: Some(blocked || !dns_dirty),
                            onclick: move |_| save_settings(form, SettingsSection::Dns, dns_runtime.clone(), dns_alive.clone(), notifications, dns_services.clone(), current.locale),
                            {arkit::icon("save", 16.0, primary_text())}
                            text { content: translate_ui(current.locale, tr::page_tr_247()), margin_left: 8.0, font_size: typography::SM, font_weight: 600, font_color: primary_text() }
                        }
                    }
                }
            )}
        }
    };
    scaffold(Route::Settings {}, rsx! {}, body)
}

fn mask_controller_secret(secret: &str) -> String {
    let characters = secret.chars().collect::<Vec<_>>();
    if characters.len() <= 16 {
        return "•".repeat(characters.len());
    }
    format!(
        "{}…{}",
        characters[..4].iter().collect::<String>(),
        characters[characters.len() - 4..]
            .iter()
            .collect::<String>(),
    )
}

fn settings_values(
    vpn: &paws_model::VpnOptions,
    ports: paws_model::NetworkPortConfig,
    allow_lan: bool,
) -> SettingsValues {
    SettingsValues {
        dns: DnsDraft {
            servers: vpn.dns_servers.join(", "),
            fallbacks: vpn.dns_fallbacks.join(", "),
            policy: vpn
                .dns_nameserver_policy
                .iter()
                .map(|(matcher, servers)| format!("{matcher} = {}", servers.join(", ")))
                .collect::<Vec<_>>()
                .join("\n"),
        },
        vpn: VpnDraft {
            system_proxy: vpn.system_proxy,
            dns_hijacking: vpn.dns_hijacking,
            allow_bypass: vpn.allow_bypass,
            stack: vpn.stack.clone(),
        },
        network: NetworkDraft {
            mixed_port: ports.mixed_port.to_string(),
            controller_port: ports.controller_port.to_string(),
            mixed_enabled: ports.mixed_enabled,
            controller_enabled: ports.controller_enabled,
            allow_lan,
        },
    }
}

fn settings_baseline(settings: &SettingsProjection) -> SettingsBaseline {
    SettingsBaseline {
        profile_id: settings.active_profile.clone(),
        revision: settings.config_revision,
        values: settings_values(
            &settings.vpn,
            settings.network_ports,
            settings.controller_access.allow_lan,
        ),
    }
}

fn save_settings(
    mut form: Signal<SettingsDraft>,
    section: SettingsSection,
    runtime: arkit::RuntimeHandle,
    alive: Rc<Cell<bool>>,
    notifications: NotificationCenter,
    services: UiServices,
    locale: UiLocale,
) {
    let Some((profile_id, revision, values)) = form.write().begin(section) else {
        return;
    };
    let core = paws_core::shared_core();
    // Capture the actual owner before work starts, never a historical running boolean.
    let expected_session = match core.runtime_status_projection() {
        Ok(snapshot) => snapshot.vpn_session_id.filter(|_| snapshot.vpn_running),
        Err(error) => {
            form.write().fail(error.to_string());
            return;
        }
    };
    let (expected_session, vpn_operation_id) =
        services.begin_owned_vpn_operation(expected_session, VpnCommandAction::Restart);
    let task = runtime.tokio().spawn(async move {
        let result = match section {
            SettingsSection::Dns => {
                let servers = parse_dns_servers_text(&values.dns.servers);
                if servers.is_empty() {
                    return Err(translate_ui(locale, tr::feedback_dns_upstream_required()));
                }
                let fallbacks = parse_dns_servers_text(&values.dns.fallbacks);
                let policy = parse_dns_policy_text(&values.dns.policy, locale)?;
                core.set_profile_dns_config_checked(
                    &profile_id,
                    revision,
                    servers,
                    fallbacks,
                    policy,
                )
                .await
            }
            SettingsSection::Vpn => {
                core.set_profile_vpn_config_checked(
                    &profile_id,
                    revision,
                    values.vpn.system_proxy,
                    values.vpn.dns_hijacking,
                    values.vpn.allow_bypass,
                    values.vpn.stack,
                )
                .await
            }
            SettingsSection::Network => {
                let parse_port = |value: &str, label: String| {
                    value.trim().parse::<u16>().map_err(|_| {
                        format!(
                            "{label}{}",
                            translate_ui(locale, tr::network_port_invalid_suffix())
                        )
                    })
                };
                let ports = paws_model::NetworkPortConfig {
                    mixed_port: parse_port(
                        &values.network.mixed_port,
                        translate_ui(locale, tr::mixed_proxy_port()),
                    )?,
                    controller_port: parse_port(
                        &values.network.controller_port,
                        translate_ui(locale, tr::controller_port()),
                    )?,
                    mixed_enabled: values.network.mixed_enabled,
                    controller_enabled: values.network.controller_enabled,
                };
                ports.validate().map_err(|error| error.to_string())?;
                core.set_profile_network_config_checked(
                    &profile_id,
                    revision,
                    ports,
                    values.network.allow_lan,
                )
                .await
            }
        };
        let saved = result.map_err(|error| error.to_string())?;
        let options_json =
            serde_json::to_string(&saved.vpn_options).map_err(|error| error.to_string());
        let baseline = SettingsBaseline {
            profile_id: saved.active_profile,
            revision: saved.revisions.config_revision,
            values: settings_values(
                &saved.vpn_options,
                saved.network_ports,
                saved.controller_access.allow_lan,
            ),
        };
        let restart = if let Some(owner) = expected_session {
            match options_json {
                Ok(options) => {
                    crate::bridge::request_restart_vpn(
                        owner,
                        saved.revisions.config_revision,
                        options,
                    )
                    .await
                }
                Err(error) => Err(error.to_string()),
            }
        } else {
            Ok(crate::bridge::VpnOperationOutcome::Completed(false))
        };
        Ok::<_, String>((baseline, restart))
    });
    // The core owns a started disk transaction through completion. Only its UI
    // acknowledgement is page-owned; never cancel a transaction midway on pop.
    arkit::dioxus_core::spawn_forever(async move {
        let result = task
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
        runtime.queue_ui(move || match result {
            Ok((baseline, restart)) => {
                let (requested, error, unconfirmed) =
                    services.finish_vpn_followup(vpn_operation_id, restart);
                if !alive.get() {
                    return;
                }
                let message = if let Some(message) = unconfirmed {
                    format!(
                        "{} {message}",
                        translate_ui(locale, tr::settings_saved_restart_unconfirmed())
                    )
                } else if let Some(error) = error {
                    format!(
                        "{} {error}",
                        translate_ui(locale, tr::settings_saved_restart_failed())
                    )
                } else if requested {
                    translate_ui(locale, tr::settings_saved_restarted())
                } else {
                    translate_ui(locale, tr::settings_saved_next_connection())
                };
                form.write().finish(section, baseline);
                notifications.publish(message);
            }
            Err(error) => {
                if let Some(operation_id) = vpn_operation_id {
                    services.finish_vpn_failure(operation_id);
                }
                if alive.get() {
                    form.write().fail(error);
                }
            }
        });
    });
}

fn copy_controller_secret(
    runtime: arkit::RuntimeHandle,
    alive: Rc<Cell<bool>>,
    notifications: NotificationCenter,
    locale: UiLocale,
    secret: String,
) {
    let task = runtime
        .tokio()
        .spawn(async move { crate::bridge::copy_text(secret).await });
    arkit::dioxus_core::spawn_forever(async move {
        let message = match task.await {
            Ok(Ok(())) => translate_ui(locale, tr::hard_zh_028()),
            Ok(Err(error)) => format!("{}{}", translate_ui(locale, tr::hard_zh_029()), error),
            Err(error) => format!("{}{}", translate_ui(locale, tr::hard_zh_030()), error),
        };
        runtime.queue_ui(move || {
            if alive.get() {
                notifications.publish(message);
            }
        });
    });
}
