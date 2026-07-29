use crate::activity_filter::{
    matches_connection_query, matches_request_filter, request_connection_query, RequestStatusFilter,
};
use crate::l10n::{strings, UiLocale, UiStrings};
use crate::log_filter::{matches_log_filter_normalized, normalize_log_query, LogLevelFilter};
use crate::mode_feedback::mode_changed_message;
use crate::notification::NotificationCenter;
use crate::profile_filter::matches_profile_query;
use crate::profile_refresh_feedback::{
    profile_activation_message, profile_batch_refresh_message, profile_delete_message,
};
use crate::provider_refresh_feedback::provider_batch_refresh_message;
use crate::proxy_grid::{
    flatten_proxy_groups, proxy_selection_chain, stabilize_proxy_items, ProxyGridItem,
};
use crate::resource_filter::{matches_geodata_query, matches_provider_query, matches_rule_query};
use crate::rule_feedback::rule_import_message;
use crate::settings_feedback::settings_saved_message;
use crate::subscription_scan::{parse_scanned_subscription, ScannedSubscriptionError};
use crate::time_format;
use crate::traffic_history::summarize_traffic_history;
use crate::ui_preferences::{LanguagePreference, ThemePreference, UiPreferences};
use crate::vpn_feedback::{vpn_command_is_pending, vpn_command_message, VpnCommandAction};
use crate::yaml_summary::summarize_yaml_edit;
use hmeta_model::{
    ManualRuleMatchKind, ManualRuleMutationKind, ManualRuleSpec, RuntimeMode, RuntimeSnapshot,
    TrafficHistoryPoint, VpnLifecycle,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub(crate) struct Command<M>(Vec<Pin<Box<dyn Future<Output = M> + Send>>>);

impl<M: Send + 'static> Command<M> {
    fn none() -> Self {
        Self(Vec::new())
    }

    fn perform<F, O, Map>(future: F, map: Map) -> Self
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
        Map: FnOnce(O) -> M + Send + 'static,
    {
        Self(vec![Box::pin(async move { map(future.await) })])
    }

    fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self(tasks.into_iter().flat_map(|task| task.0).collect())
    }

    fn into_futures(self) -> impl Iterator<Item = Pin<Box<dyn Future<Output = M> + Send>>> {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    RefreshSnapshot,
    SnapshotLoaded(RuntimeSnapshot),
    TickSnapshot(RuntimeSnapshot),
    SetLanguagePreference(LanguagePreference),
    SetThemePreference(ThemePreference),
    StartStopVpn,
    VpnCommandFinished(Result<VpnCommandResult, String>),
    VpnCommandSnapshot(RuntimeSnapshot),
    SetMode(RuntimeMode),
    ModeChanged(Result<ModeChangeResult, String>),
    SelectProxy {
        group: String,
        proxy: String,
    },
    UnfixProxy {
        group: String,
    },
    ProxySelected(Result<RuntimeSnapshot, String>),
    TestAllProxyDelays,
    AllProxyDelaysTested(Result<ProxyDelayBatchResult, String>),
    FlushDnsCache,
    FlushFakeIpCache,
    HealthcheckProxyProvider {
        provider_name: String,
    },
    HealthcheckProviderProxy {
        provider_name: String,
        proxy_name: String,
        url: String,
        expected_status: Option<String>,
    },
    ControllerDiagnosticFinished(Result<(RuntimeSnapshot, String), String>),
    CloseConnection(String),
    ConnectionClosed(Result<RuntimeSnapshot, String>),
    OpenRuleLookup,
    CloseRuleLookup,
    SetRuleLookupQuery(String),
    LookupRule,
    AddRuleFromLookup,
    RuleLookedUp {
        lookup_id: u64,
        result: Result<hmeta_core::RuleLookupResult, String>,
    },
    OpenManualRuleEditor {
        connection_id: Option<String>,
        domain: String,
        destination_ip: String,
    },
    CloseManualRuleEditor,
    SetManualRuleMatchKind(ManualRuleMatchKind),
    SetManualRuleValue(String),
    SetManualRuleTarget(String),
    SetManualRuleDisconnect(bool),
    SaveManualRule,
    ManualRuleSaved(Result<ManualRuleSaveResult, String>),
    CloseAllConnections,
    AllConnectionsClosed(Result<RuntimeSnapshot, String>),
    OpenExternalUrl(String),
    ExternalUrlOpened(Result<(), String>),
    ClearRequestHistory,
    RequestHistoryCleared(Result<RuntimeSnapshot, String>),
    ToggleLogRecording,
    LogRecordingChanged(Result<LogRecordingChangeResult, String>),
    ExportLogArchive(String),
    LogArchiveExported(Result<String, String>),
    DeleteLogArchive(String),
    LogArchiveDeleted(Result<LogArchiveDeleteResult, String>),
    ResetProfileImportFeedback,
    ImportLocalProfile,
    ScanProfileSubscription {
        name: String,
    },
    LocalProfileImportFinished(Result<ProfileImportResult, String>),
    ImportProfileFromUrl {
        url: String,
        name: String,
    },
    ProfileImportFinished(Result<ProfileImportResult, String>),
    ImportRules,
    RulesImported(Result<RuleImportResult, String>),
    ActivateProfile(String),
    ProfileActivated(Result<ProfileActivationResult, String>),
    DeleteProfile(String),
    ProfileDeleted(Result<ProfileDeleteResult, String>),
    RefreshProfile(String),
    ProfileRefreshed(Result<ProfileRefreshResult, String>),
    RefreshAllProfiles,
    ProfilesRefreshed(Result<ProfileBatchRefreshResult, String>),
    RestoreProfileBackup(String),
    ProfileBackupRestored(Result<ProfileBackupRestoreResult, String>),
    UpdateProfileSubscription {
        profile_id: String,
        name: String,
        subscription_url: String,
    },
    ProfileSubscriptionUpdated(Result<(RuntimeSnapshot, String), String>),
    ExportProfile(String),
    ProfileExported(Result<String, String>),
    OpenYamlEditor(String),
    SetYamlEditorOpen(bool),
    SetYamlEditorText(String),
    ResetYamlEditorText,
    TestYamlEditor,
    YamlEditorTested(Result<(), String>),
    SaveYamlEditor,
    YamlEditorSaved(Result<RuntimeSnapshot, String>),
    RefreshProvider {
        provider_type: String,
        provider_name: String,
    },
    ProviderRefreshed(Result<ProviderRefreshResult, String>),
    RefreshAllProviders,
    ProvidersRefreshed(Result<ProviderBatchRefreshResult, String>),
    SetRuleEnabled {
        profile_id: String,
        rule_id: String,
        enabled: bool,
    },
    ReorderRules {
        profile_id: String,
        ordered_rule_ids: Vec<String>,
    },
    DeleteRule {
        profile_id: String,
        rule_id: String,
    },
    RulesChanged(Result<RuleChangeResult, String>),
    SaveDnsSettings {
        servers_text: String,
        fallbacks_text: String,
        policy_text: String,
    },
    DnsSettingsSaved(Result<SettingsSaveResult, String>),
    SaveVpnSettings {
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    },
    VpnSettingsSaved(Result<SettingsSaveResult, String>),
}

#[derive(Clone)]
pub(crate) struct State {
    locale: UiLocale,
    preferences: UiPreferences,
    theme_dark: bool,
    snapshot: RuntimeSnapshot,
    profile_import_error: Option<String>,
    profile_import_loading: bool,
    profile_import_succeeded: bool,
    rule_import_loading: bool,
    yaml_editor_open: bool,
    yaml_editor_profile_id: Option<String>,
    yaml_editor_profile_name: String,
    yaml_editor_text: String,
    yaml_editor_original: String,
    yaml_editor_error: Option<String>,
    yaml_editor_saving: bool,
    yaml_editor_testing: bool,
    vpn_command_pending: Option<VpnCommandAction>,
    proxy_selection_pending: Option<(String, String)>,
    proxy_delay_loading: bool,
    controller_diagnostic_pending: Option<String>,
    log_recording: hmeta_core::LogRecordingStatus,
    log_recording_pending: bool,
    log_archive_export_pending: Option<String>,
    log_archive_delete_pending: Option<String>,
    next_rule_lookup_id: u64,
    rule_lookup: Option<RuleLookupState>,
    manual_rule_editor: Option<ManualRuleEditorState>,
    notifications: NotificationCenter,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleLookupState {
    id: u64,
    query: String,
    submitting: bool,
    result: Option<hmeta_core::RuleLookupResult>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManualRuleEditorState {
    connection_id: Option<String>,
    domain: String,
    destination_ip: String,
    match_kind: ManualRuleMatchKind,
    value: String,
    target: String,
    disconnect_after_save: bool,
    submitting: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManualRuleSaveResult {
    snapshot: RuntimeSnapshot,
    applied: hmeta_core::ManualRuleApplyResult,
    connection_close_requested: bool,
    connection_close_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyDelayBatchResult {
    snapshot: RuntimeSnapshot,
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ModeChangeResult {
    snapshot: RuntimeSnapshot,
    mode: RuntimeMode,
}

#[derive(Debug, Clone)]
pub(crate) struct VpnCommandResult {
    snapshot: RuntimeSnapshot,
    action: VpnCommandAction,
    profile_name: Option<String>,
    request_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderRefreshResult {
    snapshot: RuntimeSnapshot,
    provider_name: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderBatchRefreshResult {
    snapshot: RuntimeSnapshot,
    attempted_providers: Vec<(String, String)>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileImportResult {
    snapshot: RuntimeSnapshot,
    profile_name: String,
    restart_requested: bool,
    restart_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileRefreshResult {
    snapshot: RuntimeSnapshot,
    profile_name: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileBatchRefreshResult {
    snapshot: RuntimeSnapshot,
    attempted_profile_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileBackupRestoreResult {
    snapshot: RuntimeSnapshot,
    profile_name: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileActivationResult {
    snapshot: RuntimeSnapshot,
    profile_name: String,
    restart_requested: bool,
    restart_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileDeleteResult {
    snapshot: RuntimeSnapshot,
    profile_name: String,
    vpn_action: Option<ProfileDeleteVpnAction>,
    vpn_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProfileDeleteVpnAction {
    Stop,
    Restart,
}

#[derive(Debug, Clone)]
pub(crate) struct SettingsSaveResult {
    snapshot: RuntimeSnapshot,
    restart_requested: bool,
    restart_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleChangeResult {
    snapshot: RuntimeSnapshot,
    restart_requested: bool,
    restart_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleImportResult {
    snapshot: RuntimeSnapshot,
    imported_count: usize,
    reload_error: Option<String>,
    restart_requested: bool,
    restart_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LogRecordingChangeResult {
    snapshot: RuntimeSnapshot,
    status: hmeta_core::LogRecordingStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct LogArchiveDeleteResult {
    file_name: String,
    status: hmeta_core::LogRecordingStatus,
}

impl State {
    pub(crate) fn new(notifications: NotificationCenter) -> Self {
        let core = hmeta_core::shared_core();
        let snapshot = core.snapshot().unwrap_or_default();
        let log_recording = core.log_recording_status().unwrap_or_default();
        let preferences = UiPreferences::load();
        let locale = preferences.language.resolve(&system_language());
        let theme_dark = preferences.theme.resolve_dark(system_color_mode());
        Self {
            locale,
            preferences,
            theme_dark,
            snapshot,
            profile_import_error: None,
            profile_import_loading: false,
            profile_import_succeeded: false,
            rule_import_loading: false,
            yaml_editor_open: false,
            yaml_editor_profile_id: None,
            yaml_editor_profile_name: String::new(),
            yaml_editor_text: String::new(),
            yaml_editor_original: String::new(),
            yaml_editor_error: None,
            yaml_editor_saving: false,
            yaml_editor_testing: false,
            vpn_command_pending: None,
            proxy_selection_pending: None,
            proxy_delay_loading: false,
            controller_diagnostic_pending: None,
            log_recording,
            log_recording_pending: false,
            log_archive_export_pending: None,
            log_archive_delete_pending: None,
            next_rule_lookup_id: 0,
            rule_lookup: None,
            manual_rule_editor: None,
            notifications,
        }
    }

    pub(crate) fn language_preference(&self) -> LanguagePreference {
        self.preferences.language
    }

    pub(crate) fn theme_preference(&self) -> ThemePreference {
        self.preferences.theme
    }

    pub(crate) fn theme_dark(&self) -> bool {
        self.theme_dark
    }

    fn refresh_system_preferences(&mut self) {
        self.locale = self.preferences.language.resolve(&system_language());
        self.theme_dark = self.preferences.theme.resolve_dark(system_color_mode());
    }
}

pub(crate) fn reduce(state: &mut State, message: Action) -> Command<Action> {
    match message {
        Action::RefreshSnapshot => Command::perform(load_snapshot(), Action::SnapshotLoaded),
        Action::SnapshotLoaded(snapshot) => {
            state.snapshot = snapshot;
            refresh_log_recording_status(state);
            reconcile_vpn_command(state);
            state.refresh_system_preferences();
            Command::none()
        }
        Action::TickSnapshot(snapshot) => {
            state.snapshot = snapshot;
            refresh_log_recording_status(state);
            reconcile_vpn_command(state);
            state.refresh_system_preferences();
            Command::perform(delayed_snapshot(), Action::TickSnapshot)
        }
        Action::SetLanguagePreference(preference) => {
            state.preferences.language = preference;
            state.refresh_system_preferences();
            let message = match state.preferences.save() {
                Ok(()) if state.locale == UiLocale::ZhCn => "语言设置已更新".to_owned(),
                Ok(()) => "Language preference updated".to_owned(),
                Err(error) if state.locale == UiLocale::ZhCn => {
                    format!("语言设置保存失败：{error}")
                }
                Err(error) => format!("Failed to save language preference: {error}"),
            };
            show_toast(state, message)
        }
        Action::SetThemePreference(preference) => {
            state.preferences.theme = preference;
            state.refresh_system_preferences();
            let message = match state.preferences.save() {
                Ok(()) if state.locale == UiLocale::ZhCn => "主题设置已更新".to_owned(),
                Ok(()) => "Theme preference updated".to_owned(),
                Err(error) if state.locale == UiLocale::ZhCn => {
                    format!("主题设置保存失败：{error}")
                }
                Err(error) => format!("Failed to save theme preference: {error}"),
            };
            show_toast(state, message)
        }
        Action::StartStopVpn => {
            if state.vpn_command_pending.is_some()
                || matches!(state.snapshot.vpn_lifecycle, VpnLifecycle::Starting)
            {
                show_toast(
                    state,
                    if state.locale == UiLocale::ZhCn {
                        "正在等待系统 VPN 服务"
                    } else {
                        "Waiting for the system VPN service"
                    }
                    .to_owned(),
                )
            } else if state.snapshot.vpn_running {
                let ui_strings = strings(state.locale);
                state.vpn_command_pending = Some(VpnCommandAction::Stop);
                Command::batch([
                    Command::perform(
                        stop_vpn_command_and_snapshot(ui_strings),
                        Action::VpnCommandFinished,
                    ),
                    Command::perform(delayed_vpn_snapshot(), Action::VpnCommandSnapshot),
                ])
            } else if let Some(profile_id) = state.snapshot.active_profile.clone().or_else(|| {
                state
                    .snapshot
                    .profiles
                    .first()
                    .map(|profile| profile.id.clone())
            }) {
                let profile_name = state
                    .snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| profile_id.clone());
                let ui_strings = strings(state.locale);
                state.vpn_command_pending = Some(VpnCommandAction::Start);
                Command::batch([
                    Command::perform(
                        start_vpn_command_and_snapshot(profile_id, profile_name, ui_strings),
                        Action::VpnCommandFinished,
                    ),
                    Command::perform(delayed_vpn_snapshot(), Action::VpnCommandSnapshot),
                ])
            } else {
                show_toast(
                    state,
                    strings(state.locale).feedback_profile_required.to_owned(),
                )
            }
        }
        Action::VpnCommandFinished(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                if result.request_error.is_some() {
                    state.vpn_command_pending = None;
                } else {
                    reconcile_vpn_command(state);
                }
                show_toast(
                    state,
                    vpn_command_message(
                        result.action,
                        result.profile_name.as_deref(),
                        result.request_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => {
                state.vpn_command_pending = None;
                show_toast(state, error)
            }
        },
        Action::VpnCommandSnapshot(snapshot) => {
            state.snapshot = snapshot;
            reconcile_vpn_command(state);
            if state.vpn_command_pending.is_some() {
                Command::perform(delayed_vpn_snapshot(), Action::VpnCommandSnapshot)
            } else {
                Command::none()
            }
        }
        Action::SetMode(mode) => Command::perform(
            async move {
                let core = hmeta_core::shared_core();
                if mode == RuntimeMode::Global {
                    core.prepare_active_vpn()
                        .await
                        .map_err(|error| error.to_string())?;
                }
                core.set_mode(mode).map_err(|error| error.to_string())?;
                Ok(ModeChangeResult {
                    snapshot: load_snapshot().await,
                    mode,
                })
            },
            Action::ModeChanged,
        ),
        Action::ModeChanged(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    mode_changed_message(result.mode, strings(state.locale)),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_mode_change_failed_prefix,
                    error
                ),
            ),
        },
        Action::SelectProxy { group, proxy } => {
            if state.proxy_selection_pending.is_some() {
                return Command::none();
            }
            let selections = proxy_selection_chain(&state.snapshot.proxy_groups, &group, &proxy);
            state.proxy_selection_pending = Some((group, proxy));
            Command::perform(select_proxy_and_snapshot(selections), Action::ProxySelected)
        }
        Action::UnfixProxy { group } => {
            if state.proxy_selection_pending.is_some() {
                return Command::none();
            }
            state.proxy_selection_pending = Some((group.clone(), String::new()));
            Command::perform(unfix_proxy_and_snapshot(group), Action::ProxySelected)
        }
        Action::ProxySelected(result) => {
            state.proxy_selection_pending = None;
            match result {
                Ok(snapshot) => {
                    state.snapshot = snapshot;
                    Command::none()
                }
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{}",
                        strings(state.locale).feedback_proxy_switch_failed_prefix,
                        error
                    ),
                ),
            }
        }
        Action::TestAllProxyDelays => {
            if state.proxy_delay_loading {
                return Command::none();
            }
            let groups = proxy_groups_for_delay_test(&state.snapshot);
            if groups.is_empty() {
                show_toast(
                    state,
                    strings(state.locale).feedback_proxy_delay_empty.to_owned(),
                )
            } else {
                state.proxy_delay_loading = true;
                Command::perform(
                    test_proxy_delays_and_snapshot(groups),
                    Action::AllProxyDelaysTested,
                )
            }
        }
        Action::AllProxyDelaysTested(result) => {
            state.proxy_delay_loading = false;
            match result {
                Ok(result) => {
                    state.snapshot = result.snapshot;
                    show_toast(
                        state,
                        format!(
                            "{}{}{}{}{}",
                            strings(state.locale).feedback_proxy_delay_batch_prefix,
                            result.succeeded,
                            strings(state.locale).feedback_provider_batch_success_mid,
                            result.failed,
                            strings(state.locale).feedback_provider_batch_failed_suffix
                        ),
                    )
                }
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{}",
                        strings(state.locale).feedback_proxy_delay_batch_failed_prefix,
                        error
                    ),
                ),
            }
        }
        Action::FlushDnsCache => {
            if state.controller_diagnostic_pending.is_some() {
                return Command::none();
            }
            state.controller_diagnostic_pending = Some("dns".to_owned());
            Command::perform(
                async {
                    hmeta_core::shared_core()
                        .flush_dns_cache_via_controller()
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok((
                        load_snapshot().await,
                        "DNS 缓存已清理 / DNS cache flushed".to_owned(),
                    ))
                },
                Action::ControllerDiagnosticFinished,
            )
        }
        Action::FlushFakeIpCache => {
            if state.controller_diagnostic_pending.is_some() {
                return Command::none();
            }
            state.controller_diagnostic_pending = Some("fakeip".to_owned());
            Command::perform(
                async {
                    hmeta_core::shared_core()
                        .flush_fake_ip_cache_via_controller()
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok((
                        load_snapshot().await,
                        "Fake-IP 缓存已清理 / Fake-IP cache flushed".to_owned(),
                    ))
                },
                Action::ControllerDiagnosticFinished,
            )
        }
        Action::HealthcheckProxyProvider { provider_name } => {
            if state.controller_diagnostic_pending.is_some() {
                return Command::none();
            }
            state.controller_diagnostic_pending = Some(format!("provider:{provider_name}"));
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .healthcheck_proxy_provider_via_controller(&provider_name)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok((
                        load_snapshot().await,
                        format!("Provider {provider_name} 健康检查完成"),
                    ))
                },
                Action::ControllerDiagnosticFinished,
            )
        }
        Action::HealthcheckProviderProxy {
            provider_name,
            proxy_name,
            url,
            expected_status,
        } => {
            if state.controller_diagnostic_pending.is_some() {
                return Command::none();
            }
            state.controller_diagnostic_pending =
                Some(format!("provider:{provider_name}:{proxy_name}"));
            Command::perform(
                async move {
                    let delay = hmeta_core::shared_core()
                        .healthcheck_provider_proxy_via_controller(
                            &provider_name,
                            &proxy_name,
                            &url,
                            Some(5000),
                            expected_status.as_deref(),
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok((load_snapshot().await, format!("{proxy_name}: {delay} ms")))
                },
                Action::ControllerDiagnosticFinished,
            )
        }
        Action::ControllerDiagnosticFinished(result) => {
            state.controller_diagnostic_pending = None;
            match result {
                Ok((snapshot, message)) => {
                    state.snapshot = snapshot;
                    show_toast(state, message)
                }
                Err(error) => {
                    show_toast(state, format!("诊断操作失败 / Diagnostic failed: {error}"))
                }
            }
        }
        Action::CloseConnection(connection_id) => Command::perform(
            close_connection_and_snapshot(connection_id),
            Action::ConnectionClosed,
        ),
        Action::ConnectionClosed(result) => match result {
            Ok(snapshot) => {
                state.snapshot = snapshot;
                show_toast(
                    state,
                    strings(state.locale).feedback_connection_closed.to_owned(),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_connection_close_failed_prefix,
                    error
                ),
            ),
        },
        Action::OpenRuleLookup => {
            state.next_rule_lookup_id = state.next_rule_lookup_id.wrapping_add(1);
            state.rule_lookup = Some(RuleLookupState {
                id: state.next_rule_lookup_id,
                query: String::new(),
                submitting: false,
                result: None,
                error: None,
            });
            Command::none()
        }
        Action::CloseRuleLookup => {
            state.rule_lookup = None;
            Command::none()
        }
        Action::SetRuleLookupQuery(query) => {
            if let Some(lookup) = state.rule_lookup.as_mut() {
                if lookup.submitting {
                    return Command::none();
                }
                lookup.query = query;
                lookup.result = None;
                lookup.error = None;
            }
            Command::none()
        }
        Action::LookupRule => {
            let Some(lookup) = state.rule_lookup.as_mut() else {
                return Command::none();
            };
            if lookup.submitting {
                return Command::none();
            }
            if state.snapshot.active_profile.is_none() {
                lookup.error = Some(if state.locale == UiLocale::ZhCn {
                    "请先启用一个订阅配置".to_owned()
                } else {
                    "Activate a profile before querying rules".to_owned()
                });
                return Command::none();
            }
            if lookup.query.trim().is_empty() {
                lookup.error = Some(if state.locale == UiLocale::ZhCn {
                    "请输入域名或 IP 地址".to_owned()
                } else {
                    "Enter a domain name or IP address".to_owned()
                });
                return Command::none();
            }
            lookup.submitting = true;
            lookup.result = None;
            lookup.error = None;
            let lookup_id = lookup.id;
            Command::perform(lookup_rule(lookup.query.clone()), move |result| {
                Action::RuleLookedUp { lookup_id, result }
            })
        }
        Action::RuleLookedUp { lookup_id, result } => {
            if let Some(lookup) = state
                .rule_lookup
                .as_mut()
                .filter(|lookup| lookup.id == lookup_id)
            {
                lookup.submitting = false;
                match result {
                    Ok(result) => {
                        lookup.result = Some(result);
                        lookup.error = None;
                    }
                    Err(error) => {
                        lookup.result = None;
                        lookup.error = Some(error);
                    }
                }
            }
            Command::none()
        }
        Action::AddRuleFromLookup => {
            let Some((input_kind, query)) = state
                .rule_lookup
                .as_ref()
                .and_then(|lookup| lookup.result.as_ref())
                .map(|result| (result.input_kind, result.query.clone()))
            else {
                return Command::none();
            };
            state.rule_lookup = None;
            let (domain, destination_ip) = match input_kind {
                hmeta_core::RuleLookupInputKind::Domain => (query, String::new()),
                hmeta_core::RuleLookupInputKind::Ip => (String::new(), query),
            };
            Command::perform(
                async move {
                    // Let the lookup overlay unmount before publishing the
                    // manual-rule overlay; both share ArkUI's modal host.
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    (domain, destination_ip)
                },
                |(domain, destination_ip)| Action::OpenManualRuleEditor {
                    connection_id: None,
                    domain,
                    destination_ip,
                },
            )
        }
        Action::OpenManualRuleEditor {
            connection_id,
            domain,
            destination_ip,
        } => {
            open_manual_rule_editor(state, connection_id, domain, destination_ip);
            Command::none()
        }
        Action::CloseManualRuleEditor => {
            if !state
                .manual_rule_editor
                .as_ref()
                .is_some_and(|editor| editor.submitting)
            {
                state.manual_rule_editor = None;
            }
            Command::none()
        }
        Action::SetManualRuleMatchKind(match_kind) => {
            if let Some(editor) = state.manual_rule_editor.as_mut() {
                if editor.submitting {
                    return Command::none();
                }
                editor.match_kind = match_kind;
                editor.value = match match_kind {
                    ManualRuleMatchKind::Domain | ManualRuleMatchKind::DomainSuffix => {
                        editor.domain.clone()
                    }
                    ManualRuleMatchKind::IpCidr => editor.destination_ip.clone(),
                };
                editor.error = None;
            }
            Command::none()
        }
        Action::SetManualRuleValue(value) => {
            if let Some(editor) = state.manual_rule_editor.as_mut() {
                if editor.submitting {
                    return Command::none();
                }
                editor.value = value;
                editor.error = None;
            }
            Command::none()
        }
        Action::SetManualRuleTarget(target) => {
            if let Some(editor) = state.manual_rule_editor.as_mut() {
                if editor.submitting {
                    return Command::none();
                }
                editor.target = target;
                editor.error = None;
            }
            Command::none()
        }
        Action::SetManualRuleDisconnect(disconnect) => {
            if let Some(editor) = state.manual_rule_editor.as_mut() {
                if editor.submitting {
                    return Command::none();
                }
                editor.disconnect_after_save = disconnect;
            }
            Command::none()
        }
        Action::SaveManualRule => {
            let Some(editor) = state.manual_rule_editor.as_mut() else {
                return Command::none();
            };
            if editor.submitting {
                return Command::none();
            }
            let Some(profile_id) = state.snapshot.active_profile.clone() else {
                editor.error = Some(if state.locale == UiLocale::ZhCn {
                    "请先启用一个订阅配置".to_owned()
                } else {
                    "Activate a profile before adding a rule".to_owned()
                });
                return Command::none();
            };
            editor.submitting = true;
            editor.error = None;
            let spec = ManualRuleSpec {
                match_kind: editor.match_kind,
                value: editor.value.clone(),
                target: editor.target.clone(),
            };
            let connection_id = editor
                .disconnect_after_save
                .then(|| editor.connection_id.clone())
                .flatten();
            Command::perform(
                apply_manual_rule_and_snapshot(profile_id, spec, connection_id),
                Action::ManualRuleSaved,
            )
        }
        Action::ManualRuleSaved(result) => match result {
            Ok(result) => {
                let message = manual_rule_saved_message(&result, state.locale);
                state.snapshot = result.snapshot;
                state.manual_rule_editor = None;
                show_toast(state, message)
            }
            Err(error) => {
                if let Some(editor) = state.manual_rule_editor.as_mut() {
                    editor.submitting = false;
                    editor.error = Some(error);
                }
                Command::none()
            }
        },
        Action::CloseAllConnections => Command::perform(
            close_all_connections_and_snapshot(),
            Action::AllConnectionsClosed,
        ),
        Action::AllConnectionsClosed(result) => match result {
            Ok(snapshot) => {
                state.snapshot = snapshot;
                show_toast(
                    state,
                    strings(state.locale).feedback_connections_closed.to_owned(),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_connections_close_failed_prefix,
                    error
                ),
            ),
        },
        Action::OpenExternalUrl(url) => Command::perform(
            async move { crate::platform_callbacks::open_external_url(url).await },
            Action::ExternalUrlOpened,
        ),
        Action::ExternalUrlOpened(result) => match result {
            Ok(()) => Command::none(),
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_open_link_failed_prefix,
                    error
                ),
            ),
        },
        Action::ClearRequestHistory => Command::perform(
            clear_request_history_and_snapshot(),
            Action::RequestHistoryCleared,
        ),
        Action::RequestHistoryCleared(result) => match result {
            Ok(snapshot) => {
                state.snapshot = snapshot;
                show_toast(
                    state,
                    strings(state.locale)
                        .feedback_request_history_cleared
                        .to_owned(),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_request_history_clear_failed_prefix,
                    error
                ),
            ),
        },
        Action::ToggleLogRecording => {
            if state.log_recording_pending {
                return Command::none();
            }
            state.log_recording_pending = true;
            let enabled = !state.log_recording.enabled;
            Command::perform(
                set_log_recording_and_snapshot(enabled),
                Action::LogRecordingChanged,
            )
        }
        Action::LogRecordingChanged(result) => {
            state.log_recording_pending = false;
            match result {
                Ok(result) => {
                    state.snapshot = result.snapshot;
                    state.log_recording = result.status;
                    show_toast(
                        state,
                        if state.log_recording.enabled {
                            locale_text(state.locale, "已开始记录日志", "Log recording started")
                        } else {
                            locale_text(state.locale, "已停止记录日志", "Log recording stopped")
                        }
                        .to_owned(),
                    )
                }
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{error}",
                        locale_text(
                            state.locale,
                            "切换日志记录失败：",
                            "Failed to change log recording: "
                        )
                    ),
                ),
            }
        }
        Action::ExportLogArchive(file_name) => {
            if state.log_archive_export_pending.is_some()
                || state.log_archive_delete_pending.is_some()
            {
                return Command::none();
            }
            state.log_archive_export_pending = Some(file_name.clone());
            Command::perform(export_log_archive(file_name), Action::LogArchiveExported)
        }
        Action::LogArchiveExported(result) => {
            state.log_archive_export_pending = None;
            match result {
                Ok(file_name) => show_toast(
                    state,
                    format!(
                        "{}{file_name}",
                        locale_text(state.locale, "日志已导出：", "Log exported: ")
                    ),
                ),
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{error}",
                        locale_text(state.locale, "日志导出失败：", "Failed to export log: ")
                    ),
                ),
            }
        }
        Action::DeleteLogArchive(file_name) => {
            if state.log_archive_export_pending.is_some()
                || state.log_archive_delete_pending.is_some()
            {
                return Command::none();
            }
            state.log_archive_delete_pending = Some(file_name.clone());
            Command::perform(delete_log_archive(file_name), Action::LogArchiveDeleted)
        }
        Action::LogArchiveDeleted(result) => {
            state.log_archive_delete_pending = None;
            match result {
                Ok(result) => {
                    state.log_recording = result.status;
                    show_toast(
                        state,
                        format!(
                            "{}{}",
                            locale_text(state.locale, "日志已删除：", "Log deleted: "),
                            result.file_name
                        ),
                    )
                }
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{error}",
                        locale_text(state.locale, "日志删除失败：", "Failed to delete log: ")
                    ),
                ),
            }
        }
        Action::ResetProfileImportFeedback => {
            state.profile_import_error = None;
            state.profile_import_succeeded = false;
            Command::none()
        }
        Action::ImportLocalProfile => {
            if state.profile_import_loading {
                return Command::none();
            }
            state.profile_import_loading = true;
            state.profile_import_error = None;
            state.profile_import_succeeded = false;
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                import_profile_file_and_snapshot(was_vpn_running, ui_strings),
                Action::LocalProfileImportFinished,
            )
        }
        Action::ScanProfileSubscription { name } => {
            if state.profile_import_loading {
                return Command::none();
            }
            state.profile_import_loading = true;
            state.profile_import_error = None;
            state.profile_import_succeeded = false;
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                scan_profile_subscription_and_snapshot(name, was_vpn_running, ui_strings),
                Action::LocalProfileImportFinished,
            )
        }
        Action::LocalProfileImportFinished(result) => {
            state.profile_import_loading = false;
            match result {
                Ok(result) => {
                    state.snapshot = result.snapshot;
                    state.profile_import_error = None;
                    state.profile_import_succeeded = true;
                    show_toast(
                        state,
                        localized_profile_import_message(
                            &result.profile_name,
                            result.restart_requested,
                            result.restart_error.as_deref(),
                            strings(state.locale),
                        ),
                    )
                }
                Err(error) => {
                    // File-picker cancel should not sticky-error the network form.
                    if picker_was_cancelled(&error) {
                        state.profile_import_error = None;
                        state.profile_import_succeeded = false;
                        Command::none()
                    } else {
                        state.profile_import_error = Some(error.clone());
                        state.profile_import_succeeded = false;
                        show_toast(
                            state,
                            format!(
                                "{}{}",
                                strings(state.locale).profiles_import_failed_prefix,
                                error
                            ),
                        )
                    }
                }
            }
        }
        Action::ImportProfileFromUrl { url, name } => {
            if state.profile_import_loading {
                return Command::none();
            }
            state.profile_import_succeeded = false;
            let url = url.trim().to_owned();
            if url.is_empty() {
                state.profile_import_error = Some(
                    strings(state.locale)
                        .profiles_import_url_required
                        .to_owned(),
                );
                return Command::none();
            }
            let name = match name.trim() {
                "" => None,
                value => Some(value.to_owned()),
            };
            state.profile_import_loading = true;
            state.profile_import_error = None;
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                import_profile_url_and_snapshot(url, name, was_vpn_running, ui_strings),
                Action::ProfileImportFinished,
            )
        }
        Action::ProfileImportFinished(result) => {
            state.profile_import_loading = false;
            match result {
                Ok(result) => {
                    state.snapshot = result.snapshot;
                    state.profile_import_error = None;
                    state.profile_import_succeeded = true;
                    show_toast(
                        state,
                        localized_profile_import_message(
                            &result.profile_name,
                            result.restart_requested,
                            result.restart_error.as_deref(),
                            strings(state.locale),
                        ),
                    )
                }
                Err(error) => {
                    state.profile_import_error = Some(error);
                    state.profile_import_succeeded = false;
                    Command::none()
                }
            }
        }
        Action::ImportRules => {
            if state.rule_import_loading {
                return Command::none();
            }
            state.rule_import_loading = true;
            let active_profile = state.snapshot.active_profile.clone();
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                import_rules_and_snapshot(active_profile, was_vpn_running, ui_strings),
                Action::RulesImported,
            )
        }
        Action::RulesImported(result) => {
            state.rule_import_loading = false;
            match result {
                Ok(result) => {
                    state.snapshot = result.snapshot;
                    show_toast(
                        state,
                        rule_import_message(
                            result.imported_count,
                            result.reload_error.as_deref(),
                            result.restart_requested,
                            result.restart_error.as_deref(),
                            strings(state.locale),
                        ),
                    )
                }
                Err(error) if picker_was_cancelled(&error) => Command::none(),
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{}",
                        strings(state.locale).feedback_rule_import_failed_prefix,
                        error
                    ),
                ),
            }
        }
        Action::ActivateProfile(profile_id) => {
            let was_vpn_running = state.snapshot.vpn_running;
            let profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| profile_id.clone());
            let ui_strings = strings(state.locale);
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .activate_profile(&profile_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    let restart_error =
                        request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
                    Ok(ProfileActivationResult {
                        snapshot: load_snapshot().await,
                        profile_name,
                        restart_requested: was_vpn_running,
                        restart_error,
                    })
                },
                Action::ProfileActivated,
            )
        }
        Action::ProfileActivated(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    profile_activation_message(
                        &result.profile_name,
                        result.restart_requested,
                        result.restart_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_profile_activate_failed_prefix,
                    error
                ),
            ),
        },
        Action::DeleteProfile(profile_id) => {
            let was_active = state.snapshot.active_profile.as_deref() == Some(profile_id.as_str());
            let was_vpn_running = state.snapshot.vpn_running;
            let profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| profile_id.clone());
            let ui_strings = strings(state.locale);
            Command::perform(
                delete_profile_and_snapshot(
                    profile_id,
                    profile_name,
                    was_active,
                    was_vpn_running,
                    ui_strings,
                ),
                Action::ProfileDeleted,
            )
        }
        Action::ProfileDeleted(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    profile_delete_message(
                        &result.profile_name,
                        result.vpn_action.map(|action| {
                            profile_delete_vpn_action_label(action, strings(state.locale))
                        }),
                        result.vpn_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_profile_delete_failed_prefix,
                    error
                ),
            ),
        },
        Action::RefreshProfile(profile_id) => {
            let profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| profile_id.clone());
            Command::perform(
                async move {
                    let error = hmeta_core::shared_core()
                        .refresh_profile(&profile_id)
                        .await
                        .err()
                        .map(|error| error.to_string());
                    Ok::<ProfileRefreshResult, String>(ProfileRefreshResult {
                        snapshot: load_snapshot().await,
                        profile_name,
                        error,
                    })
                },
                Action::ProfileRefreshed,
            )
        }
        Action::ProfileRefreshed(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                if let Some(error) = result.error {
                    show_toast(
                        state,
                        format!(
                            "{}{}{}{}",
                            strings(state.locale).feedback_subscription_prefix,
                            result.profile_name,
                            strings(state.locale).feedback_subscription_refresh_failed_suffix,
                            error
                        ),
                    )
                } else {
                    show_toast(
                        state,
                        format!(
                            "{}{}{}",
                            strings(state.locale).feedback_subscription_prefix,
                            result.profile_name,
                            strings(state.locale).feedback_subscription_refreshed_suffix
                        ),
                    )
                }
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_subscription_refresh_failed_prefix,
                    error
                ),
            ),
        },
        Action::RefreshAllProfiles => {
            let attempted_profile_ids = state
                .snapshot
                .profiles
                .iter()
                .filter(|profile| profile.subscription_url.is_some())
                .map(|profile| profile.id.clone())
                .collect::<Vec<_>>();
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .refresh_all_profiles()
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(ProfileBatchRefreshResult {
                        snapshot: load_snapshot().await,
                        attempted_profile_ids,
                    })
                },
                Action::ProfilesRefreshed,
            )
        }
        Action::ProfilesRefreshed(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                let failed =
                    count_failed_refreshed_profiles(&state.snapshot, &result.attempted_profile_ids);
                show_toast(
                    state,
                    profile_batch_refresh_message(
                        strings(state.locale).feedback_profile_refresh_all_label,
                        result.attempted_profile_ids.len(),
                        failed,
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_subscription_refresh_failed_prefix,
                    error
                ),
            ),
        },
        Action::RestoreProfileBackup(profile_id) => {
            let profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| profile_id.clone());
            Command::perform(
                async move {
                    let error = hmeta_core::shared_core()
                        .restore_profile_backup(&profile_id)
                        .await
                        .err()
                        .map(|error| error.to_string());
                    Ok::<ProfileBackupRestoreResult, String>(ProfileBackupRestoreResult {
                        snapshot: load_snapshot().await,
                        profile_name,
                        error,
                    })
                },
                Action::ProfileBackupRestored,
            )
        }
        Action::ProfileBackupRestored(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    localized_profile_backup_restore_message(
                        &result.profile_name,
                        result.error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).profiles_backup_restore_failed_prefix,
                    error
                ),
            ),
        },
        Action::UpdateProfileSubscription {
            profile_id,
            name,
            subscription_url,
        } => Command::perform(
            async move {
                hmeta_core::shared_core()
                    .update_profile_subscription(&profile_id, &name, &subscription_url)
                    .map_err(|error| error.to_string())?;
                Ok((load_snapshot().await, name.trim().to_owned()))
            },
            Action::ProfileSubscriptionUpdated,
        ),
        Action::ProfileSubscriptionUpdated(result) => match result {
            Ok((snapshot, profile_name)) => {
                state.snapshot = snapshot;
                show_toast(
                    state,
                    format!(
                        "{}{}",
                        profile_name,
                        if state.locale == UiLocale::ZhCn {
                            " 的订阅信息已更新"
                        } else {
                            " subscription updated"
                        }
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    if state.locale == UiLocale::ZhCn {
                        "更新订阅失败："
                    } else {
                        "Update subscription failed: "
                    },
                    error
                ),
            ),
        },
        Action::ExportProfile(profile_id) => {
            let profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| "profile".to_owned());
            match hmeta_core::shared_core().profile_raw_yaml(&profile_id) {
                Ok(raw_yaml) => Command::perform(
                    async move {
                        crate::platform_callbacks::export_profile(profile_name.clone(), raw_yaml)
                            .await?;
                        Ok(profile_name)
                    },
                    Action::ProfileExported,
                ),
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{}",
                        strings(state.locale).profiles_yaml_read_failed_prefix,
                        error
                    ),
                ),
            }
        }
        Action::ProfileExported(result) => match result {
            Ok(profile_name) => show_toast(
                state,
                format!(
                    "{}{}",
                    profile_name,
                    if state.locale == UiLocale::ZhCn {
                        " 已导出"
                    } else {
                        " exported"
                    }
                ),
            ),
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    if state.locale == UiLocale::ZhCn {
                        "导出配置失败："
                    } else {
                        "Export profile failed: "
                    },
                    error
                ),
            ),
        },
        Action::OpenYamlEditor(profile_id) => {
            state.yaml_editor_profile_id = Some(profile_id.clone());
            state.yaml_editor_profile_name = state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| "Profile".to_owned());
            match hmeta_core::shared_core().profile_raw_yaml(&profile_id) {
                Ok(raw_yaml) => {
                    state.yaml_editor_text = raw_yaml.clone();
                    state.yaml_editor_original = raw_yaml;
                    state.yaml_editor_error = None;
                    state.yaml_editor_open = true;
                    state.yaml_editor_saving = false;
                    state.yaml_editor_testing = false;
                    Command::none()
                }
                Err(error) => show_toast(
                    state,
                    format!(
                        "{}{}",
                        strings(state.locale).profiles_yaml_read_failed_prefix,
                        error
                    ),
                ),
            }
        }
        Action::SetYamlEditorOpen(open) => {
            state.yaml_editor_open = open;
            state.yaml_editor_error = None;
            if !open {
                state.yaml_editor_saving = false;
                state.yaml_editor_testing = false;
            }
            Command::none()
        }
        Action::SetYamlEditorText(value) => {
            state.yaml_editor_text = value;
            state.yaml_editor_error = None;
            Command::none()
        }
        Action::ResetYamlEditorText => {
            state.yaml_editor_text = state.yaml_editor_original.clone();
            state.yaml_editor_error = None;
            Command::none()
        }
        Action::TestYamlEditor => {
            if state.yaml_editor_saving || state.yaml_editor_testing {
                return Command::none();
            }
            if state.yaml_editor_text.trim().is_empty() {
                return show_toast(state, strings(state.locale).profiles_yaml_empty.to_owned());
            }
            let raw_yaml = state.yaml_editor_text.clone();
            state.yaml_editor_testing = true;
            state.yaml_editor_error = None;
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .validate_profile_content(&raw_yaml)
                        .await
                        .map_err(|error| error.to_string())
                },
                Action::YamlEditorTested,
            )
        }
        Action::YamlEditorTested(result) => {
            state.yaml_editor_testing = false;
            match result {
                Ok(()) => show_toast(state, strings(state.locale).profiles_yaml_valid.to_owned()),
                Err(error) => {
                    state.yaml_editor_error = Some(error);
                    Command::none()
                }
            }
        }
        Action::SaveYamlEditor => {
            let Some(profile_id) = state.yaml_editor_profile_id.clone() else {
                return show_toast(
                    state,
                    strings(state.locale)
                        .profiles_yaml_profile_required
                        .to_owned(),
                );
            };
            if state.yaml_editor_saving || state.yaml_editor_testing {
                return Command::none();
            }
            let raw_yaml = state.yaml_editor_text.clone();
            state.yaml_editor_saving = true;
            state.yaml_editor_error = None;
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .update_profile_content(&profile_id, &raw_yaml)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(load_snapshot().await)
                },
                Action::YamlEditorSaved,
            )
        }
        Action::YamlEditorSaved(result) => {
            state.yaml_editor_saving = false;
            match result {
                Ok(snapshot) => {
                    state.snapshot = snapshot;
                    state.yaml_editor_open = false;
                    state.yaml_editor_error = None;
                    state.yaml_editor_original = state.yaml_editor_text.clone();
                    show_toast(
                        state,
                        strings(state.locale)
                            .profiles_yaml_saved_reloaded
                            .to_owned(),
                    )
                }
                Err(error) => {
                    state.yaml_editor_error = Some(error);
                    Command::none()
                }
            }
        }
        Action::RefreshProvider {
            provider_type,
            provider_name,
        } => Command::perform(
            async move {
                let error = hmeta_core::shared_core()
                    .refresh_provider_of_type(&provider_type, &provider_name)
                    .await
                    .err()
                    .map(|error| error.to_string());
                Ok::<ProviderRefreshResult, String>(ProviderRefreshResult {
                    snapshot: load_snapshot().await,
                    provider_name,
                    error,
                })
            },
            Action::ProviderRefreshed,
        ),
        Action::ProviderRefreshed(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                if let Some(error) = result.error {
                    show_toast(
                        state,
                        format!(
                            "{}{}{}{}",
                            strings(state.locale).feedback_resource_prefix,
                            result.provider_name,
                            strings(state.locale).feedback_resource_refresh_failed_suffix,
                            error
                        ),
                    )
                } else {
                    show_toast(
                        state,
                        format!(
                            "{}{}{}",
                            strings(state.locale).feedback_resource_prefix,
                            result.provider_name,
                            strings(state.locale).feedback_resource_refreshed_suffix
                        ),
                    )
                }
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_resource_refresh_failed_prefix,
                    error
                ),
            ),
        },
        Action::RefreshAllProviders => {
            let attempted_providers = state
                .snapshot
                .providers
                .iter()
                .map(|provider| (provider.provider_type.clone(), provider.name.clone()))
                .collect::<Vec<_>>();
            Command::perform(
                async move {
                    let error = hmeta_core::shared_core()
                        .refresh_all_providers()
                        .await
                        .err()
                        .map(|error| error.to_string());
                    Ok::<ProviderBatchRefreshResult, String>(ProviderBatchRefreshResult {
                        snapshot: load_snapshot().await,
                        attempted_providers,
                        error,
                    })
                },
                Action::ProvidersRefreshed,
            )
        }
        Action::ProvidersRefreshed(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                let failed =
                    count_failed_refreshed_providers(&state.snapshot, &result.attempted_providers);
                show_toast(
                    state,
                    provider_batch_refresh_message(
                        result.attempted_providers.len(),
                        failed,
                        result.error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_resource_refresh_failed_prefix,
                    error
                ),
            ),
        },
        Action::SetRuleEnabled {
            profile_id,
            rule_id,
            enabled,
        } => {
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                set_rule_enabled_and_snapshot(
                    profile_id,
                    rule_id,
                    enabled,
                    was_vpn_running,
                    ui_strings,
                ),
                Action::RulesChanged,
            )
        }
        Action::ReorderRules {
            profile_id,
            ordered_rule_ids,
        } => {
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                reorder_rules_and_snapshot(
                    profile_id,
                    ordered_rule_ids,
                    was_vpn_running,
                    ui_strings,
                ),
                Action::RulesChanged,
            )
        }
        Action::DeleteRule {
            profile_id,
            rule_id,
        } => {
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                delete_rule_and_snapshot(profile_id, rule_id, was_vpn_running, ui_strings),
                Action::RulesChanged,
            )
        }
        Action::RulesChanged(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    settings_saved_message(
                        strings(state.locale).feedback_label_rules,
                        result.restart_requested,
                        result.restart_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_rule_update_failed_prefix,
                    error
                ),
            ),
        },
        Action::SaveDnsSettings {
            servers_text,
            fallbacks_text,
            policy_text,
        } => {
            let Some(profile_id) = state.snapshot.active_profile.clone() else {
                return show_toast(
                    state,
                    strings(state.locale)
                        .feedback_active_profile_required
                        .to_owned(),
                );
            };
            let dns_servers = parse_dns_servers_text(&servers_text);
            if dns_servers.is_empty() {
                return show_toast(
                    state,
                    strings(state.locale)
                        .feedback_dns_upstream_required
                        .to_owned(),
                );
            }
            let dns_fallbacks = parse_dns_servers_text(&fallbacks_text);
            let ui_strings = strings(state.locale);
            let dns_policy = match parse_dns_policy_text(&policy_text, ui_strings) {
                Ok(policy) => policy,
                Err(error) => return show_toast(state, error),
            };
            let was_vpn_running = state.snapshot.vpn_running;
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .set_profile_dns_config(&profile_id, dns_servers, dns_fallbacks, dns_policy)
                        .await
                        .map_err(|error| error.to_string())?;
                    let restart_error =
                        request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
                    Ok(SettingsSaveResult {
                        snapshot: load_snapshot().await,
                        restart_requested: was_vpn_running,
                        restart_error,
                    })
                },
                Action::DnsSettingsSaved,
            )
        }
        Action::DnsSettingsSaved(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    settings_saved_message(
                        strings(state.locale).feedback_label_dns_settings,
                        result.restart_requested,
                        result.restart_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_dns_save_failed_prefix,
                    error
                ),
            ),
        },
        Action::SaveVpnSettings {
            system_proxy,
            dns_hijacking,
            allow_bypass,
            stack,
        } => {
            let Some(profile_id) = state.snapshot.active_profile.clone() else {
                return show_toast(
                    state,
                    strings(state.locale)
                        .feedback_active_profile_required
                        .to_owned(),
                );
            };
            let stack = stack.trim().to_owned();
            if stack.is_empty() {
                return show_toast(
                    state,
                    strings(state.locale).feedback_vpn_stack_required.to_owned(),
                );
            }
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .set_profile_vpn_config(
                            &profile_id,
                            system_proxy,
                            dns_hijacking,
                            allow_bypass,
                            stack,
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                    let restart_error =
                        request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
                    Ok(SettingsSaveResult {
                        snapshot: load_snapshot().await,
                        restart_requested: was_vpn_running,
                        restart_error,
                    })
                },
                Action::VpnSettingsSaved,
            )
        }
        Action::VpnSettingsSaved(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    settings_saved_message(
                        strings(state.locale).feedback_label_vpn_settings,
                        result.restart_requested,
                        result.restart_error.as_deref(),
                        strings(state.locale),
                    ),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_vpn_save_failed_prefix,
                    error
                ),
            ),
        },
    }
}

fn open_manual_rule_editor(
    state: &mut State,
    connection_id: Option<String>,
    domain: String,
    destination_ip: String,
) {
    let domain = domain.trim().to_owned();
    let destination_ip = destination_ip.trim().to_owned();
    let (match_kind, value) = if domain.is_empty() {
        if destination_ip.is_empty() {
            (ManualRuleMatchKind::Domain, String::new())
        } else {
            (ManualRuleMatchKind::IpCidr, destination_ip.clone())
        }
    } else {
        (ManualRuleMatchKind::Domain, domain.clone())
    };
    state.manual_rule_editor = Some(ManualRuleEditorState {
        connection_id,
        domain,
        destination_ip,
        match_kind,
        value,
        target: "DIRECT".to_owned(),
        disconnect_after_save: false,
        submitting: false,
        error: None,
    });
}

fn system_language() -> String {
    std::env::var("HMETA_UI_LOCALE").unwrap_or_else(|_| "zh-CN".to_owned())
}

fn system_color_mode() -> i32 {
    std::env::var("HMETA_SYSTEM_COLOR_MODE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

async fn load_snapshot() -> RuntimeSnapshot {
    let core = hmeta_core::shared_core();
    let _ = core.sync_external_controller_config().await;
    core.snapshot().unwrap_or_default()
}

async fn delayed_snapshot() -> RuntimeSnapshot {
    let _ = hmeta_core::shared_core()
        .wait_for_platform_change(Duration::from_millis(1000))
        .await;
    load_snapshot().await
}

async fn delayed_vpn_snapshot() -> RuntimeSnapshot {
    tokio::time::sleep(Duration::from_millis(200)).await;
    load_snapshot().await
}

async fn bootstrap_active_profile() -> RuntimeSnapshot {
    let core = hmeta_core::shared_core();
    // State::new restores a revision-checked proxy-group cache synchronously,
    // so the dashboard can render immediately. Parse the complete meow config
    // only after the first frame, then replace the cache-backed snapshot.
    let _ = core.prepare_active_vpn().await;
    let refresh_core = core.clone();
    tokio::spawn(async move {
        let _ = refresh_core.refresh_due_profiles().await;
    });
    load_snapshot().await
}

async fn lookup_rule(query: String) -> Result<hmeta_core::RuleLookupResult, String> {
    hmeta_core::shared_core()
        .lookup_rule(&query)
        .await
        .map_err(|error| error.to_string())
}

fn reconcile_vpn_command(state: &mut State) {
    state.vpn_command_pending = state.vpn_command_pending.filter(|action| {
        vpn_command_is_pending(
            *action,
            state.snapshot.vpn_lifecycle,
            state.snapshot.vpn_running,
        )
    });
}

fn count_failed_refreshed_profiles(
    snapshot: &RuntimeSnapshot,
    attempted_profile_ids: &[String],
) -> usize {
    snapshot
        .profiles
        .iter()
        .filter(|profile| {
            attempted_profile_ids.iter().any(|id| id == &profile.id)
                && profile.last_refresh_error.is_some()
        })
        .count()
}

fn count_failed_refreshed_providers(
    snapshot: &RuntimeSnapshot,
    attempted_providers: &[(String, String)],
) -> usize {
    snapshot
        .providers
        .iter()
        .filter(|provider| {
            attempted_providers.iter().any(|(provider_type, name)| {
                provider_type == &provider.provider_type && name == &provider.name
            }) && provider.last_refresh_error.is_some()
        })
        .count()
}

async fn start_vpn_command_and_snapshot(
    profile_id: String,
    profile_name: String,
    ui_strings: &'static UiStrings,
) -> Result<VpnCommandResult, String> {
    let core = hmeta_core::shared_core();
    let active_profile = core
        .snapshot()
        .map_err(|error| error.to_string())?
        .active_profile;
    if active_profile.as_deref() != Some(profile_id.as_str()) {
        core.activate_profile(&profile_id).await.map_err(|error| {
            format!(
                "{}{}{}{}",
                ui_strings.feedback_vpn_start_profile_load_failed_prefix,
                profile_name,
                ui_strings.feedback_vpn_start_profile_load_failed_mid,
                error
            )
        })?;
    }
    let options_json = core.active_vpn_options_json().map_err(|error| {
        format!(
            "{}{}",
            ui_strings.feedback_vpn_start_options_failed_prefix, error
        )
    })?;
    let request_error = crate::platform_callbacks::request_start_vpn(options_json)
        .await
        .err()
        .map(|error| error.to_string());
    Ok(VpnCommandResult {
        snapshot: load_snapshot().await,
        action: VpnCommandAction::Start,
        profile_name: Some(profile_name),
        request_error,
    })
}

async fn stop_vpn_command_and_snapshot(
    ui_strings: &'static UiStrings,
) -> Result<VpnCommandResult, String> {
    let request_error = match crate::platform_callbacks::request_stop_vpn().await {
        Ok(()) => None,
        Err(error) => {
            hmeta_core::shared_core().stop_vpn().map_err(|fallback| {
                format!(
                    "{}{}{}{}",
                    ui_strings.feedback_vpn_stop_callback_failed_prefix,
                    error,
                    ui_strings.feedback_vpn_stop_fallback_failed_mid,
                    fallback
                )
            })?;
            Some(error.to_string())
        }
    };
    Ok(VpnCommandResult {
        snapshot: load_snapshot().await,
        action: VpnCommandAction::Stop,
        profile_name: None,
        request_error,
    })
}

async fn request_vpn_restart_if_running(
    was_vpn_running: bool,
    ui_strings: &UiStrings,
) -> Option<String> {
    if !was_vpn_running {
        return None;
    }

    let mut errors = Vec::new();
    if let Err(error) = crate::platform_callbacks::request_stop_vpn().await {
        match hmeta_core::shared_core().stop_vpn() {
            Ok(()) => {
                errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_stop_fallback_applied_prefix, error
                ));
                return Some(errors.join("；"));
            }
            Err(fallback) => {
                errors.push(format!(
                    "{}{}{}{}",
                    ui_strings.feedback_vpn_stop_failed_prefix,
                    error,
                    ui_strings.feedback_vpn_stop_fallback_failed_suffix,
                    fallback
                ));
                return Some(errors.join("；"));
            }
        }
    }

    match hmeta_core::shared_core().active_vpn_options_json() {
        Ok(options_json) => {
            if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json).await {
                errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_start_callback_failed_prefix, error
                ));
            }
        }
        Err(error) => errors.push(format!(
            "{}{}",
            ui_strings.feedback_vpn_options_failed_prefix, error
        )),
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors.join("；"))
    }
}

fn dns_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (String, String, String) {
    (
        snapshot.vpn_options.dns_servers.join(", "),
        snapshot.vpn_options.dns_fallbacks.join(", "),
        dns_policy_text(&snapshot.vpn_options.dns_nameserver_policy),
    )
}

fn vpn_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (bool, bool, bool, String) {
    let stack = hmeta_model::VpnStack::try_from(snapshot.vpn_options.stack.as_str())
        .unwrap_or_default()
        .as_str()
        .to_owned();
    (
        snapshot.vpn_options.system_proxy,
        snapshot.vpn_options.dns_hijacking,
        snapshot.vpn_options.allow_bypass,
        stack,
    )
}

fn dns_policy_text(policy: &BTreeMap<String, Vec<String>>) -> String {
    policy
        .iter()
        .map(|(matcher, servers)| format!("{matcher} = {}", servers.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_dns_servers_text(value: &str) -> Vec<String> {
    let mut servers = Vec::new();
    for item in value.split(|character: char| {
        character == ',' || character == ';' || character.is_ascii_whitespace()
    }) {
        let item = item.trim();
        if item.is_empty() || servers.iter().any(|server| server == item) {
            continue;
        }
        servers.push(item.to_owned());
    }
    servers
}

fn parse_dns_policy_text(
    value: &str,
    ui_strings: &UiStrings,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut policy = BTreeMap::new();
    for line in value.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((matcher, servers)) = line.split_once('=') else {
            return Err(ui_strings.feedback_dns_policy_format_error.to_owned());
        };
        let matcher = matcher.trim();
        if matcher.is_empty() {
            return Err(ui_strings.feedback_dns_policy_matcher_required.to_owned());
        }
        let servers = parse_dns_servers_text(servers);
        if servers.is_empty() {
            return Err(format!(
                "{}{}{}",
                ui_strings.feedback_dns_policy_upstream_missing_prefix,
                matcher,
                ui_strings.feedback_dns_policy_upstream_missing_suffix
            ));
        }
        policy.insert(matcher.to_owned(), servers);
    }
    Ok(policy)
}

async fn import_profile_url_and_snapshot(
    url: String,
    name: Option<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let id = hmeta_core::shared_core()
        .import_profile_from_url(&url, name)
        .await
        .map_err(|error| error.to_string())?;
    hmeta_core::shared_core()
        .activate_profile(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(profile_import_result(id, was_vpn_running, ui_strings).await)
}

async fn scan_profile_subscription_and_snapshot(
    name: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let payload = crate::platform_callbacks::scan_subscription_code()
        .await
        .map_err(|error| format!("{}{}", ui_strings.profiles_scan_failed_prefix, error))?;
    let scanned = match parse_scanned_subscription(&payload) {
        Ok(scanned) => scanned,
        Err(ScannedSubscriptionError::Empty) => {
            return Err("profile scan cancelled".to_owned());
        }
        Err(ScannedSubscriptionError::Unsupported) => {
            return Err(ui_strings.profiles_scan_invalid.to_owned());
        }
    };
    let name = match name.trim() {
        "" => scanned.name,
        value => Some(value.to_owned()),
    };
    let core = hmeta_core::shared_core();
    let existing = core.snapshot().ok().and_then(|snapshot| {
        snapshot
            .profiles
            .into_iter()
            .find(|profile| profile.subscription_url.as_deref() == Some(scanned.url.as_str()))
    });
    if let Some(profile) = existing {
        if let Some(name) = name {
            core.update_profile_subscription(&profile.id, &name, &scanned.url)
                .map_err(|error| error.to_string())?;
        }
        core.refresh_profile(&profile.id)
            .await
            .map_err(|error| error.to_string())?;
        core.activate_profile(&profile.id)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(profile_import_result(profile.id, was_vpn_running, ui_strings).await);
    }
    import_profile_url_and_snapshot(scanned.url, name, was_vpn_running, ui_strings).await
}

async fn import_profile_file_and_snapshot(
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let (name, raw_yaml) = crate::platform_callbacks::pick_profile_text().await?;
    let id = hmeta_core::shared_core()
        .import_profile_from_content(&name, "local-file", &raw_yaml, None)
        .await
        .map_err(|error| error.to_string())?;
    hmeta_core::shared_core()
        .activate_profile(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(profile_import_result(id, was_vpn_running, ui_strings).await)
}

async fn profile_import_result(
    profile_id: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> ProfileImportResult {
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
    let snapshot = load_snapshot().await;
    let profile_name = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.name.clone())
        .unwrap_or(profile_id);
    ProfileImportResult {
        snapshot,
        profile_name,
        restart_requested: was_vpn_running,
        restart_error,
    }
}

async fn import_rules_and_snapshot(
    active_profile: Option<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleImportResult, String> {
    let profile_id =
        active_profile.ok_or_else(|| ui_strings.feedback_active_profile_required.to_owned())?;
    let (name, rules_text) = crate::platform_callbacks::pick_profile_text().await?;
    let source = format!("rules:{name}");
    let imported_rule_ids = hmeta_core::shared_core()
        .import_rules_from_content(Some(&profile_id), &source, &rules_text)
        .map_err(|error| error.to_string())?;
    let reload_error = hmeta_core::shared_core()
        .activate_profile(&profile_id)
        .await
        .err()
        .map(|error| error.to_string());
    let restart_error = if reload_error.is_none() {
        request_vpn_restart_if_running(was_vpn_running, ui_strings).await
    } else {
        None
    };
    Ok(RuleImportResult {
        snapshot: load_snapshot().await,
        imported_count: imported_rule_ids.len(),
        reload_error,
        restart_requested: was_vpn_running,
        restart_error,
    })
}

fn picker_was_cancelled(error: &str) -> bool {
    error.to_ascii_lowercase().contains("cancel")
        || error.contains("取消")
        || error.contains("已取消")
}

async fn delete_profile_and_snapshot(
    profile_id: String,
    profile_name: String,
    was_active: bool,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileDeleteResult, String> {
    let mut vpn_errors = Vec::new();
    if was_active && was_vpn_running {
        if let Err(error) = crate::platform_callbacks::request_stop_vpn().await {
            match hmeta_core::shared_core().stop_vpn() {
                Ok(()) => vpn_errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_stop_fallback_applied_prefix, error
                )),
                Err(fallback) => {
                    return Err(format!(
                        "{}{}{}{}",
                        ui_strings.feedback_vpn_stop_failed_prefix,
                        error,
                        ui_strings.feedback_vpn_stop_fallback_failed_suffix,
                        fallback
                    ));
                }
            }
        }
    }
    hmeta_core::shared_core()
        .delete_profile(&profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = load_snapshot().await;
    let mut vpn_action = None;
    if was_active && was_vpn_running && snapshot.active_profile.is_some() {
        vpn_action = Some(ProfileDeleteVpnAction::Restart);
        let options_json = hmeta_core::shared_core()
            .active_vpn_options_json()
            .map_err(|error| error.to_string())?;
        if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json).await {
            vpn_errors.push(format!(
                "{}{}",
                ui_strings.feedback_vpn_start_callback_failed_prefix, error
            ));
        }
    } else if was_active && was_vpn_running {
        vpn_action = Some(ProfileDeleteVpnAction::Stop);
    }
    Ok(ProfileDeleteResult {
        snapshot: load_snapshot().await,
        profile_name,
        vpn_action,
        vpn_error: (!vpn_errors.is_empty()).then(|| vpn_errors.join("；")),
    })
}

async fn select_proxy_and_snapshot(
    selections: Vec<(String, String)>,
) -> Result<RuntimeSnapshot, String> {
    for (group, proxy) in selections {
        hmeta_core::shared_core()
            .select_proxy_via_controller(&group, &proxy)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(load_snapshot().await)
}

async fn unfix_proxy_and_snapshot(group: String) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .unfix_proxy_via_controller(&group)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

async fn test_proxy_delays_and_snapshot(
    groups: Vec<(String, usize)>,
) -> Result<ProxyDelayBatchResult, String> {
    let mut succeeded = 0;
    let mut failed = 0;
    for (group, member_count) in groups {
        match hmeta_core::shared_core()
            .test_proxy_group_via_controller(&group, None, Some(5000))
            .await
        {
            Ok(delays) => {
                succeeded += delays.values().filter(|delay| **delay > 0).count();
                failed += delays.values().filter(|delay| **delay == 0).count();
            }
            Err(_) => failed += member_count,
        }
    }
    Ok(ProxyDelayBatchResult {
        snapshot: load_snapshot().await,
        succeeded,
        failed,
    })
}

fn proxy_groups_for_delay_test(snapshot: &RuntimeSnapshot) -> Vec<(String, usize)> {
    snapshot
        .proxy_groups
        .iter()
        .filter(|group| !group.proxies.is_empty())
        .map(|group| (group.name.clone(), group.proxies.len()))
        .collect()
}

async fn close_connection_and_snapshot(connection_id: String) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .close_connection_via_controller(&connection_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

async fn apply_manual_rule_and_snapshot(
    profile_id: String,
    spec: ManualRuleSpec,
    connection_id: Option<String>,
) -> Result<ManualRuleSaveResult, String> {
    let applied = hmeta_core::shared_core()
        .apply_manual_rule(&profile_id, &spec)
        .await
        .map_err(|error| error.to_string())?;
    let connection_close_requested = connection_id.is_some();
    let connection_close_error = if let Some(connection_id) = connection_id {
        hmeta_core::shared_core()
            .close_connection_via_controller(&connection_id)
            .await
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    Ok(ManualRuleSaveResult {
        snapshot: load_snapshot().await,
        applied,
        connection_close_requested,
        connection_close_error,
    })
}

async fn close_all_connections_and_snapshot() -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .close_all_connections_via_controller()
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

async fn clear_request_history_and_snapshot() -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .clear_request_history()
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

fn refresh_log_recording_status(state: &mut State) {
    if let Ok(status) = hmeta_core::shared_core().log_recording_status() {
        state.log_recording = status;
    }
}

fn locale_text(locale: UiLocale, zh: &'static str, en: &'static str) -> &'static str {
    if locale == UiLocale::ZhCn {
        zh
    } else {
        en
    }
}

async fn set_log_recording_and_snapshot(enabled: bool) -> Result<LogRecordingChangeResult, String> {
    let status = hmeta_core::shared_core()
        .set_log_recording_enabled(enabled)
        .map_err(|error| error.to_string())?;
    Ok(LogRecordingChangeResult {
        snapshot: load_snapshot().await,
        status,
    })
}

async fn export_log_archive(file_name: String) -> Result<String, String> {
    let content = hmeta_core::shared_core()
        .read_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    crate::platform_callbacks::export_log(file_name.clone(), content).await?;
    Ok(file_name)
}

async fn delete_log_archive(file_name: String) -> Result<LogArchiveDeleteResult, String> {
    let status = hmeta_core::shared_core()
        .delete_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    Ok(LogArchiveDeleteResult { file_name, status })
}

async fn set_rule_enabled_and_snapshot(
    profile_id: String,
    rule_id: String,
    enabled: bool,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .set_rule_enabled(&profile_id, &rule_id, enabled)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

async fn delete_rule_and_snapshot(
    profile_id: String,
    rule_id: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .delete_rule(&rule_id)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

async fn reorder_rules_and_snapshot(
    profile_id: String,
    ordered_rule_ids: Vec<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .reorder_rules(&profile_id, &ordered_rule_ids)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

async fn reload_profile_after_rule_change(
    profile_id: &str,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .activate_profile(profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
    Ok(RuleChangeResult {
        snapshot: load_snapshot().await,
        restart_requested: was_vpn_running,
        restart_error,
    })
}

fn localized_profile_import_message(
    profile_name: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    let base = format!(
        "{}{}{}",
        strings.profiles_import_toast_prefix,
        profile_name,
        strings.profiles_import_toast_success_suffix
    );
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}",
            strings.profiles_import_toast_restart_failed_suffix, error
        )
    } else if restart_requested {
        format!("{base}{}", strings.profiles_import_toast_restart_suffix)
    } else {
        base
    }
}

fn localized_profile_backup_restore_message(
    profile_name: &str,
    error: Option<&str>,
    strings: &UiStrings,
) -> String {
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}{}",
            strings.feedback_profile_prefix,
            profile_name,
            strings.profiles_backup_restore_failed_suffix,
            error
        )
    } else {
        format!(
            "{}{}{}",
            strings.feedback_profile_prefix,
            profile_name,
            strings.profiles_backup_restore_success_suffix
        )
    }
}

fn profile_delete_vpn_action_label(
    action: ProfileDeleteVpnAction,
    strings: &UiStrings,
) -> &'static str {
    match action {
        ProfileDeleteVpnAction::Stop => strings.feedback_vpn_action_stop,
        ProfileDeleteVpnAction::Restart => strings.feedback_vpn_action_restart,
    }
}

fn manual_rule_saved_message(result: &ManualRuleSaveResult, locale: UiLocale) -> String {
    let mut parts = vec![
        match (locale, result.applied.mutation.kind) {
            (UiLocale::ZhCn, ManualRuleMutationKind::Added) => "手动规则已添加".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Updated) => "冲突规则已更新".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Reenabled) => "手动规则已重新启用".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Unchanged) => "相同规则已存在".to_owned(),
            (_, ManualRuleMutationKind::Added) => "Manual rule added".to_owned(),
            (_, ManualRuleMutationKind::Updated) => "Conflicting rule updated".to_owned(),
            (_, ManualRuleMutationKind::Reenabled) => "Manual rule re-enabled".to_owned(),
            (_, ManualRuleMutationKind::Unchanged) => "The same rule already exists".to_owned(),
        },
        result.applied.mutation.line.clone(),
    ];
    if result.applied.live_updated {
        parts.push(if locale == UiLocale::ZhCn {
            "运行时规则已热更新".to_owned()
        } else {
            "Runtime rules updated live".to_owned()
        });
    } else {
        parts.push(if locale == UiLocale::ZhCn {
            "将在 VPN 下次启动时生效".to_owned()
        } else {
            "Takes effect on the next VPN start".to_owned()
        });
    }
    if !result.applied.rule_mode_active {
        parts.push(if locale == UiLocale::ZhCn {
            "当前不是规则模式，切换到规则模式后才会命中".to_owned()
        } else {
            "Rule mode is not active; switch to Rule mode for matching".to_owned()
        });
    }
    if let Some(error) = &result.connection_close_error {
        parts.push(if locale == UiLocale::ZhCn {
            format!("规则已保存，但断开当前连接失败：{error}")
        } else {
            format!("Rule saved, but the current connection could not be closed: {error}")
        });
    } else if result.connection_close_requested {
        parts.push(if locale == UiLocale::ZhCn {
            "当前连接已断开，新连接将使用新规则".to_owned()
        } else {
            "Current connection closed; the new rule applies when it reconnects".to_owned()
        });
    }
    parts.join(" · ")
}

fn show_toast(state: &mut State, message: String) -> Command<Action> {
    state.notifications.publish(message);
    Command::none()
}

#[path = "view.rs"]
mod view;

pub(crate) use view::App;
