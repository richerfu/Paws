use crate::activity_filter::{
    matches_connection_query, matches_request_filter, request_connection_query, RequestStatusFilter,
};
use crate::installed_app_filter::matches_installed_application_query;
use crate::l10n::{strings, UiLocale, UiStrings};
use crate::log_filter::{matches_log_filter, LogLevelFilter};
use crate::mode_feedback::mode_changed_message;
use crate::notification::NotificationCenter;
use crate::profile_filter::matches_profile_query;
use crate::profile_refresh_feedback::{
    profile_activation_message, profile_batch_refresh_message, profile_delete_message,
};
use crate::provider_refresh_feedback::provider_batch_refresh_message;
use crate::proxy_grid::{flatten_proxy_groups, proxy_selection_chain, ProxyGridItem};
use crate::resource_filter::{matches_geodata_query, matches_provider_query, matches_rule_query};
use crate::rule_feedback::rule_import_message;
use crate::settings_feedback::settings_saved_message;
use crate::time_format;
use crate::traffic_history::summarize_traffic_history;
use crate::ui_preferences::{LanguagePreference, ThemePreference, UiPreferences};
use crate::vpn_feedback::{vpn_command_is_pending, vpn_command_message, VpnCommandAction};
use crate::yaml_summary::summarize_yaml_edit;
use hmeta_model::{
    InstalledApplication, PerAppMode, RuntimeMode, RuntimeSnapshot, TrafficHistoryPoint,
    VpnLifecycle,
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
    SetMode(RuntimeMode),
    ModeChanged(Result<ModeChangeResult, String>),
    SelectProxy {
        group: String,
        proxy: String,
    },
    ProxySelected(Result<RuntimeSnapshot, String>),
    TestAllProxyDelays,
    AllProxyDelaysTested(Result<ProxyDelayBatchResult, String>),
    CloseConnection(String),
    ConnectionClosed(Result<RuntimeSnapshot, String>),
    CloseAllConnections,
    AllConnectionsClosed(Result<RuntimeSnapshot, String>),
    OpenExternalUrl(String),
    ExternalUrlOpened(Result<(), String>),
    ClearRequestHistory,
    RequestHistoryCleared(Result<RuntimeSnapshot, String>),
    ClearLogs,
    LogsCleared(Result<RuntimeSnapshot, String>),
    ResetProfileImportFeedback,
    ImportLocalProfile,
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
    SavePerAppSettings {
        mode: PerAppMode,
        trusted_applications_text: String,
        blocked_applications_text: String,
    },
    PerAppSettingsSaved(Result<SettingsSaveResult, String>),
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
    RefreshInstalledApplications,
    InstalledApplicationsLoaded(Result<Vec<InstalledApplication>, String>),
}

#[derive(Clone)]
pub(crate) struct State {
    locale: UiLocale,
    preferences: UiPreferences,
    theme_dark: bool,
    snapshot: RuntimeSnapshot,
    profile_import_error: Option<String>,
    profile_import_loading: bool,
    yaml_editor_open: bool,
    yaml_editor_profile_id: Option<String>,
    yaml_editor_profile_name: String,
    yaml_editor_text: String,
    yaml_editor_original: String,
    yaml_editor_error: Option<String>,
    yaml_editor_saving: bool,
    yaml_editor_testing: bool,
    installed_applications: Vec<InstalledApplication>,
    installed_applications_loading: bool,
    installed_applications_error: Option<String>,
    vpn_command_pending: Option<VpnCommandAction>,
    proxy_selection_pending: Option<(String, String)>,
    proxy_delay_loading: bool,
    notifications: NotificationCenter,
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

impl State {
    pub(crate) fn new(notifications: NotificationCenter) -> Self {
        let snapshot = hmeta_core::shared_core().snapshot().unwrap_or_default();
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
            yaml_editor_open: false,
            yaml_editor_profile_id: None,
            yaml_editor_profile_name: String::new(),
            yaml_editor_text: String::new(),
            yaml_editor_original: String::new(),
            yaml_editor_error: None,
            yaml_editor_saving: false,
            yaml_editor_testing: false,
            installed_applications: Vec::new(),
            installed_applications_loading: false,
            installed_applications_error: None,
            vpn_command_pending: None,
            proxy_selection_pending: None,
            proxy_delay_loading: false,
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
            reconcile_vpn_command(state);
            state.refresh_system_preferences();
            Command::none()
        }
        Action::TickSnapshot(snapshot) => {
            state.snapshot = snapshot;
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
                Command::perform(
                    stop_vpn_command_and_snapshot(ui_strings),
                    Action::VpnCommandFinished,
                )
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
                Command::perform(
                    start_vpn_command_and_snapshot(profile_id, profile_name, ui_strings),
                    Action::VpnCommandFinished,
                )
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
        Action::SetMode(mode) => Command::perform(
            async move {
                hmeta_core::shared_core()
                    .set_mode(mode)
                    .map_err(|error| error.to_string())?;
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
            let proxies = proxy_names_for_delay_test(&state.snapshot);
            if proxies.is_empty() {
                show_toast(
                    state,
                    strings(state.locale).feedback_proxy_delay_empty.to_owned(),
                )
            } else {
                state.proxy_delay_loading = true;
                Command::perform(
                    test_proxy_delays_and_snapshot(proxies),
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
        Action::ClearLogs => Command::perform(clear_logs_and_snapshot(), Action::LogsCleared),
        Action::LogsCleared(result) => match result {
            Ok(snapshot) => {
                state.snapshot = snapshot;
                show_toast(
                    state,
                    strings(state.locale).feedback_logs_cleared.to_owned(),
                )
            }
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_logs_clear_failed_prefix,
                    error
                ),
            ),
        },
        Action::ResetProfileImportFeedback => {
            state.profile_import_error = None;
            Command::none()
        }
        Action::ImportLocalProfile => {
            state.profile_import_error = None;
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                import_profile_file_and_snapshot(was_vpn_running, ui_strings),
                Action::LocalProfileImportFinished,
            )
        }
        Action::LocalProfileImportFinished(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
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
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).profiles_import_failed_prefix,
                    error
                ),
            ),
        },
        Action::ImportProfileFromUrl { url, name } => {
            if state.profile_import_loading {
                return Command::none();
            }
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
                    Command::none()
                }
            }
        }
        Action::ImportRules => {
            let active_profile = state.snapshot.active_profile.clone().or_else(|| {
                state
                    .snapshot
                    .profiles
                    .first()
                    .map(|profile| profile.id.clone())
            });
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                import_rules_and_snapshot(active_profile, was_vpn_running, ui_strings),
                Action::RulesImported,
            )
        }
        Action::RulesImported(result) => match result {
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
            Err(error) => show_toast(
                state,
                format!(
                    "{}{}",
                    strings(state.locale).feedback_rule_import_failed_prefix,
                    error
                ),
            ),
        },
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
                    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
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
        Action::SavePerAppSettings {
            mode,
            trusted_applications_text,
            blocked_applications_text,
        } => {
            let Some(profile_id) = state.snapshot.active_profile.clone() else {
                return show_toast(
                    state,
                    strings(state.locale)
                        .feedback_active_profile_required
                        .to_owned(),
                );
            };
            let trusted_applications = parse_applications_text(&trusted_applications_text);
            let blocked_applications = parse_applications_text(&blocked_applications_text);
            let was_vpn_running = state.snapshot.vpn_running;
            let ui_strings = strings(state.locale);
            Command::perform(
                async move {
                    hmeta_core::shared_core()
                        .set_profile_per_app_config(
                            &profile_id,
                            mode,
                            trusted_applications,
                            blocked_applications,
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
                    Ok(SettingsSaveResult {
                        snapshot: load_snapshot().await,
                        restart_requested: was_vpn_running,
                        restart_error,
                    })
                },
                Action::PerAppSettingsSaved,
            )
        }
        Action::PerAppSettingsSaved(result) => match result {
            Ok(result) => {
                state.snapshot = result.snapshot;
                show_toast(
                    state,
                    settings_saved_message(
                        strings(state.locale).feedback_label_per_app_settings,
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
                    strings(state.locale).feedback_per_app_save_failed_prefix,
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
                    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
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
                    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
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
        Action::RefreshInstalledApplications => {
            state.installed_applications_loading = true;
            state.installed_applications_error = None;
            Command::perform(
                load_installed_applications(),
                Action::InstalledApplicationsLoaded,
            )
        }
        Action::InstalledApplicationsLoaded(result) => {
            state.installed_applications_loading = false;
            match result {
                Ok(applications) => {
                    state.installed_applications = applications;
                    state.installed_applications_error = None;
                    Command::none()
                }
                Err(error) => {
                    state.installed_applications_error = Some(error.clone());
                    show_toast(
                        state,
                        format!(
                            "{}{}",
                            strings(state.locale).feedback_installed_apps_load_failed_prefix,
                            error
                        ),
                    )
                }
            }
        }
    }
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
    hmeta_core::shared_core().snapshot().unwrap_or_default()
}

async fn delayed_snapshot() -> RuntimeSnapshot {
    tokio::time::sleep(Duration::from_millis(1000)).await;
    load_snapshot().await
}

async fn bootstrap_active_profile() -> RuntimeSnapshot {
    let core = hmeta_core::shared_core();
    let _ = core.refresh_due_profiles().await;
    if let Some(profile_id) = core
        .snapshot()
        .ok()
        .and_then(|snapshot| snapshot.active_profile)
    {
        let _ = core.reload_config(&profile_id).await;
    }
    load_snapshot().await
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

async fn load_installed_applications() -> Result<Vec<InstalledApplication>, String> {
    crate::platform_callbacks::list_installed_applications().await
}

async fn start_vpn_command_and_snapshot(
    profile_id: String,
    profile_name: String,
    ui_strings: &'static UiStrings,
) -> Result<VpnCommandResult, String> {
    hmeta_core::shared_core()
        .activate_profile(&profile_id)
        .await
        .map_err(|error| {
            format!(
                "{}{}{}{}",
                ui_strings.feedback_vpn_start_profile_load_failed_prefix,
                profile_name,
                ui_strings.feedback_vpn_start_profile_load_failed_mid,
                error
            )
        })?;
    let options_json = hmeta_core::shared_core()
        .active_vpn_options_json()
        .map_err(|error| {
            format!(
                "{}{}",
                ui_strings.feedback_vpn_start_options_failed_prefix, error
            )
        })?;
    let request_error = crate::platform_callbacks::request_start_vpn(options_json)
        .err()
        .map(|error| error.to_string());
    tokio::time::sleep(Duration::from_millis(350)).await;
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
    let request_error = match crate::platform_callbacks::request_stop_vpn() {
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
    tokio::time::sleep(Duration::from_millis(350)).await;
    Ok(VpnCommandResult {
        snapshot: load_snapshot().await,
        action: VpnCommandAction::Stop,
        profile_name: None,
        request_error,
    })
}

fn request_vpn_restart_if_running(was_vpn_running: bool, ui_strings: &UiStrings) -> Option<String> {
    if !was_vpn_running {
        return None;
    }

    let mut errors = Vec::new();
    if let Err(error) = crate::platform_callbacks::request_stop_vpn() {
        match hmeta_core::shared_core().stop_vpn() {
            Ok(()) => errors.push(format!(
                "{}{}",
                ui_strings.feedback_vpn_stop_fallback_applied_prefix, error
            )),
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
            if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json) {
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

fn per_app_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (PerAppMode, String, String) {
    (
        snapshot.vpn_options.per_app_mode,
        snapshot.vpn_options.trusted_applications.join("\n"),
        snapshot.vpn_options.blocked_applications.join("\n"),
    )
}

fn dns_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (String, String, String) {
    (
        snapshot.vpn_options.dns_servers.join(", "),
        snapshot.vpn_options.dns_fallbacks.join(", "),
        dns_policy_text(&snapshot.vpn_options.dns_nameserver_policy),
    )
}

fn vpn_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (bool, bool, bool, String) {
    (
        snapshot.vpn_options.system_proxy,
        snapshot.vpn_options.dns_hijacking,
        snapshot.vpn_options.allow_bypass,
        snapshot.vpn_options.stack.clone(),
    )
}

fn dns_policy_text(policy: &BTreeMap<String, Vec<String>>) -> String {
    policy
        .iter()
        .map(|(matcher, servers)| format!("{matcher} = {}", servers.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_applications_text(value: &str) -> Vec<String> {
    let mut applications = Vec::new();
    for item in value.split(|character: char| {
        character == ',' || character == ';' || character.is_ascii_whitespace()
    }) {
        let item = item.trim();
        if item.is_empty() || applications.iter().any(|application| application == item) {
            continue;
        }
        applications.push(item.to_owned());
    }
    applications
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

fn add_application_to_text(value: &str, bundle_name: &str) -> String {
    let mut applications = parse_applications_text(value);
    if !applications
        .iter()
        .any(|application| application == bundle_name)
    {
        applications.push(bundle_name.to_owned());
    }
    applications.join("\n")
}

fn remove_application_from_text(value: &str, bundle_name: &str) -> String {
    parse_applications_text(value)
        .into_iter()
        .filter(|application| application != bundle_name)
        .collect::<Vec<_>>()
        .join("\n")
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
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
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
        request_vpn_restart_if_running(was_vpn_running, ui_strings)
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

async fn delete_profile_and_snapshot(
    profile_id: String,
    profile_name: String,
    was_active: bool,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileDeleteResult, String> {
    let mut vpn_errors = Vec::new();
    if was_active && was_vpn_running {
        if let Err(error) = crate::platform_callbacks::request_stop_vpn() {
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
        if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json) {
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

async fn test_proxy_delays_and_snapshot(
    proxies: Vec<String>,
) -> Result<ProxyDelayBatchResult, String> {
    let mut succeeded = 0;
    let mut failed = 0;
    for proxy in proxies {
        match hmeta_core::shared_core()
            .test_proxy_delay_via_controller(&proxy, None, Some(5000))
            .await
        {
            Ok(_) => succeeded += 1,
            Err(_) => failed += 1,
        }
    }
    Ok(ProxyDelayBatchResult {
        snapshot: load_snapshot().await,
        succeeded,
        failed,
    })
}

fn proxy_names_for_delay_test(snapshot: &RuntimeSnapshot) -> Vec<String> {
    let mut names = Vec::new();
    for proxy in snapshot
        .proxy_groups
        .iter()
        .flat_map(|group| group.proxies.iter())
    {
        if !names.iter().any(|name| name == &proxy.name) {
            names.push(proxy.name.clone());
        }
    }
    names
}

async fn close_connection_and_snapshot(connection_id: String) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .close_connection_via_controller(&connection_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
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

async fn clear_logs_and_snapshot() -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .clear_logs()
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
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
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings);
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

fn show_toast(state: &mut State, message: String) -> Command<Action> {
    state.notifications.publish(message);
    Command::none()
}

#[path = "view.rs"]
mod view;

pub(crate) use view::App;
