use super::*;
use crate::reactive_signal::{replace_if_changed, update_if_changed};
use arkit::prelude::*;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreferencesProjection {
    pub locale: UiLocale,
    pub language: LanguagePreference,
    pub theme: ThemePreference,
    pub dark: bool,
    pub applied_color_mode: Option<i32>,
    pub pending_color_mode: Option<i32>,
    pub color_mode_error: Option<String>,
    pub preferences_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionProjection {
    pub lifecycle: VpnLifecycle,
    pub engine_loaded: bool,
    pub running: bool,
    pub vpn_running: bool,
    pub vpn_session_id: Option<String>,
    pub network_protected: bool,
    pub network_protect_error: Option<String>,
    pub bootstrap_error: Option<String>,
    pub runtime_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfilesProjection {
    pub active_profile: Option<String>,
    pub profiles: Vec<paws_model::ProfileSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxiesProjection {
    pub mode: RuntimeMode,
    pub groups: Vec<paws_model::ProxyGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivityProjection {
    pub connections: Vec<paws_model::ConnectionSummary>,
    pub requests: Vec<paws_model::RequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelemetryProjection {
    pub traffic: paws_model::TrafficSnapshot,
    pub history: Vec<TrafficHistoryPoint>,
    pub active_profile_usage: Option<paws_core::ActiveProfileUsage>,
    pub dns: paws_model::DnsSnapshot,
    pub exit_location: paws_model::ExitLocationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourcesProjection {
    pub rules: Vec<paws_model::RuleSummary>,
    pub providers: Vec<paws_model::ProviderSummary>,
    pub geodata: Vec<paws_model::GeodataFileSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagnosticsProjection {
    pub diagnostics: paws_model::ControllerDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogsProjection {
    pub logs: Vec<paws_model::LogEntry>,
    pub recording: paws_core::LogRecordingStatus,
    pub recording_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProfileImportState {
    pub error: Option<String>,
    pub loading: bool,
    pub succeeded: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProxyOperationState {
    pub selection_pending: Option<(String, String)>,
    pub delay_loading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResourceOperationState {
    pub rule_import_loading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticOperationState {
    pub pending: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LogOperationState {
    pub recording_pending: bool,
    pub archive_export_pending: Option<String>,
    pub archive_delete_pending: Option<String>,
}

/// Root-scoped operation coordination, separate from core-backed domain
/// projections. Pages subscribe to these small signals only where progress is
/// rendered, so a spinner cannot invalidate a large list projection.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct UiOperationStores {
    pub vpn: Signal<VpnOperationState>,
    pub profile_import: Signal<ProfileImportState>,
    pub proxy: Signal<ProxyOperationState>,
    pub resources: Signal<ResourceOperationState>,
    pub diagnostics: Signal<DiagnosticOperationState>,
    pub logs: Signal<LogOperationState>,
}

pub(crate) fn use_ui_operation_stores() -> UiOperationStores {
    UiOperationStores {
        vpn: use_signal(VpnOperationState::default),
        profile_import: use_signal(ProfileImportState::default),
        proxy: use_signal(ProxyOperationState::default),
        resources: use_signal(ResourceOperationState::default),
        diagnostics: use_signal(DiagnosticOperationState::default),
        logs: use_signal(LogOperationState::default),
    }
}

impl UiOperationStores {
    pub(crate) fn update_vpn(self, update: impl FnOnce(&mut VpnOperationState)) -> bool {
        update_if_changed(self.vpn, update)
    }

    pub(crate) fn update_profile_import(mut self, update: impl FnOnce(&mut ProfileImportState)) {
        update(&mut self.profile_import.write());
    }

    pub(crate) fn update_proxy(mut self, update: impl FnOnce(&mut ProxyOperationState)) {
        update(&mut self.proxy.write());
    }

    pub(crate) fn update_resources(mut self, update: impl FnOnce(&mut ResourceOperationState)) {
        update(&mut self.resources.write());
    }

    pub(crate) fn update_diagnostics(mut self, update: impl FnOnce(&mut DiagnosticOperationState)) {
        update(&mut self.diagnostics.write());
    }

    pub(crate) fn update_logs(mut self, update: impl FnOnce(&mut LogOperationState)) {
        update(&mut self.logs.write());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsProjection {
    pub config_revision: u64,
    pub active_profile: Option<String>,
    pub vpn: paws_model::VpnOptions,
    pub controller_running: bool,
    pub controller_addr: Option<String>,
    pub controller_access: paws_model::ControllerAccessConfig,
    pub network_ports: paws_model::NetworkPortConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AboutProjection {
    pub about: paws_model::AboutSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuleEditorDrafts {
    pub lookup: Option<RuleLookupState>,
    pub manual: Option<ManualRuleEditorState>,
}

#[derive(Clone)]
pub(crate) struct LocalRuleEditors {
    pub signal: Signal<RuleEditorDrafts>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    query_task: std::sync::Arc<std::sync::Mutex<Option<tokio::task::AbortHandle>>>,
}

impl PartialEq for LocalRuleEditors {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal && std::sync::Arc::ptr_eq(&self.alive, &other.alive)
    }
}

impl LocalRuleEditors {
    pub(crate) fn new(signal: Signal<RuleEditorDrafts>) -> Self {
        Self {
            signal,
            alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            query_task: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub(crate) fn update(&self, update: impl FnOnce(&mut RuleEditorDrafts)) {
        update_if_changed(self.signal, update);
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn dispose(&self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
        self.cancel_query();
    }

    pub(crate) fn replace_query(&self, task: tokio::task::AbortHandle) {
        self.cancel_query();
        *self.query_task.lock().expect("rule lookup task lock") = Some(task);
    }

    pub(crate) fn cancel_query(&self) {
        if let Some(task) = self
            .query_task
            .lock()
            .expect("rule lookup task lock")
            .take()
        {
            task.abort();
        }
    }

    pub(crate) fn clear_query(&self) {
        self.query_task
            .lock()
            .expect("rule lookup task lock")
            .take();
    }
}

/// Typed reactive sources. A runtime publication updates only projections
/// whose values actually changed, so telemetry sampling does not invalidate
/// configuration editors or unrelated routes.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct UiStores {
    versions: Signal<paws_core::RuntimeRevisions>,
    pub preferences: Signal<PreferencesProjection>,
    pub session: Signal<SessionProjection>,
    pub profiles: Signal<ProfilesProjection>,
    pub proxies: Signal<ProxiesProjection>,
    pub activity: Signal<ActivityProjection>,
    pub telemetry: Signal<TelemetryProjection>,
    pub resources: Signal<ResourcesProjection>,
    pub diagnostics: Signal<DiagnosticsProjection>,
    pub logs: Signal<LogsProjection>,
    pub settings: Signal<SettingsProjection>,
    pub about: Signal<AboutProjection>,
}

fn initial_applied_revisions(
    config: Option<paws_core::RuntimeRevisions>,
    telemetry: Option<paws_core::RuntimeRevisions>,
    status: Option<paws_core::RuntimeRevisions>,
    resource: Option<paws_core::RuntimeRevisions>,
) -> paws_core::RuntimeRevisions {
    let mut applied = paws_core::RuntimeRevisions::default();
    for revision in [config, telemetry, status, resource].into_iter().flatten() {
        applied.revision = applied.revision.max(revision.revision);
        applied.observed_at_unix_nanos = applied
            .observed_at_unix_nanos
            .max(revision.observed_at_unix_nanos);
    }
    // Each projection was read independently. Only its owning domain proves
    // that the corresponding payload was actually loaded at this revision;
    // copying cross-domain stamps from a later read can otherwise suppress the
    // provider fetch which would repair an older startup payload.
    applied.config_revision = config.map_or(0, |revision| revision.config_revision);
    applied.telemetry_revision = telemetry.map_or(0, |revision| revision.telemetry_revision);
    applied.status_revision = status.map_or(0, |revision| revision.status_revision);
    applied.resource_revision = resource.map_or(0, |revision| revision.resource_revision);
    applied
}

#[derive(Clone)]
struct InitialUiState {
    versions: paws_core::RuntimeRevisions,
    preferences: PreferencesProjection,
    session: SessionProjection,
    profiles: ProfilesProjection,
    proxies: ProxiesProjection,
    activity: ActivityProjection,
    telemetry: TelemetryProjection,
    resources: ResourcesProjection,
    diagnostics: DiagnosticsProjection,
    logs: LogsProjection,
    settings: SettingsProjection,
    about: AboutProjection,
}

pub(crate) fn use_ui_stores(notifications: NotificationCenter) -> UiStores {
    let initial = use_hook(move || Rc::new(load_initial_ui_state(notifications)));
    let versions = initial.clone();
    let preferences = initial.clone();
    let session = initial.clone();
    let profiles = initial.clone();
    let proxies = initial.clone();
    let activity = initial.clone();
    let telemetry = initial.clone();
    let resources = initial.clone();
    let diagnostics = initial.clone();
    let logs = initial.clone();
    let settings = initial.clone();
    let about = initial;
    UiStores {
        versions: use_signal(move || versions.versions),
        preferences: use_signal(move || preferences.preferences.clone()),
        session: use_signal(move || session.session.clone()),
        profiles: use_signal(move || profiles.profiles.clone()),
        proxies: use_signal(move || proxies.proxies.clone()),
        activity: use_signal(move || activity.activity.clone()),
        telemetry: use_signal(move || telemetry.telemetry.clone()),
        resources: use_signal(move || resources.resources.clone()),
        diagnostics: use_signal(move || diagnostics.diagnostics.clone()),
        logs: use_signal(move || logs.logs.clone()),
        settings: use_signal(move || settings.settings.clone()),
        about: use_signal(move || about.about.clone()),
    }
}

fn load_initial_ui_state(notifications: NotificationCenter) -> InitialUiState {
    let core = paws_core::shared_core();
    let mut initialization_errors = Vec::new();
    let config = core.config_projection().map_err(|error| {
        let message = format!("Configuration state is unavailable: {error}");
        notifications.publish(message.clone());
        initialization_errors.push(message);
    });
    let telemetry_state = core.telemetry_projection().map_err(|error| {
        let message = format!("Telemetry state is unavailable: {error}");
        notifications.publish(message.clone());
        initialization_errors.push(message);
    });
    let status = core.runtime_status_projection().map_err(|error| {
        let message = format!("Runtime status is unavailable: {error}");
        notifications.publish(message.clone());
        initialization_errors.push(message);
    });
    let resource_state = core.resource_projection().map_err(|error| {
        let message = format!("Resource state is unavailable: {error}");
        notifications.publish(message.clone());
        initialization_errors.push(message);
    });
    let (recording, recording_error) = match core.log_recording_status() {
        Ok(status) => {
            let recording_error = status.last_error.clone();
            (status, recording_error)
        }
        Err(error) => {
            let message = format!("Log recording state is unavailable: {error}");
            notifications.publish(message.clone());
            (paws_core::LogRecordingStatus::default(), Some(message))
        }
    };
    let (saved_preferences, preferences_error) = match UiPreferences::load() {
        Ok(preferences) => (preferences, None),
        Err(error) => {
            notifications.publish(error.clone());
            (UiPreferences::default(), Some(error))
        }
    };
    let system = crate::system_preferences::current();
    let preferences = PreferencesProjection {
        locale: saved_preferences.language.resolve(&system.locale),
        language: saved_preferences.language,
        theme: saved_preferences.theme,
        dark: saved_preferences.theme.resolve_dark(system.color_mode),
        applied_color_mode: None,
        pending_color_mode: None,
        color_mode_error: None,
        preferences_error,
    };
    let runtime_error =
        (!initialization_errors.is_empty()).then(|| initialization_errors.join("; "));
    let session = SessionProjection {
        lifecycle: status
            .as_ref()
            .map_or(VpnLifecycle::Stopped, |status| status.vpn_lifecycle),
        engine_loaded: status.as_ref().is_ok_and(|status| status.engine_loaded),
        running: status.as_ref().is_ok_and(|status| status.engine_loaded),
        vpn_running: status.as_ref().is_ok_and(|status| status.vpn_running),
        vpn_session_id: status
            .as_ref()
            .ok()
            .and_then(|status| status.vpn_session_id.clone()),
        network_protected: status.as_ref().is_ok_and(|status| status.network_protected),
        network_protect_error: status
            .as_ref()
            .ok()
            .and_then(|status| status.network_protect_error.clone()),
        bootstrap_error: None,
        runtime_error,
    };
    let profiles = ProfilesProjection {
        active_profile: config
            .as_ref()
            .ok()
            .and_then(|config| config.active_profile.clone()),
        profiles: config
            .as_ref()
            .map_or_else(|_| Vec::new(), |config| config.profiles.clone()),
    };
    let proxies = ProxiesProjection {
        mode: config
            .as_ref()
            .map_or(RuntimeMode::Rule, |config| config.mode),
        groups: resource_state.as_ref().map_or_else(
            |_| {
                status
                    .as_ref()
                    .map_or_else(|_| Vec::new(), |status| status.proxy_groups.clone())
            },
            |resources| resources.proxy_groups.clone(),
        ),
    };
    let activity = ActivityProjection {
        connections: telemetry_state
            .as_ref()
            .map_or_else(|_| Vec::new(), |telemetry| telemetry.connections.clone()),
        requests: telemetry_state.as_ref().map_or_else(
            |_| Vec::new(),
            |telemetry| telemetry.request_history.clone(),
        ),
    };
    let telemetry = TelemetryProjection {
        traffic: telemetry_state.as_ref().map_or_else(
            |_| Default::default(),
            |telemetry| telemetry.traffic.clone(),
        ),
        history: telemetry_state.as_ref().map_or_else(
            |_| Vec::new(),
            |telemetry| telemetry.traffic_history.clone(),
        ),
        active_profile_usage: telemetry_state
            .as_ref()
            .ok()
            .and_then(|telemetry| telemetry.active_profile_usage.clone()),
        dns: telemetry_state
            .as_ref()
            .map_or_else(|_| Default::default(), |telemetry| telemetry.dns.clone()),
        exit_location: status.as_ref().map_or_else(
            |_| Default::default(),
            |status| status.exit_location.clone(),
        ),
    };
    let resources = ResourcesProjection {
        rules: config
            .as_ref()
            .map_or_else(|_| Vec::new(), |config| config.rules.clone()),
        providers: resource_state.as_ref().map_or_else(
            |_| {
                config
                    .as_ref()
                    .map_or_else(|_| Vec::new(), |config| config.providers.clone())
            },
            |resources| resources.providers.clone(),
        ),
        geodata: resource_state.as_ref().map_or_else(
            |_| {
                status
                    .as_ref()
                    .map_or_else(|_| Vec::new(), |status| status.geodata.clone())
            },
            |resources| resources.geodata.clone(),
        ),
    };
    let diagnostics = DiagnosticsProjection {
        diagnostics: telemetry_state.as_ref().map_or_else(
            |_| {
                status.as_ref().map_or_else(
                    |_| Default::default(),
                    |status| status.controller_diagnostics.clone(),
                )
            },
            |telemetry| telemetry.controller_diagnostics.clone(),
        ),
    };
    let logs = LogsProjection {
        logs: telemetry_state
            .as_ref()
            .map_or_else(|_| Vec::new(), |telemetry| telemetry.logs.clone()),
        recording,
        recording_error,
    };
    let settings = SettingsProjection {
        config_revision: config
            .as_ref()
            .map_or(0, |config| config.revisions.config_revision),
        active_profile: config
            .as_ref()
            .ok()
            .and_then(|config| config.active_profile.clone()),
        vpn: config
            .as_ref()
            .map_or_else(|_| Default::default(), |config| config.vpn_options.clone()),
        controller_running: status
            .as_ref()
            .is_ok_and(|status| status.controller_running),
        controller_addr: status
            .as_ref()
            .ok()
            .and_then(|status| status.controller_addr.clone()),
        controller_access: config.as_ref().map_or_else(
            |_| Default::default(),
            |config| config.controller_access.clone(),
        ),
        network_ports: config
            .as_ref()
            .map_or_else(|_| Default::default(), |config| config.network_ports),
    };
    let about = AboutProjection {
        about: status
            .as_ref()
            .map_or_else(|_| Default::default(), |status| status.about.clone()),
    };
    let versions = initial_applied_revisions(
        config.as_ref().ok().map(|projection| projection.revisions),
        telemetry_state
            .as_ref()
            .ok()
            .map(|projection| projection.revisions),
        status.as_ref().ok().map(|projection| projection.revisions),
        resource_state
            .as_ref()
            .ok()
            .map(|projection| projection.revisions),
    );
    InitialUiState {
        versions,
        preferences,
        session,
        profiles,
        proxies,
        activity,
        telemetry,
        resources,
        diagnostics,
        logs,
        settings,
        about,
    }
}

impl UiStores {
    pub(crate) fn runtime_revisions(&self) -> paws_core::RuntimeRevisions {
        *self.versions.peek()
    }

    pub(crate) fn resource_revision(&self) -> u64 {
        self.versions.peek().resource_revision
    }

    pub(crate) fn update_logs(mut self, update: impl FnOnce(&mut LogsProjection)) {
        update(&mut self.logs.write());
    }

    pub(crate) fn apply_config_projection(
        mut self,
        projection: paws_core::ConfigProjection,
    ) -> bool {
        let mut versions = *self.versions.peek();
        if projection.revisions.config_revision < versions.config_revision {
            return false;
        }
        versions.revision = versions.revision.max(projection.revisions.revision);
        versions.config_revision = projection.revisions.config_revision;
        versions.observed_at_unix_nanos = versions
            .observed_at_unix_nanos
            .max(projection.revisions.observed_at_unix_nanos);
        replace_if_changed(&mut self.versions, versions);

        let profile_active = projection.active_profile.clone();
        let profiles_changed = {
            let current = self.profiles.peek();
            current.active_profile != profile_active || current.profiles != projection.profiles
        };
        if profiles_changed {
            let mut current = self.profiles.write();
            current.active_profile = profile_active;
            current.profiles = projection.profiles;
        }
        if self.proxies.peek().mode != projection.mode {
            self.proxies.write().mode = projection.mode;
        }
        if self.resources.peek().rules != projection.rules {
            self.resources.write().rules = projection.rules;
        }
        let settings_changed = {
            let current = self.settings.peek();
            current.config_revision != projection.revisions.config_revision
                || current.active_profile != projection.active_profile
                || current.vpn != projection.vpn_options
                || current.controller_access != projection.controller_access
                || current.network_ports != projection.network_ports
        };
        if settings_changed {
            let mut current = self.settings.write();
            current.config_revision = projection.revisions.config_revision;
            current.active_profile = projection.active_profile;
            current.vpn = projection.vpn_options;
            current.controller_access = projection.controller_access;
            current.network_ports = projection.network_ports;
        }
        true
    }

    pub(crate) fn apply_telemetry_projection(
        mut self,
        projection: paws_core::TelemetryProjection,
    ) -> bool {
        let mut versions = *self.versions.peek();
        if projection.revisions.telemetry_revision < versions.telemetry_revision {
            return false;
        }
        versions.revision = versions.revision.max(projection.revisions.revision);
        versions.telemetry_revision = projection.revisions.telemetry_revision;
        versions.observed_at_unix_nanos = versions
            .observed_at_unix_nanos
            .max(projection.revisions.observed_at_unix_nanos);
        replace_if_changed(&mut self.versions, versions);

        replace_if_changed(
            &mut self.activity,
            ActivityProjection {
                connections: projection.connections,
                requests: projection.request_history,
            },
        );
        let telemetry_changed = {
            let current = self.telemetry.peek();
            current.traffic != projection.traffic
                || current.history != projection.traffic_history
                || current.active_profile_usage != projection.active_profile_usage
                || current.dns != projection.dns
        };
        if telemetry_changed {
            let mut current = self.telemetry.write();
            current.traffic = projection.traffic;
            current.history = projection.traffic_history;
            current.active_profile_usage = projection.active_profile_usage;
            current.dns = projection.dns;
        }
        let logs_changed = {
            let current = self.logs.peek();
            current.logs != projection.logs
                || current.recording_error != projection.log_recording_error
        };
        if logs_changed {
            let mut current = self.logs.write();
            current.logs = projection.logs;
            current.recording_error = projection.log_recording_error;
        }
        if self.diagnostics.peek().diagnostics != projection.controller_diagnostics {
            self.diagnostics.write().diagnostics = projection.controller_diagnostics;
        }
        true
    }

    pub(crate) fn apply_status_projection(
        mut self,
        projection: paws_core::RuntimeStatusProjection,
    ) -> bool {
        let mut versions = *self.versions.peek();
        if projection.revisions.status_revision < versions.status_revision {
            return false;
        }
        versions.revision = versions.revision.max(projection.revisions.revision);
        versions.status_revision = projection.revisions.status_revision;
        versions.observed_at_unix_nanos = versions
            .observed_at_unix_nanos
            .max(projection.revisions.observed_at_unix_nanos);
        replace_if_changed(&mut self.versions, versions);

        let runtime_error = self.session.peek().runtime_error.clone();
        let bootstrap_error = self.session.peek().bootstrap_error.clone();
        replace_if_changed(
            &mut self.session,
            SessionProjection {
                lifecycle: projection.vpn_lifecycle,
                engine_loaded: projection.engine_loaded,
                running: projection.engine_loaded,
                vpn_running: projection.vpn_running,
                vpn_session_id: projection.vpn_session_id,
                network_protected: projection.network_protected,
                network_protect_error: projection.network_protect_error,
                bootstrap_error,
                runtime_error,
            },
        );
        if self.telemetry.peek().exit_location != projection.exit_location {
            self.telemetry.write().exit_location = projection.exit_location;
        }
        let settings_changed = {
            let current = self.settings.peek();
            current.controller_running != projection.controller_running
                || current.controller_addr != projection.controller_addr
        };
        if settings_changed {
            let mut current = self.settings.write();
            current.controller_running = projection.controller_running;
            current.controller_addr = projection.controller_addr;
        }
        replace_if_changed(
            &mut self.about,
            AboutProjection {
                about: projection.about,
            },
        );
        true
    }

    pub(crate) fn apply_resource_projection(
        mut self,
        projection: paws_core::ResourceProjection,
    ) -> bool {
        let mut versions = *self.versions.peek();
        if projection.revisions.resource_revision < versions.resource_revision {
            return false;
        }
        versions.revision = versions.revision.max(projection.revisions.revision);
        versions.resource_revision = projection.revisions.resource_revision;
        versions.observed_at_unix_nanos = versions
            .observed_at_unix_nanos
            .max(projection.revisions.observed_at_unix_nanos);
        replace_if_changed(&mut self.versions, versions);
        let resources_changed = {
            let current = self.resources.peek();
            current.providers != projection.providers || current.geodata != projection.geodata
        };
        if resources_changed {
            let mut current = self.resources.write();
            current.providers = projection.providers;
            current.geodata = projection.geodata;
        }
        if self.proxies.peek().groups != projection.proxy_groups {
            self.proxies.write().groups = projection.proxy_groups;
        }
        true
    }

    pub(crate) fn set_runtime_error(mut self, error: String) {
        if self.session.peek().runtime_error.as_ref() != Some(&error) {
            self.session.write().runtime_error = Some(error);
        }
    }

    pub(crate) fn set_bootstrap_error(mut self, error: Option<String>) {
        if self.session.peek().bootstrap_error != error {
            self.session.write().bootstrap_error = error;
        }
    }

    pub(crate) fn clear_runtime_error(mut self) {
        if self.session.peek().runtime_error.is_some() {
            self.session.write().runtime_error = None;
        }
    }

    pub(crate) fn set_preferences(mut self, next: PreferencesProjection) {
        replace_if_changed(&mut self.preferences, next);
    }
}
