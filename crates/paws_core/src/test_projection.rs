//! Complete projections used only by Core unit tests. Production consumers
//! use explicit runtime domains or narrow mutation receipts instead.
use super::*;
use paws_model::RuleSummary;

fn public_controller_address(state: &CoreState, standalone_meow: bool) -> Option<SocketAddr> {
    if state.platform_vpn_running {
        state.platform_public_controller_addr.or_else(|| {
            (standalone_meow && state.platform_vpn_engine == Some(VpnEngine::Meow))
                .then(|| {
                    state
                        .api_controller
                        .as_ref()
                        .map(|controller| controller.bind_addr)
                })
                .flatten()
        })
    } else {
        state
            .api_controller
            .as_ref()
            .map(|controller| controller.bind_addr)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSnapshot {
    #[serde(default)]
    pub status_revision: u64,
    #[serde(default)]
    pub telemetry_revision: u64,
    #[serde(default)]
    pub resource_revision: u64,
    /// Monotonically increasing identity for the complete runtime projection.
    /// Consumers must ignore snapshots older than the last one they applied.
    #[serde(default)]
    pub revision: u64,
    /// Monotonically increasing identity for persisted profile/configuration data.
    #[serde(default)]
    pub config_revision: u64,
    /// Wall-clock observation time for diagnostics. Ordering must use `revision`.
    #[serde(default)]
    pub observed_at_unix_nanos: u128,
    /// Identity of the currently connected platform VPN session. Consumers
    /// must revalidate it before restarting resources after async settings work.
    #[serde(default)]
    pub vpn_session_id: Option<String>,
    #[serde(default)]
    pub vpn_lifecycle: VpnLifecycle,
    #[serde(default)]
    pub engine_loaded: bool,
    /// Actual running backend; never inferred from the next-connection preference.
    #[serde(default)]
    pub active_engine: Option<VpnEngine>,
    pub running: bool,
    pub vpn_running: bool,
    #[serde(default)]
    pub network_protected: bool,
    #[serde(default)]
    pub network_protect_error: Option<String>,
    #[serde(default)]
    pub controller_running: bool,
    #[serde(default)]
    pub controller_addr: Option<String>,
    #[serde(default)]
    pub controller_diagnostics: ControllerDiagnostics,
    #[serde(default)]
    pub controller_access: ControllerAccessConfig,
    #[serde(default)]
    pub network_ports: NetworkPortConfig,
    pub active_profile: Option<String>,
    pub mode: RuntimeMode,
    pub traffic: TrafficSnapshot,
    #[serde(default)]
    pub traffic_history: Vec<TrafficHistoryPoint>,
    #[serde(default)]
    pub dns: DnsSnapshot,
    #[serde(default)]
    pub vpn_options: VpnOptions,
    #[serde(default)]
    pub exit_location: ExitLocationSnapshot,
    pub proxy_groups: Vec<ProxyGroup>,
    pub profiles: Vec<ProfileSummary>,
    pub rules: Vec<RuleSummary>,
    pub providers: Vec<ProviderSummary>,
    #[serde(default)]
    pub geodata: Vec<GeodataFileSummary>,
    pub logs: Vec<LogEntry>,
    pub connections: Vec<ConnectionSummary>,
    #[serde(default)]
    pub request_history: Vec<RequestSummary>,
    #[serde(default)]
    pub about: AboutSnapshot,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            status_revision: 0,
            telemetry_revision: 0,
            resource_revision: 0,
            revision: 0,
            config_revision: 0,
            observed_at_unix_nanos: 0,
            vpn_session_id: None,
            vpn_lifecycle: VpnLifecycle::Stopped,
            engine_loaded: false,
            active_engine: None,
            running: false,
            vpn_running: false,
            network_protected: false,
            network_protect_error: None,
            controller_running: false,
            controller_addr: None,
            controller_diagnostics: ControllerDiagnostics::default(),
            controller_access: ControllerAccessConfig::default(),
            network_ports: NetworkPortConfig::default(),
            active_profile: None,
            mode: RuntimeMode::Rule,
            traffic: TrafficSnapshot {
                upload_bytes: 0,
                download_bytes: 0,
                upload_speed: 0,
                download_speed: 0,
                tun_upload_bytes: 0,
                tun_download_bytes: 0,
                tun_upload_speed: 0,
                tun_download_speed: 0,
                meow_upload_bytes: 0,
                meow_download_bytes: 0,
                meow_upload_speed: 0,
                meow_download_speed: 0,
            },
            traffic_history: Vec::new(),
            dns: DnsSnapshot::default(),
            vpn_options: VpnOptions::default(),
            exit_location: ExitLocationSnapshot::default(),
            proxy_groups: Vec::new(),
            profiles: Vec::new(),
            rules: Vec::new(),
            providers: Vec::new(),
            geodata: Vec::new(),
            logs: Vec::new(),
            connections: Vec::new(),
            request_history: Vec::new(),
            about: AboutSnapshot::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelemetryProjection {
    pub revisions: RuntimeRevisions,
    pub traffic: TrafficSnapshot,
    pub traffic_history: Vec<TrafficHistoryPoint>,
    pub active_profile_usage: Option<ActiveProfileUsage>,
    pub controller_diagnostics: ControllerDiagnostics,
    pub dns: DnsSnapshot,
    pub startup_health: Option<StartupHealth>,
    pub logs: Vec<LogEntry>,
    pub log_recording_error: Option<String>,
    pub connections: Vec<ConnectionSummary>,
    pub request_history: Vec<RequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveProfileUsage {
    pub profile_id: String,
    pub upload_bytes: u64,
    pub download_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStatusProjection {
    pub revisions: RuntimeRevisions,
    pub vpn_lifecycle: VpnLifecycle,
    pub engine_loaded: bool,
    pub active_engine: Option<VpnEngine>,
    pub vpn_running: bool,
    pub vpn_session_id: Option<String>,
    pub network_protected: bool,
    pub network_protect_error: Option<String>,
    pub controller_running: bool,
    pub controller_addr: Option<String>,
    pub controller_diagnostics: ControllerDiagnostics,
    pub exit_location: ExitLocationSnapshot,
    pub proxy_groups: Vec<ProxyGroup>,
    pub geodata: Vec<GeodataFileSummary>,
    pub about: AboutSnapshot,
}

impl CoreHandle {
    pub(crate) fn telemetry_projection(&self) -> Result<TelemetryProjection, PawsError> {
        let state = self.lock_state()?;
        let active_profile_usage = state.profiles.active_profile().and_then(|profile_id| {
            state
                .platform_profile_traffic
                .as_ref()
                .filter(|(remote_id, _, _)| remote_id == profile_id)
                .map(|(_, upload_bytes, download_bytes)| ActiveProfileUsage {
                    profile_id: profile_id.to_owned(),
                    upload_bytes: *upload_bytes,
                    download_bytes: *download_bytes,
                })
                .or_else(|| {
                    state
                        .profiles
                        .profile(profile_id)
                        .ok()
                        .map(|profile| ActiveProfileUsage {
                            profile_id: profile_id.to_owned(),
                            upload_bytes: profile.upload_bytes,
                            download_bytes: profile.download_bytes,
                        })
                })
        });
        let root = state.profiles.root().to_path_buf();
        let session = platform_vpn_session_id(&state);
        let engine = state.platform_vpn_engine;
        let running = state.platform_vpn_running || self.vpn.is_running();
        let mut projection = TelemetryProjection {
            revisions: runtime_revisions(&state),
            traffic: state.traffic.clone(),
            traffic_history: state.traffic_history.iter().cloned().collect(),
            active_profile_usage,
            controller_diagnostics: state.controller_diagnostics.clone(),
            dns: state.dns.clone(),
            startup_health: None,
            logs: projected_logs(&state),
            log_recording_error: log_recording_error(&state),
            connections: state.connections.clone(),
            request_history: state.request_history.iter().cloned().rev().collect(),
        };
        drop(state);
        if running {
            projection.startup_health =
                StartupHealth::read_current(&root, session.as_deref(), engine);
        }
        Ok(projection)
    }

    pub(crate) fn runtime_status_projection(&self) -> Result<RuntimeStatusProjection, PawsError> {
        let state = self.lock_state()?;
        let native_vpn_running = self.vpn.is_running();
        let public_controller =
            public_controller_address(&state, native_vpn_running && self.platform_ipc()?.is_none());
        Ok(RuntimeStatusProjection {
            revisions: runtime_revisions(&state),
            vpn_lifecycle: vpn_lifecycle(
                state.engine_loaded,
                state.platform_vpn_starting,
                state.platform_vpn_running,
                native_vpn_running,
                state.platform_network_protected,
                state.platform_network_protect_error.as_deref(),
            ),
            engine_loaded: state.engine_loaded,
            active_engine: if native_vpn_running {
                Some(VpnEngine::Meow)
            } else if state.platform_vpn_running {
                state.platform_vpn_engine
            } else {
                None
            },
            vpn_running: native_vpn_running || state.platform_vpn_running,
            vpn_session_id: platform_vpn_session_id(&state),
            network_protected: state.platform_network_protected,
            network_protect_error: state.platform_network_protect_error.clone(),
            controller_running: public_controller.is_some(),
            controller_addr: public_controller.map(|address| address.to_string()),
            controller_diagnostics: state.controller_diagnostics.clone(),
            exit_location: state.exit_location.clone(),
            proxy_groups: state.proxy_groups.clone(),
            geodata: state.geodata.clone(),
            about: about_snapshot(),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<RuntimeSnapshot, PawsError> {
        let state = self.lock_state()?;
        let native_vpn_running = self.vpn.is_running();
        let public_controller =
            public_controller_address(&state, native_vpn_running && self.platform_ipc()?.is_none());
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let profiles = profile_summaries(&state);
        let controller_diagnostics = state.controller_diagnostics.clone();
        let vpn_running = native_vpn_running || state.platform_vpn_running;
        Ok(RuntimeSnapshot {
            status_revision: state.status_revision,
            telemetry_revision: state.telemetry_revision,
            resource_revision: state.resource_revision,
            revision: state.revision,
            config_revision: state.config_revision,
            observed_at_unix_nanos: state.observed_at_unix_nanos,
            vpn_session_id: platform_vpn_session_id(&state),
            vpn_lifecycle: vpn_lifecycle(
                state.engine_loaded,
                state.platform_vpn_starting,
                state.platform_vpn_running,
                native_vpn_running,
                state.platform_network_protected,
                state.platform_network_protect_error.as_deref(),
            ),
            engine_loaded: state.engine_loaded,
            active_engine: if native_vpn_running {
                Some(VpnEngine::Meow)
            } else if state.platform_vpn_running {
                state.platform_vpn_engine
            } else {
                None
            },
            running: state.engine_loaded,
            vpn_running,
            network_protected: state.platform_network_protected,
            network_protect_error: state.platform_network_protect_error.clone(),
            controller_running: public_controller.is_some(),
            controller_addr: public_controller.map(|address| address.to_string()),
            controller_diagnostics,
            controller_access: state.controller_access.clone(),
            network_ports: state.network_ports,
            active_profile,
            mode: state.mode,
            traffic: state.traffic.clone(),
            traffic_history: state.traffic_history.iter().cloned().collect(),
            dns: state.dns.clone(),
            vpn_options: state.vpn_options.clone(),
            exit_location: state.exit_location.clone(),
            proxy_groups: state.proxy_groups.clone(),
            profiles,
            rules: state.runtime_rules.clone(),
            providers: state.providers.clone(),
            geodata: state.geodata.clone(),
            logs: projected_logs(&state),
            connections: state.connections.clone(),
            request_history: state.request_history.iter().cloned().rev().collect(),
            about: about_snapshot(),
        })
    }
}
