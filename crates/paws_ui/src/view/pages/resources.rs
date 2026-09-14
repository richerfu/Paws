use super::super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VirtualResourcePalette {
    surface: u32,
    foreground: u32,
    muted: u32,
    muted_foreground: u32,
    border: u32,
    success: u32,
    warning: u32,
    danger: u32,
}

#[derive(Clone, PartialEq, Eq)]
enum VirtualResourceRow {
    Summary {
        active_profile_name: String,
        engine_loaded: bool,
        mode_label: String,
        enabled_rule_count: usize,
        total_rule_count: usize,
        total_provider_count: usize,
        ready_geodata_count: usize,
        total_geodata_count: usize,
    },
    GeodataHeader {
        ready_count: usize,
        total_count: usize,
    },
    Geodata(paws_model::GeodataFileSummary),
    GeodataEmpty,
    ProvidersHeader,
    Provider(Box<paws_model::ProviderSummary>),
    ProvidersEmpty,
    RulesHeader {
        has_active_profile: bool,
    },
    Rule(paws_model::RuleSummary),
    RulesEmpty,
    Footer,
}

#[derive(Clone, PartialEq)]
struct VirtualResourceListState {
    rows: Rc<Vec<VirtualResourceRow>>,
    all_rules: Rc<Vec<paws_model::RuleSummary>>,
    locale: UiLocale,
    palette: VirtualResourcePalette,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum VirtualResourceRowKey {
    Summary,
    GeodataHeader,
    Geodata { name: String, path: String },
    GeodataEmpty,
    ProvidersHeader,
    Provider { provider_type: String, name: String },
    ProvidersEmpty,
    RulesHeader,
    Rule { profile_id: String, id: String },
    RulesEmpty,
    Footer,
}

pub(crate) fn resources_page() -> Element {
    let services = use_context::<UiServices>();
    let page_tasks = use_page_tasks();
    let cleanup_services = services.clone();
    use_drop(move || cleanup_services.cancel_rule_import());
    let local_editors = use_local_rule_editors();
    let lookup_services = services.clone();
    let lookup_editors = local_editors.clone();
    let import_services = services.clone();
    let import_tasks = page_tasks.clone();
    let manual_services = services.clone();
    let manual_editors = local_editors.clone();
    let health_services = services.clone();
    let provider_services = services.clone();
    let toggle_services = services.clone();
    let reorder_services = services.clone();
    let delete_services = services.clone();
    let mut query = use_signal(String::new);
    let geodata_detail = use_signal(|| None::<paws_model::GeodataFileSummary>);
    let provider_detail = use_signal(|| None::<String>);
    let stores = use_context::<UiStores>();
    let operations = use_context::<UiOperationStores>();
    let locale = stores.preferences.read().locale;
    let session = stores.session.read();
    let profiles = stores.profiles.read();
    let proxies = stores.proxies.read();
    let resources = stores.resources.read();
    let query_value = query();
    let active_profile_name = profiles
        .profiles
        .iter()
        .find(|profile| profile.active)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| translate_ui(locale, tr::page_tr_154()));
    let enabled_rule_count = resources.rules.iter().filter(|rule| rule.enabled).count();
    let total_rule_count = resources.rules.len();
    let total_provider_count = resources.providers.len();
    let ready_geodata_count = resources.geodata.iter().filter(|file| file.exists).count();
    let total_geodata_count = resources.geodata.len();
    let mode_label = match proxies.mode {
        RuntimeMode::Rule => translate_ui(locale, tr::page_tr_164()),
        RuntimeMode::Global => translate_ui(locale, tr::page_tr_165()),
        RuntimeMode::Direct => translate_ui(locale, tr::page_tr_166()),
    };
    let providers = resources
        .providers
        .iter()
        .filter(|provider| matches_provider_query(provider, &query_value))
        .cloned()
        .collect::<Vec<_>>();
    let rules = resources
        .rules
        .iter()
        .filter(|rule| matches_rule_query(rule, &query_value))
        .cloned()
        .collect::<Vec<_>>();
    let geodata = resources
        .geodata
        .iter()
        .filter(|file| matches_geodata_query(file, &query_value))
        .cloned()
        .collect::<Vec<_>>();
    let selected_geodata = geodata_detail();
    let selected_provider = provider_detail().and_then(|name| {
        resources
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .cloned()
    });
    let mut rows = Vec::with_capacity(8 + geodata.len() + providers.len() + rules.len());
    rows.push(VirtualResourceRow::Summary {
        active_profile_name,
        engine_loaded: session.engine_loaded,
        mode_label,
        enabled_rule_count,
        total_rule_count,
        total_provider_count,
        ready_geodata_count,
        total_geodata_count,
    });
    rows.push(VirtualResourceRow::GeodataHeader {
        ready_count: ready_geodata_count,
        total_count: total_geodata_count,
    });
    if geodata.is_empty() {
        rows.push(VirtualResourceRow::GeodataEmpty);
    } else {
        rows.extend(geodata.into_iter().map(VirtualResourceRow::Geodata));
    }
    rows.push(VirtualResourceRow::ProvidersHeader);
    if providers.is_empty() {
        rows.push(VirtualResourceRow::ProvidersEmpty);
    } else {
        rows.extend(
            providers
                .into_iter()
                .map(Box::new)
                .map(VirtualResourceRow::Provider),
        );
    }
    rows.push(VirtualResourceRow::RulesHeader {
        has_active_profile: profiles.active_profile.is_some(),
    });
    if rules.is_empty() {
        rows.push(VirtualResourceRow::RulesEmpty);
    } else {
        rows.extend(rules.into_iter().map(VirtualResourceRow::Rule));
    }
    rows.push(VirtualResourceRow::Footer);
    let theme = use_theme();
    let palette = VirtualResourcePalette {
        surface: theme.colors.card,
        foreground: theme.colors.foreground,
        muted: theme.colors.muted,
        muted_foreground: theme.colors.muted_foreground,
        border: theme.colors.border,
        success: success(),
        warning: warning(),
        danger: theme.colors.destructive,
    };
    let rows = Rc::new(rows);
    let all_rules = Rc::new(resources.rules.clone());
    let body = rsx! {
        column {
            width: "100%",
            height: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(translate_ui(locale, tr::resources_search_placeholder())),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row { height: 12.0 }
            row {
                layout_weight: 1.0,
                width: "100%",
                VirtualResourceList {
                    rows,
                    all_rules,
                    locale,
                    palette,
                    geodata_detail,
                    provider_detail,
                    resource_operations: operations.resources,
                    diagnostic_operations: operations.diagnostics,
                    on_import_rules: move |_| import_services.import_rules(import_tasks.clone()),
                    on_open_manual_rule: move |_| manual_services.open_manual_rule_editor(manual_editors.clone(), None, String::new(), String::new()),
                    on_healthcheck_provider: move |provider_name: String| health_services.healthcheck_proxy_provider(provider_name),
                    on_refresh_provider: move |(provider_type, provider_name): (String, String)| provider_services.refresh_provider(provider_type, provider_name),
                    on_set_rule_enabled: move |(profile_id, rule_id, enabled): (String, String, bool)| toggle_services.set_rule_enabled(profile_id, rule_id, enabled),
                    on_reorder_rules: move |(profile_id, ordered_ids): (String, Vec<String>)| reorder_services.reorder_rules(profile_id, ordered_ids),
                    on_delete_rule: move |(profile_id, rule_id): (String, String)| delete_services.delete_rule(profile_id, rule_id),
                }
            }
        }
    };
    let actions = rsx! {
        row {
            FlatButton { variant: FlatButtonVariant::Ghost, size: ButtonSize::Icon, onclick: move |_| lookup_services.open_rule_lookup(lookup_editors.clone()), {arkit::icon("route", 17.0, text_color())} }
            FlatButton { variant: FlatButtonVariant::Ghost, size: ButtonSize::Icon, onclick: move |_| services.refresh_all_providers(), {arkit::icon("refresh-cw", 17.0, text_color())} }
        }
    };
    let page = fixed_scaffold(Route::Resources {}, actions, body);
    rsx! {
        {page}
        if let Some(file) = selected_geodata {
            {geodata_detail_dialog(locale, file, geodata_detail)}
        }
        if let Some(provider) = selected_provider {
            ProviderDetailDialog { locale, provider, selected: provider_detail }
        }
        ManualRuleDialog { local: local_editors.clone() }
        RuleLookupDialog { local: local_editors }
    }
}

#[component]
fn VirtualResourceList(
    rows: Rc<Vec<VirtualResourceRow>>,
    all_rules: Rc<Vec<paws_model::RuleSummary>>,
    locale: UiLocale,
    palette: VirtualResourcePalette,
    geodata_detail: Signal<Option<paws_model::GeodataFileSummary>>,
    provider_detail: Signal<Option<String>>,
    resource_operations: Signal<ResourceOperationState>,
    diagnostic_operations: Signal<DiagnosticOperationState>,
    on_import_rules: EventHandler<()>,
    on_open_manual_rule: EventHandler<()>,
    on_healthcheck_provider: EventHandler<String>,
    on_refresh_provider: EventHandler<(String, String)>,
    on_set_rule_enabled: EventHandler<(String, String, bool)>,
    on_reorder_rules: EventHandler<(String, Vec<String>)>,
    on_delete_rule: EventHandler<(String, String)>,
) -> Element {
    let item_keys = rows
        .iter()
        .map(|row| match row {
            VirtualResourceRow::Summary { .. } => VirtualResourceRowKey::Summary,
            VirtualResourceRow::GeodataHeader { .. } => VirtualResourceRowKey::GeodataHeader,
            VirtualResourceRow::Geodata(file) => VirtualResourceRowKey::Geodata {
                name: file.name.clone(),
                path: file.path.clone(),
            },
            VirtualResourceRow::GeodataEmpty => VirtualResourceRowKey::GeodataEmpty,
            VirtualResourceRow::ProvidersHeader => VirtualResourceRowKey::ProvidersHeader,
            VirtualResourceRow::Provider(provider) => VirtualResourceRowKey::Provider {
                provider_type: provider.provider_type.clone(),
                name: provider.name.clone(),
            },
            VirtualResourceRow::ProvidersEmpty => VirtualResourceRowKey::ProvidersEmpty,
            VirtualResourceRow::RulesHeader { .. } => VirtualResourceRowKey::RulesHeader,
            VirtualResourceRow::Rule(rule) => VirtualResourceRowKey::Rule {
                profile_id: rule.profile_id.clone(),
                id: rule.id.clone(),
            },
            VirtualResourceRow::RulesEmpty => VirtualResourceRowKey::RulesEmpty,
            VirtualResourceRow::Footer => VirtualResourceRowKey::Footer,
        })
        .collect::<Vec<_>>();
    let next_list_state = VirtualResourceListState {
        rows,
        all_rules,
        locale,
        palette,
    };
    let mut list_state = use_signal(|| next_list_state.clone());
    use_effect(use_reactive(
        (&next_list_state,),
        move |(next_list_state,)| {
            if *list_state.peek() != next_list_state {
                list_state.set(next_list_state);
            }
        },
    ));

    // Detached rows observe list_state and operation Signals directly.
    let stamps = item_keys
        .into_iter()
        .map(|id| VirtualItemStamp::new(id, ()))
        .collect();
    let source = use_virtual_items(VirtualKind::List, stamps, move |index| {
        rsx! {
            VirtualResourceRowView {
                index,
                list_state,
                geodata_detail,
                provider_detail,
                resource_operations,
                diagnostic_operations,
                on_import_rules,
                on_open_manual_rule,
                on_healthcheck_provider,
                on_refresh_provider,
                on_set_rule_enabled,
                on_reorder_rules,
                on_delete_rule,
            }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            scroll_bar: "off",
            list_cached_count: 18_i32,
        }
    }
}

#[component]
fn VirtualResourceRowView(
    index: u32,
    list_state: Signal<VirtualResourceListState>,
    mut geodata_detail: Signal<Option<paws_model::GeodataFileSummary>>,
    mut provider_detail: Signal<Option<String>>,
    resource_operations: Signal<ResourceOperationState>,
    diagnostic_operations: Signal<DiagnosticOperationState>,
    on_import_rules: EventHandler<()>,
    on_open_manual_rule: EventHandler<()>,
    on_healthcheck_provider: EventHandler<String>,
    on_refresh_provider: EventHandler<(String, String)>,
    on_set_rule_enabled: EventHandler<(String, String, bool)>,
    on_reorder_rules: EventHandler<(String, Vec<String>)>,
    on_delete_rule: EventHandler<(String, String)>,
) -> Element {
    let current = list_state.read();
    let Some(row) = current.rows.get(index as usize).cloned() else {
        return rsx! {};
    };
    let locale = current.locale;
    let palette = current.palette;
    let all_rules = current.all_rules.clone();
    drop(current);

    match row {
        VirtualResourceRow::Summary {
            active_profile_name,
            engine_loaded,
            mode_label,
            enabled_rule_count,
            total_rule_count,
            total_provider_count,
            ready_geodata_count,
            total_geodata_count,
        } => virtual_resource_card(
            translate_ui(locale, tr::page_tr_181()),
            Some(active_profile_name),
            rsx! {
                column {
                    width: "100%",
                    {virtual_resource_info_row(
                        translate_ui(locale, tr::page_tr_182()),
                        if engine_loaded { translate_ui(locale, tr::page_tr_183()) } else { translate_ui(locale, tr::page_tr_184()) },
                        palette,
                    )}
                    {virtual_resource_info_row(translate_ui(locale, tr::page_tr_185()), mode_label, palette)}
                    {virtual_resource_info_row(translate_ui(locale, tr::page_tr_186()), format!("{enabled_rule_count}/{total_rule_count}"), palette)}
                    {virtual_resource_info_row("Provider", total_provider_count.to_string(), palette)}
                    {virtual_resource_info_row("GeoData", format!("{ready_geodata_count}/{total_geodata_count}"), palette)}
                }
            },
            palette,
        ),
        VirtualResourceRow::GeodataHeader {
            ready_count,
            total_count,
        } => {
            let status_color = if ready_count == total_count && total_count > 0 {
                palette.success
            } else {
                palette.warning
            };
            rsx! {
                row {
                    width: "100%",
                    height: 64.0,
                    margin_top: 12.0,
                    padding_left: spacing::LG,
                    padding_right: spacing::LG,
                    align_items: "center",
                    background_color: palette.surface,
                    border_width: 1.0,
                    border_color: palette.border,
                    border_radius: radius::LG,
                    text { content: "GeoData", font_size: typography::SM, font_weight: 600, font_color: palette.foreground }
                    row { layout_weight: 1.0 }
                    text {
                        content: format!("{ready_count}/{total_count} {}", translate_ui(locale, tr::page_tr_187())),
                        font_size: typography::XS,
                        font_weight: 500,
                        font_color: status_color,
                    }
                }
            }
        }
        VirtualResourceRow::Geodata(file) => {
            let detail = file.clone();
            let status = if file.exists {
                translate_ui(locale, tr::page_tr_179())
            } else {
                translate_ui(locale, tr::page_tr_180())
            };
            let metadata = if file.exists {
                format!("{status} · {}", format_total(file.bytes.unwrap_or(0)))
            } else {
                status.to_owned()
            };
            let status_color = if file.exists {
                palette.success
            } else {
                palette.danger
            };
            rsx! {
                button {
                    width: "100%",
                    height: 76.0,
                    margin_top: 6.0,
                    padding_left: 14.0,
                    padding_right: 12.0,
                    background_color: palette.surface,
                    border_width: 1.0,
                    border_color: palette.border,
                    border_radius: radius::LG,
                    onclick: move |_| geodata_detail.set(Some(detail.clone())),
                    row {
                        width: "100%",
                        align_items: "center",
                        row {
                            width: 36.0,
                            height: 36.0,
                            align_items: "center",
                            justify_content: "center",
                            background_color: palette.muted,
                            border_radius: radius::LG,
                            {arkit::icon("file-text", 17.0, status_color)}
                        }
                        column {
                            layout_weight: 1.0,
                            margin_left: 11.0,
                            align_items: "start",
                            text { content: file.name, width: "100%", font_size: typography::SM, font_weight: 600, font_color: palette.foreground, max_lines: 1 }
                            text { content: metadata, width: "100%", margin_top: 3.0, font_size: typography::XS, font_color: status_color, max_lines: 1 }
                        }
                        {arkit::icon("chevron-right", 15.0, palette.muted_foreground)}
                    }
                }
            }
        }
        VirtualResourceRow::GeodataEmpty => {
            virtual_resource_empty("file-x", translate_ui(locale, tr::page_tr_188()), palette)
        }
        VirtualResourceRow::ProvidersHeader => {
            virtual_resource_section_label(translate_ui(locale, tr::page_tr_189()), palette)
        }
        VirtualResourceRow::Provider(provider) => rsx! {
            VirtualProviderRow {
                locale,
                palette,
                provider: *provider,
                provider_detail,
                diagnostic_operations,
                on_healthcheck: on_healthcheck_provider,
                on_refresh: on_refresh_provider,
            }
        },
        VirtualResourceRow::ProvidersEmpty => {
            virtual_resource_empty("database", translate_ui(locale, tr::page_tr_190()), palette)
        }
        VirtualResourceRow::RulesHeader { has_active_profile } => rsx! {
            ResourceRulesHeader {
                locale,
                palette,
                has_active_profile,
                resource_operations,
                on_import: on_import_rules,
                on_open_manual: on_open_manual_rule,
            }
        },
        VirtualResourceRow::Rule(rule) => rsx! {
            column {
                width: "100%",
                {rule_view(
                    locale,
                    palette,
                    &all_rules,
                    rule,
                    on_set_rule_enabled,
                    on_reorder_rules,
                    on_delete_rule,
                )}
                row { height: 6.0 }
            }
        },
        VirtualResourceRow::RulesEmpty => virtual_resource_empty(
            "list-checks",
            translate_ui(locale, tr::page_tr_193()),
            palette,
        ),
        VirtualResourceRow::Footer => rsx! { row { height: spacing::MD } },
    }
}

fn virtual_resource_card(
    title: impl Into<String>,
    subtitle: Option<String>,
    body: Element,
    palette: VirtualResourcePalette,
) -> Element {
    rsx! {
        column {
            width: "100%",
            padding: spacing::LG,
            align_items: "start",
            background_color: palette.surface,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
            text { content: title.into(), font_size: typography::SM, line_height: 20.0, font_weight: 600, font_color: palette.foreground }
            if let Some(subtitle) = subtitle {
                text { content: subtitle, margin_top: spacing::XXS, font_size: typography::XS, line_height: 18.0, font_color: palette.muted_foreground }
            }
            column { width: "100%", margin_top: spacing::MD, {body} }
        }
    }
}

fn virtual_resource_info_row(
    label: impl Into<String>,
    value: impl Into<String>,
    palette: VirtualResourcePalette,
) -> Element {
    rsx! {
        row {
            width: "100%",
            min_height: 34.0,
            align_items: "center",
            text { content: label.into(), font_size: typography::XS, font_color: palette.muted_foreground }
            row { layout_weight: 1.0 }
            text { content: value.into(), font_size: typography::XS, font_weight: 500, font_color: palette.foreground, max_lines: 2, text_align: "end" }
        }
    }
}

fn virtual_resource_section_label(
    label: impl Into<String>,
    palette: VirtualResourcePalette,
) -> Element {
    rsx! {
        row {
            width: "100%",
            height: 48.0,
            margin_top: 10.0,
            align_items: "center",
            text { content: label.into(), font_size: typography::SM, font_weight: 600, font_color: palette.foreground }
        }
    }
}

fn virtual_resource_empty(
    icon: &'static str,
    message: impl Into<String>,
    palette: VirtualResourcePalette,
) -> Element {
    rsx! {
        row {
            width: "100%",
            height: 72.0,
            padding_left: spacing::LG,
            padding_right: spacing::LG,
            align_items: "center",
            justify_content: "center",
            background_color: palette.surface,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
            {arkit::icon(icon, 16.0, palette.muted_foreground)}
            text { content: message.into(), margin_left: 8.0, font_size: typography::XS, font_color: palette.muted_foreground, max_lines: 2 }
        }
    }
}

#[component]
fn ResourceRulesHeader(
    locale: UiLocale,
    palette: VirtualResourcePalette,
    has_active_profile: bool,
    resource_operations: Signal<ResourceOperationState>,
    on_import: EventHandler<()>,
    on_open_manual: EventHandler<()>,
) -> Element {
    let import_loading = resource_operations.read().rule_import_loading;
    let import_disabled = import_loading || !has_active_profile;
    rsx! {
        row {
            width: "100%",
            height: 52.0,
            margin_top: 10.0,
            align_items: "center",
            text { content: translate_ui(locale, tr::resources_rules_title()), font_size: typography::SM, font_weight: 600, font_color: palette.foreground }
            row { layout_weight: 1.0 }
            button {
                height: 36.0,
                padding_left: 8.0,
                padding_right: 8.0,
                background_color: palette.surface,
                border_width: 0.0,
                border_radius: radius::MD,
                enabled: !import_disabled,
                opacity: if import_disabled { 0.5 } else { 1.0 },
                onclick: move |_| {
                    if !resource_operations.peek().rule_import_loading {
                        on_import.call(());
                    }
                },
                row {
                    align_items: "center",
                    {arkit::icon(if import_loading { "loader-circle" } else { "file-up" }, 14.0, palette.foreground)}
                    text { content: translate_ui(locale, tr::resources_import_rules()), margin_left: 5.0, font_size: typography::XS, font_weight: 600, font_color: palette.foreground }
                }
            }
            button {
                height: 36.0,
                padding_left: 8.0,
                padding_right: 8.0,
                background_color: palette.surface,
                border_width: 0.0,
                border_radius: radius::MD,
                enabled: has_active_profile,
                opacity: if has_active_profile { 1.0 } else { 0.5 },
                onclick: move |_| on_open_manual.call(()),
                row {
                    align_items: "center",
                    {arkit::icon("plus", 14.0, palette.foreground)}
                    text { content: translate_ui(locale, tr::page_tr_192()), margin_left: 5.0, font_size: typography::XS, font_weight: 600, font_color: palette.foreground }
                }
            }
        }
    }
}

#[component]
fn VirtualProviderRow(
    locale: UiLocale,
    palette: VirtualResourcePalette,
    provider: paws_model::ProviderSummary,
    mut provider_detail: Signal<Option<String>>,
    diagnostic_operations: Signal<DiagnosticOperationState>,
    on_healthcheck: EventHandler<String>,
    on_refresh: EventHandler<(String, String)>,
) -> Element {
    let diagnostic_pending = diagnostic_operations.read().pending.is_some();
    let refresh_provider_type = provider.provider_type.clone();
    let refresh_provider_name = provider.name.clone();
    let health_provider_name = provider.name.clone();
    let detail_provider_name = provider.name.clone();
    let member_count = provider.members.len();
    let alive_count = provider
        .members
        .iter()
        .filter(|member| member.alive)
        .count();
    let can_healthcheck = provider.provider_type == "proxy" && provider.health_check_enabled;
    let provider_status = if provider.last_refresh_error.is_some() {
        translate_ui(locale, tr::page_tr_167())
    } else if provider
        .vehicle_type
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("inline"))
    {
        translate_ui(locale, tr::page_tr_168())
    } else if provider.cache_exists {
        translate_ui(locale, tr::page_tr_169())
    } else {
        translate_ui(locale, tr::page_tr_170())
    };
    let cache_status = if provider.cache_exists {
        format_total(provider.cache_bytes.unwrap_or(0))
    } else {
        translate_ui(locale, tr::page_tr_173())
    };
    let interval = provider
        .interval_seconds
        .map(|value| format!("{value}s"))
        .unwrap_or_else(|| "-".to_owned());
    let title = truncate_text(&provider.name, 38);
    let subtitle = format!(
        "{} · {}",
        provider.provider_type,
        provider.vehicle_type.clone().unwrap_or_default()
    );
    rsx! {
        column {
            width: "100%",
            margin_bottom: 8.0,
            padding: spacing::LG,
            align_items: "start",
            background_color: palette.surface,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
            text { content: title, font_size: typography::SM, line_height: 20.0, font_weight: 600, font_color: palette.foreground }
            text { content: subtitle, margin_top: spacing::XXS, font_size: typography::XS, line_height: 18.0, font_color: palette.muted_foreground }
            column {
                width: "100%",
                margin_top: spacing::MD,
                {virtual_resource_info_row(translate_ui(locale, tr::page_tr_171()), provider_status, palette)}
                {virtual_resource_info_row(translate_ui(locale, tr::page_tr_172()), cache_status, palette)}
                {virtual_resource_info_row(translate_ui(locale, tr::page_tr_174()), interval, palette)}
                if provider.provider_type == "proxy" {
                    {virtual_resource_info_row(translate_ui(locale, tr::page_tr_175()), format!("{alive_count}/{member_count}"), palette)}
                }
                if let Some(error) = provider.last_refresh_error.clone() {
                    text { content: compact(&error), margin_top: 6.0, font_size: typography::XS, font_color: palette.danger, max_lines: 2 }
                }
                row { height: 4.0 }
                row {
                    width: "100%",
                    justify_content: "end",
                    button {
                        height: 36.0,
                        padding_left: 8.0,
                        padding_right: 8.0,
                        background_color: palette.surface,
                        border_width: 0.0,
                        border_radius: radius::MD,
                        onclick: move |_| provider_detail.set(Some(detail_provider_name.clone())),
                        row {
                            align_items: "center",
                            {arkit::icon("list", 14.0, palette.foreground)}
                            text { content: translate_ui(locale, tr::page_tr_176()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: palette.foreground }
                        }
                    }
                    if can_healthcheck {
                        button {
                            height: 36.0,
                            padding_left: 8.0,
                            padding_right: 8.0,
                            background_color: palette.surface,
                            border_width: 0.0,
                            border_radius: radius::MD,
                            enabled: !diagnostic_pending,
                            opacity: if diagnostic_pending { 0.5 } else { 1.0 },
                            onclick: move |_| on_healthcheck.call(health_provider_name.clone()),
                            row {
                                align_items: "center",
                                {arkit::icon("heart-pulse", 14.0, palette.foreground)}
                                text { content: translate_ui(locale, tr::page_tr_177()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: palette.foreground }
                            }
                        }
                    }
                    button {
                        height: 36.0,
                        padding_left: 8.0,
                        padding_right: 8.0,
                        background_color: palette.surface,
                        border_width: 0.0,
                        border_radius: radius::MD,
                        onclick: move |_| on_refresh.call((refresh_provider_type.clone(), refresh_provider_name.clone())),
                        row {
                            align_items: "center",
                            {arkit::icon("refresh-cw", 14.0, palette.foreground)}
                            text { content: translate_ui(locale, tr::page_tr_178()), margin_left: 6.0, font_size: typography::XS, font_weight: 600, font_color: palette.foreground }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RuleLookupDialog(local: LocalRuleEditors) -> Element {
    let services = use_context::<UiServices>();
    let open = local.signal.read().lookup.is_some();
    if !open {
        return rsx! {};
    }
    let close_editors = local.clone();
    rsx! {
        FlatDialog {
            open,
            on_close: move |_| services.close_rule_lookup(close_editors.clone()),
            RuleLookupDialogContent { local }
        }
    }
}

#[component]
fn RuleLookupDialogContent(local: LocalRuleEditors) -> Element {
    let services = use_context::<UiServices>();
    let query_services = services.clone();
    let add_services = services.clone();
    let stores = use_context::<UiStores>();
    let locale = stores.preferences.read().locale;
    let editors = local.signal.read().clone();
    let profiles = stores.profiles.read();
    let proxies = stores.proxies.read();
    let Some(lookup) = editors.lookup else {
        return rsx! {};
    };
    let can_lookup =
        !lookup.submitting && !lookup.query.trim().is_empty() && profiles.active_profile.is_some();
    let query_editors = local.clone();
    let add_editors = local.clone();
    let lookup_editors = local;

    rsx! {
        DialogHeader {
            title: translate_ui(locale, tr::page_tr_195()),
            description: Some(translate_ui(locale, tr::page_tr_196())),
        }
        row { height: 14.0 }
        Input {
            value: Some(lookup.query.clone()),
            placeholder: Some("example.com / 203.0.113.1".to_owned()),
            width: Some("100%".into()),
            disabled: lookup.submitting,
            on_change: move |value| query_services.set_rule_lookup_query(query_editors.clone(), value),
        }
        if profiles.active_profile.is_none() {
            text {
                content: translate_ui(locale, tr::page_tr_197()),
                margin_top: 9.0,
                font_size: typography::XS,
                line_height: 16.0,
                font_color: warning(),
            }
        }
        if proxies.mode != RuntimeMode::Rule {
            text {
                content: translate_ui(locale, tr::page_tr_198()),
                margin_top: 9.0,
                font_size: typography::XS,
                line_height: 16.0,
                font_color: warning(),
            }
        }
        if let Some(error) = lookup.error {
            text {
                content: error,
                margin_top: 9.0,
                font_size: typography::XS,
                line_height: 16.0,
                font_color: danger(),
            }
        }
        if let Some(result) = lookup.result {
            column {
                width: "100%",
                margin_top: 12.0,
                padding: 12.0,
                border_width: 1.0,
                border_color: if result.matched { success() } else { line() },
                border_radius: radius::LG,
                background_color: muted(),
                row {
                    width: "100%",
                    align_items: "center",
                    {arkit::icon(if result.matched { "route" } else { "x" }, 16.0, if result.matched { success() } else { subtle() })}
                    text {
                        content: if result.matched { translate_ui(locale, tr::page_tr_199()) } else { translate_ui(locale, tr::page_tr_200()) },
                        margin_left: 7.0,
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: if result.matched { success() } else { text_color() },
                    }
                }
                if let Some(rule_line) = result.rule_line {
                    text {
                        content: rule_line,
                        width: "100%",
                        margin_top: 8.0,
                        font_size: typography::XS,
                        line_height: 18.0,
                        font_weight: 600,
                        font_color: text_color(),
                        max_lines: 3,
                        text_overflow: "ellipsis",
                    }
                } else {
                    text {
                        content: translate_ui(locale, tr::page_tr_201()),
                        width: "100%",
                        margin_top: 8.0,
                        font_size: typography::XS,
                        line_height: 18.0,
                        font_color: subtle(),
                    }
                }
                row { height: 7.0 }
                {info_row(
                    translate_ui(locale, tr::page_tr_202()),
                    match result.input_kind {
                        paws_core::RuleLookupInputKind::Domain => translate_ui(locale, tr::page_tr_203()),
                        paws_core::RuleLookupInputKind::Ip => "IP".to_owned(),
                    },
                )}
                if result.resolution_attempted {
                    {info_row(
                        translate_ui(locale, tr::page_tr_204()),
                        result.resolved_ip.unwrap_or_else(|| translate_ui(locale, tr::page_tr_205())),
                    )}
                }
                {info_row(translate_ui(locale, tr::page_tr_206()), result.target)}
                row { height: 8.0 }
                FlatButton {
                    variant: FlatButtonVariant::Outline,
                    width: "100%",
                    onclick: move |_| add_services.add_rule_from_lookup(add_editors.clone()),
                    {arkit::icon("plus", 15.0, text_color())}
                    text {
                        content: translate_ui(locale, tr::page_tr_207()),
                        margin_left: 7.0,
                        font_size: typography::XS,
                        font_weight: 600,
                        font_color: text_color(),
                    }
                }
            }
        }
        DialogFooter {
            FlatButton {
                variant: FlatButtonVariant::Primary,
                width: "100%",
                disabled: Some(!can_lookup),
                onclick: move |_| services.lookup_rule(lookup_editors.clone()),
                if lookup.submitting {
                    Spinner { size: 16.0, color: Some(primary_text()) }
                } else {
                    {arkit::icon("search", 16.0, primary_text())}
                }
                text {
                    content: if lookup.submitting { translate_ui(locale, tr::page_tr_208()) } else { translate_ui(locale, tr::page_tr_209()) },
                    margin_left: 8.0,
                    font_size: typography::SM,
                    font_weight: 600,
                    font_color: primary_text(),
                }
            }
        }
    }
}

#[component]
fn ProviderDetailDialog(
    locale: UiLocale,
    provider: paws_model::ProviderSummary,
    mut selected: Signal<Option<String>>,
) -> Element {
    let services = use_context::<UiServices>();
    let operations = use_context::<UiOperationStores>();
    let pending = operations.diagnostics.read().pending.is_some();
    let provider_name = provider.name.clone();
    let health_url = provider
        .health_check_url
        .clone()
        .unwrap_or_else(|| "https://www.gstatic.com/generate_204".to_owned());
    let expected_status = provider.expected_status.clone();
    let members = provider.members.into_iter().map(|member| {
        let member_services = services.clone();
        let check_provider = provider_name.clone();
        let check_proxy = member.name.clone();
        let check_url = health_url.clone();
        let check_expected = expected_status.clone();
        let status = if member.alive {
            translate_ui(locale, tr::page_tr_210())
        } else {
            translate_ui(locale, tr::page_tr_211())
        };
        let delay = member
            .delay_ms
            .map(|delay| format!("{delay} ms"))
            .unwrap_or_else(|| translate_ui(locale, tr::page_tr_212()));
        rsx! {
            row {
                width: "100%",
                height: 50.0,
                padding_left: 10.0,
                padding_right: 8.0,
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text { content: truncate_text(&member.name, 34), width: "100%", font_size: typography::XS, font_weight: 600, font_color: text_color(), max_lines: 1 }
                    text { content: format!("{} · {} · {}", member.proxy_type, status, delay), margin_top: 3.0, width: "100%", font_size: typography::XS, font_color: if member.alive { success() } else { danger() }, max_lines: 1 }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Icon,
                    disabled: Some(pending),
                    onclick: move |_| member_services.healthcheck_provider_proxy(check_provider.clone(), check_proxy.clone(), check_url.clone(), check_expected.clone()),
                    {arkit::icon("gauge", 15.0, text_color())}
                }
            }
            Separator {}
        }
    }).collect::<Vec<_>>();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: truncate_text(&provider_name, 42),
                description: Some(format!("{} {}", members.len(), translate_ui(locale, tr::page_tr_213()))),
            }
            row { height: 12.0 }
            if members.is_empty() {
                text { content: translate_ui(locale, tr::page_tr_214()), font_size: typography::XS, font_color: subtle() }
            } else {
                column {
                    width: "100%",
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: radius::LG,
                    clip: true,
                    {members.into_iter()}
                }
            }
        }
    }
}

fn geodata_detail_dialog(
    locale: UiLocale,
    file: paws_model::GeodataFileSummary,
    mut selected: Signal<Option<paws_model::GeodataFileSummary>>,
) -> Element {
    let availability = if file.exists {
        translate_ui(locale, tr::page_tr_215())
    } else {
        translate_ui(locale, tr::page_tr_216())
    };
    let size = file
        .bytes
        .map(format_total)
        .unwrap_or_else(|| "-".to_owned());
    let updated_at = file
        .updated_at
        .as_deref()
        .and_then(time_format::format_unix_seconds)
        .or(file.updated_at.clone())
        .unwrap_or_else(|| "-".to_owned());
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: file.name,
                description: Some(availability.to_owned()),
            }
            row { height: 16.0 }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: radius::LG,
                padding_left: spacing::MD,
                padding_right: spacing::MD,
                {info_row(translate_ui(locale, tr::page_tr_171()), availability)}
                Separator {}
                {info_row(translate_ui(locale, tr::page_tr_217()), size)}
                Separator {}
                {info_row(translate_ui(locale, tr::page_tr_218()), updated_at)}
            }
            row { height: 14.0 }
            text { content: translate_ui(locale, tr::page_tr_219()), font_size: typography::XS, font_weight: 500, font_color: subtle() }
            row { height: 6.0 }
            row {
                width: "100%",
                padding: 11.0,
                background_color: muted(),
                border_radius: radius::LG,
                text {
                    content: file.path,
                    width: "100%",
                    font_size: typography::XS,
                    line_height: 17.0,
                    font_color: text_color(),
                    max_lines: 5,
                }
            }
        }
    }
}

fn rule_view(
    locale: UiLocale,
    palette: VirtualResourcePalette,
    all_rules: &[paws_model::RuleSummary],
    rule: paws_model::RuleSummary,
    on_set_enabled: EventHandler<(String, String, bool)>,
    on_reorder: EventHandler<(String, Vec<String>)>,
    on_delete: EventHandler<(String, String)>,
) -> Element {
    let editable = rule.source != "profile-yaml";
    let rule_source = if editable {
        rule.source.clone()
    } else {
        translate_ui(locale, tr::hard_zh_022())
    };
    let toggle_profile = rule.profile_id.clone();
    let toggle_id = rule.id.clone();
    let delete_profile = rule.profile_id.clone();
    let delete_id = rule.id.clone();
    let enabled = rule.enabled;
    // Subscription YAML rules are immutable and can number in the tens of
    // thousands. Reorder lookup is meaningful only for manual rules; doing it
    // for every YAML row turns page construction into O(n²) work.
    let (up, down) = if editable {
        (
            reordered_rule_ids(all_rules, &rule.profile_id, &rule.id, -1),
            reordered_rule_ids(all_rules, &rule.profile_id, &rule.id, 1),
        )
    } else {
        (None, None)
    };
    rsx! {
        column {
            width: "100%",
            height: 88.0,
            padding_top: 8.0,
            padding_right: 8.0,
            padding_bottom: 8.0,
            padding_left: 10.0,
            background_color: palette.surface,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: radius::LG,
            clip: true,
            row {
                width: "100%",
                height: 32.0,
                align_items: "center",
                text {
                    content: format!("#{}", rule.order + 1),
                    font_size: typography::XS,
                    font_weight: 600,
                    font_color: if enabled { palette.success } else { palette.muted_foreground },
                    max_lines: 1,
                }
                row {
                    layout_weight: 1.0,
                    margin_left: 7.0,
                    margin_right: 4.0,
                    text {
                        content: rule_source,
                        width: "100%",
                        font_size: typography::XS,
                        font_color: palette.muted_foreground,
                        max_lines: 1,
                    }
                }
                if editable {
                    {compact_rule_action(if enabled { "toggle-right" } else { "toggle-left" }, if enabled { palette.success } else { palette.muted_foreground }, move || on_set_enabled.call((toggle_profile.clone(), toggle_id.clone(), !enabled)), palette)}
                    if let Some(ids) = up {
                        {compact_rule_action("arrow-up", palette.muted_foreground, { let on_reorder = on_reorder.clone(); let profile_id = rule.profile_id.clone(); move || on_reorder.call((profile_id.clone(), ids.clone())) }, palette)}
                    }
                    if let Some(ids) = down {
                        {compact_rule_action("arrow-down", palette.muted_foreground, { let on_reorder = on_reorder.clone(); let profile_id = rule.profile_id.clone(); move || on_reorder.call((profile_id.clone(), ids.clone())) }, palette)}
                    }
                    {compact_rule_action("trash-2", palette.danger, move || on_delete.call((delete_profile.clone(), delete_id.clone())), palette)}
                } else {
                    {virtual_resource_pill(translate_ui(locale, tr::page_tr_220()), palette.success, palette)}
                }
            }
            text {
                content: truncate_text(&rule.line, 180),
                width: "100%",
                margin_top: 5.0,
                font_size: typography::XS,
                line_height: 16.0,
                font_color: palette.foreground,
                max_lines: 2,
            }
        }
    }
}

fn compact_rule_action<F>(
    icon: &'static str,
    color: u32,
    mut action: F,
    palette: VirtualResourcePalette,
) -> Element
where
    F: FnMut() + 'static,
{
    rsx! {
        button {
            width: 32.0,
            height: 32.0,
            padding: 0.0,
            background_color: palette.surface,
            border_width: 0.0,
            border_radius: radius::MD,
            onclick: move |_| action(),
            row {
                width: "100%",
                height: "100%",
                align_items: "center",
                justify_content: "center",
                {arkit::icon(icon, 15.0, color)}
            }
        }
    }
}

fn virtual_resource_pill(
    label: impl Into<String>,
    color: u32,
    palette: VirtualResourcePalette,
) -> Element {
    rsx! {
        row {
            height: 24.0,
            padding_left: 8.0,
            padding_right: 8.0,
            align_items: "center",
            justify_content: "center",
            background_color: palette.muted,
            border_radius: radius::FULL,
            text { content: label.into(), font_size: typography::XS, font_weight: 600, font_color: color, max_lines: 1 }
        }
    }
}

fn reordered_rule_ids(
    rules: &[paws_model::RuleSummary],
    profile_id: &str,
    rule_id: &str,
    delta: isize,
) -> Option<Vec<String>> {
    let mut ordered = rules
        .iter()
        .filter(|rule| rule.profile_id == profile_id && rule.source != "profile-yaml")
        .collect::<Vec<_>>();
    ordered.sort_by_key(|rule| rule.order);
    let index = ordered.iter().position(|rule| rule.id == rule_id)?;
    let target = index.checked_add_signed(delta)?;
    if target >= ordered.len() {
        return None;
    }
    ordered.swap(index, target);
    Some(ordered.into_iter().map(|rule| rule.id.clone()).collect())
}
