use axum::extract::{Request, State as AxumState};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response as AxumResponse;
use futures::StreamExt;
use meow_common::sniffer::SnifferConfig;
use meow_common::{AdapterType, ConnType, Metadata, Network, TunnelMode};
use meow_config::{
    proxy_provider::ProxyProvider, raw::RawConfig, rule_provider::RuleProvider, Config,
    NamedListener,
};
use meow_listener::MixedListener;
use meow_tunnel::rule_ir::LazyMatchOutcome;
use meow_tunnel::Tunnel;
use once_cell::sync::Lazy;
use paws_model::{
    from_json, to_json, AboutSnapshot, ConnectionSummary, ControllerAccessConfig,
    ControllerDiagnostics, DnsSnapshot, ExitLocationSnapshot, GeodataFileSummary, LogEntry,
    ManualRuleMutation, ManualRuleSpec, NetworkPortConfig, PawsError, ProfileSummary,
    ProviderProxySummary, ProviderSummary, ProxyGroup, ProxyItem, RequestSummary, RuntimeMode,
    RuntimeSnapshot, TrafficHistoryPoint, TrafficSnapshot, VpnLifecycle, VpnOptions,
};
use paws_profile::{normalize_profile_content, ProfileCheckpoint, ProfileStore};
use paws_vpn::{TunSession, TunStats, VpnLifecycle as NativeVpnLifecycle};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

mod controller;
mod exit_location;
pub mod http_client;
mod log_recording;
mod logging;
mod platform_ipc;
mod platform_owner;
mod providers;
mod routing;
mod runtime_snapshot;
mod subscription;
mod telemetry;

pub use controller::shared_core;
use controller::*;
use exit_location::*;
pub use log_recording::{LogArchiveSummary, LogRecordingStatus};
use log_recording::{RecordedLogBuffer, RuntimeLogBuffer, MAX_IN_MEMORY_LOGS};
use logging::*;
use platform_ipc::PlatformIpc;
use platform_owner::{
    JournalRead, PlatformVpnOwnerJournal, PlatformVpnOwnerLease, PlatformVpnOwnerLeaseObservation,
    PlatformVpnOwnerLeaseRecord, PlatformVpnOwnerLeaseRole, PlatformVpnOwnerPhase, ProcessIdentity,
};
use providers::*;
use routing::*;
use runtime_snapshot::*;
use subscription::*;
use telemetry::*;

static CORE: Lazy<Arc<CoreHandle>> = Lazy::new(|| Arc::new(CoreHandle::new()));
static APP_HOME_CONFIGURATION_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static RUNTIME_LOGS: Lazy<Arc<Mutex<RuntimeLogBuffer>>> =
    Lazy::new(|| Arc::new(Mutex::new(RuntimeLogBuffer::default())));
static API_LOG_TXS: Lazy<
    Arc<Mutex<VecDeque<tokio::sync::broadcast::Sender<meow_api::log_stream::LogMessage>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(VecDeque::new())));
static INSTALL_RUNTIME_LOG_LAYER: Once = Once::new();
const MAX_API_LOG_SENDERS: usize = 8;
const MAX_REQUEST_HISTORY: usize = 128;
const MAX_TRAFFIC_HISTORY: usize = 32;
const PLATFORM_VPN_START_DEADLINE: Duration = Duration::from_secs(120);
const PLATFORM_HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(15);
const PLATFORM_HEARTBEAT_WAKE_GRACE: Duration = Duration::from_secs(6);
const PLATFORM_OS_STOP_RECOVERY_DEADLINE: Duration = Duration::from_secs(30);
const PLATFORM_OS_STOP_RECOVERY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const EXIT_LOCATION_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const EXIT_LOCATION_RETRY_INTERVAL: Duration = Duration::from_secs(60);
const MIXED_LISTENER_READY_TIMEOUT: Duration = Duration::from_secs(2);
const MIXED_LISTENER_READY_RETRY: Duration = Duration::from_millis(25);
const RUNTIME_UI_CACHE_FILE: &str = "runtime/ui-cache.json";
const RUNTIME_UI_CACHE_VERSION: u32 = 1;
const APP_VERSION: &str = "1.0.0";
const MEOW_RS_VERSION: &str = "0.21.2";
const ARKIT_REV: &str = env!("PAWS_ARKIT_REV");
const RUST_VERSION: &str = "1.89";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlatformStartOutcome {
    #[default]
    Idle,
    Pending,
    Connected,
    Failed,
    Cancelled,
}

/// Exact-owner result used to serialize a platform stop with an outstanding
/// HarmonyOS ability-start Promise. Some system versions keep that Promise
/// pending after the Extension has already attached, so stop coordination
/// must observe the Extension bind independently from the dispatch Promise.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformAttachOutcome {
    Attached,
    Delivered,
    Terminal,
    Superseded,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PlatformVpnState {
    start_attempt_id: String,
    start_outcome: PlatformStartOutcome,
    delivery_observed: bool,
    extension_attached: bool,
    stop_requested: bool,
    extension_owner_pid: u32,
    extension_owner_start_time: u64,
    cleanup_complete: bool,
    starting: bool,
    running: bool,
    network_protected: bool,
    network_protect_error: Option<String>,
    updated_at: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformVpnControl {
    mode: RuntimeMode,
    #[serde(default)]
    global_proxy: Option<String>,
    #[serde(default)]
    active_profile: Option<String>,
    #[serde(default)]
    proxy_selections: BTreeMap<String, String>,
    updated_at: u128,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PlatformVpnTelemetry {
    updated_at: u128,
    active_profile: Option<String>,
    traffic: TrafficSnapshot,
    traffic_history: Vec<TrafficHistoryPoint>,
    dns: DnsSnapshot,
    connections: Vec<ConnectionSummary>,
    request_history: Vec<RequestSummary>,
    logs: Vec<LogEntry>,
    profile_upload_bytes: u64,
    profile_download_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeUiCache {
    version: u32,
    active_profile: String,
    profile_updated_at: Option<String>,
    proxy_groups: Vec<ProxyGroup>,
}

struct ApiControllerRuntime {
    bind_addr: SocketAddr,
    client_addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
    memory_task: tokio::task::JoinHandle<()>,
    raw_config: Arc<parking_lot::RwLock<RawConfig>>,
    baseline_raw_config: RawConfig,
    proxy_providers: Arc<dashmap::DashMap<String, Arc<ProxyProvider>>>,
    config_revision: Arc<AtomicU64>,
    synced_revision: u64,
    memory_in_use_bytes: Arc<AtomicU64>,
    memory_limit_bytes: Arc<AtomicU64>,
}

struct MixedListenerRuntime {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MixedListenerRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Deserialize)]
struct ControllerMemoryFrame {
    inuse: u64,
    oslimit: u64,
}

impl Drop for ApiControllerRuntime {
    fn drop(&mut self) {
        self.task.abort();
        self.memory_task.abort();
    }
}

impl ApiControllerRuntime {
    async fn shutdown(&mut self) {
        self.task.abort();
        self.memory_task.abort();
        let _ = (&mut self.task).await;
        let _ = (&mut self.memory_task).await;
    }
}

struct CoreState {
    revision: u64,
    config_revision: u64,
    telemetry_revision: u64,
    status_revision: u64,
    resource_revision: u64,
    observed_at_unix_nanos: u128,
    engine_loaded: bool,
    platform_vpn_starting: bool,
    platform_vpn_running: bool,
    platform_vpn_intent_epoch: u64,
    platform_os_stop_epoch: u64,
    platform_os_stop_attempt_id: String,
    platform_os_stop_in_flight: bool,
    platform_vpn_issuer_lease: Option<PlatformVpnOwnerLease>,
    platform_vpn_extension_lease: Option<PlatformVpnOwnerLease>,
    platform_start_sequence: u64,
    platform_start_attempt_id: String,
    platform_start_outcome: PlatformStartOutcome,
    platform_start_delivery_observed: bool,
    platform_extension_attached: bool,
    platform_stop_requested: bool,
    platform_extension_owner_pid: u32,
    platform_extension_owner_start_time: u64,
    platform_vpn_cleanup_complete: bool,
    platform_network_protected: bool,
    platform_network_protect_error: Option<String>,
    platform_vpn_state_updated_at: u128,
    platform_remote_state_updated_at: u128,
    platform_remote_state_seen_at: Option<Instant>,
    platform_remote_stale_since: Option<Instant>,
    /// The UI's monotonic heartbeat watchdog, rather than an ordinary native
    /// failure, proved that this exact Extension owner stopped responding.
    /// This is intentionally process-local and is cleared only by cleanup or
    /// replacement ownership.
    platform_watchdog_cleanup_recoverable: bool,
    platform_vpn_control_updated_at: u128,
    runtime_ui_cache_writes_enabled: bool,
    mode: RuntimeMode,
    profiles: ProfileStore,
    tunnel: Option<Tunnel>,
    sniffer_config: SnifferConfig,
    proxy_groups: Vec<ProxyGroup>,
    providers: Vec<ProviderSummary>,
    runtime_rules: Vec<paws_model::RuleSummary>,
    provider_refresh: HashMap<String, ProviderRefreshState>,
    resource_operation_sequences: HashMap<String, u64>,
    traffic: TrafficSnapshot,
    traffic_history: VecDeque<TrafficHistoryPoint>,
    dns: DnsSnapshot,
    connections: Vec<ConnectionSummary>,
    platform_logs: Option<Vec<LogEntry>>,
    platform_profile_traffic: Option<(String, u64, u64)>,
    geodata: Vec<GeodataFileSummary>,
    last_traffic_sample: Option<(Instant, u64, u64)>,
    last_meow_traffic_sample: Option<(Instant, u64, u64)>,
    logs: RecordedLogBuffer,
    request_history: VecDeque<RequestSummary>,
    vpn_options: VpnOptions,
    controller_access: ControllerAccessConfig,
    network_ports: NetworkPortConfig,
    exit_location: ExitLocationSnapshot,
    last_exit_location_check: Option<Instant>,
    exit_location_revision: u64,
    api_controller: Option<ApiControllerRuntime>,
    controller_diagnostics: ControllerDiagnostics,
    controller_config_sync_count: u64,
    last_controller_config_sync_at: Option<String>,
    last_controller_config_sync_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ProviderRefreshState {
    refreshed_at: String,
    error: Option<String>,
}

fn invalidate_exit_location(state: &mut CoreState) {
    state.exit_location = ExitLocationSnapshot::default();
    state.last_exit_location_check = None;
    state.exit_location_revision = state.exit_location_revision.wrapping_add(1);
}

fn runtime_revisions(state: &CoreState) -> RuntimeRevisions {
    RuntimeRevisions {
        revision: state.revision,
        config_revision: state.config_revision,
        telemetry_revision: state.telemetry_revision,
        status_revision: state.status_revision,
        resource_revision: state.resource_revision,
        observed_at_unix_nanos: state.observed_at_unix_nanos,
    }
}

fn projected_logs(state: &CoreState) -> Vec<LogEntry> {
    if !state.logs.enabled() {
        return Vec::new();
    }
    let local_logs = merged_logs(&state.logs);
    state
        .platform_logs
        .as_ref()
        .map_or(local_logs.clone(), |platform_logs| {
            merge_platform_logs(local_logs, platform_logs)
        })
}

fn log_recording_error(state: &CoreState) -> Option<String> {
    let local_error = state.logs.last_error().map(ToOwned::to_owned);
    let runtime_error = match RUNTIME_LOGS.lock() {
        Ok(logs) => logs.last_error().map(ToOwned::to_owned),
        Err(_) => Some("runtime log recording lock poisoned".to_owned()),
    };
    match (local_error, runtime_error) {
        (Some(local), Some(runtime)) if local != runtime => Some(format!(
            "application log writer: {local}; runtime log writer: {runtime}"
        )),
        (Some(error), _) | (_, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn validate_supported_vpn_options(options: &VpnOptions) -> Result<(), PawsError> {
    if options.system_proxy {
        return Err(PawsError::Core(
            "system proxy management is not supported by the HarmonyOS VPN platform".to_owned(),
        ));
    }
    if options.allow_bypass {
        return Err(PawsError::Core(
            "application bypass is not supported by the HarmonyOS VPN platform".to_owned(),
        ));
    }
    Ok(())
}

fn platform_vpn_telemetry_projection(state: &CoreState) -> PlatformVpnTelemetry {
    let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
    let (profile_upload_bytes, profile_download_bytes) = active_profile
        .as_deref()
        .and_then(|profile_id| state.profiles.profile(profile_id).ok())
        .map(|profile| (profile.upload_bytes, profile.download_bytes))
        .unwrap_or_default();
    PlatformVpnTelemetry {
        updated_at: now_unix_nanos(),
        active_profile,
        traffic: state.traffic.clone(),
        traffic_history: state.traffic_history.iter().cloned().collect(),
        dns: state.dns.clone(),
        connections: state.connections.clone(),
        request_history: state.request_history.iter().cloned().rev().collect(),
        logs: projected_logs(state),
        profile_upload_bytes,
        profile_download_bytes,
    }
}

fn sample_controller_diagnostics(state: &CoreState) -> ControllerDiagnostics {
    state
        .api_controller
        .as_ref()
        .map(|controller| ControllerDiagnostics {
            memory_in_use_bytes: controller.memory_in_use_bytes.load(Ordering::Relaxed),
            memory_limit_bytes: controller.memory_limit_bytes.load(Ordering::Relaxed),
            config_sync_count: state.controller_config_sync_count,
            last_config_sync_at: state.last_controller_config_sync_at.clone(),
            last_config_sync_error: state.last_controller_config_sync_error.clone(),
        })
        .unwrap_or_else(|| ControllerDiagnostics {
            config_sync_count: state.controller_config_sync_count,
            last_config_sync_at: state.last_controller_config_sync_at.clone(),
            last_config_sync_error: state.last_controller_config_sync_error.clone(),
            ..ControllerDiagnostics::default()
        })
}

fn sample_runtime_resources(state: &mut CoreState) -> bool {
    let previous_proxy_groups = state.proxy_groups.clone();
    let previous_providers = state.providers.clone();
    if let Some(tunnel) = state.tunnel.clone() {
        // Periodic observation updates the in-memory resource projection only.
        // User-driven selection/configuration paths own the UI-cache write;
        // sampling an unchanged tunnel must not enqueue a filesystem write
        // every second.
        let mut refreshed = proxy_groups_from_tunnel(&tunnel);
        preserve_proxy_group_member_order(&state.proxy_groups, &mut refreshed);
        state.proxy_groups = refreshed;
    }
    if let Some(proxy_providers) = state
        .api_controller
        .as_ref()
        .map(|controller| Arc::clone(&controller.proxy_providers))
    {
        enrich_proxy_provider_members(&mut state.providers, &proxy_providers);
    }
    state.proxy_groups != previous_proxy_groups || state.providers != previous_providers
}

fn platform_vpn_session_id(state: &CoreState) -> Option<String> {
    (state.platform_vpn_running
        && state.platform_start_outcome == PlatformStartOutcome::Connected
        && !state.platform_start_attempt_id.is_empty())
    .then(|| state.platform_start_attempt_id.clone())
}

impl Default for CoreState {
    fn default() -> Self {
        let profiles = ProfileStore::open_default_or_unavailable();
        let proxy_groups = load_runtime_ui_cache(&profiles)
            .map(|cache| cache.proxy_groups)
            .unwrap_or_default();
        let logs = RecordedLogBuffer::new(profiles.root());
        let geodata = profiles.geodata_files();
        let vpn_options = VpnOptions::default();
        let dns = dns_snapshot(&vpn_options, None);
        Self {
            revision: 1,
            config_revision: 1,
            telemetry_revision: 0,
            status_revision: 1,
            resource_revision: 1,
            observed_at_unix_nanos: now_unix_nanos(),
            engine_loaded: false,
            platform_vpn_starting: false,
            platform_vpn_running: false,
            platform_vpn_intent_epoch: 0,
            platform_os_stop_epoch: 0,
            platform_os_stop_attempt_id: String::new(),
            platform_os_stop_in_flight: false,
            platform_vpn_issuer_lease: None,
            platform_vpn_extension_lease: None,
            platform_start_sequence: 0,
            platform_start_attempt_id: String::new(),
            platform_start_outcome: PlatformStartOutcome::Idle,
            platform_start_delivery_observed: false,
            platform_extension_attached: false,
            platform_stop_requested: false,
            platform_extension_owner_pid: 0,
            platform_extension_owner_start_time: 0,
            platform_vpn_cleanup_complete: false,
            platform_network_protected: false,
            platform_network_protect_error: None,
            platform_vpn_state_updated_at: 0,
            platform_remote_state_updated_at: 0,
            platform_remote_state_seen_at: None,
            platform_remote_stale_since: None,
            platform_watchdog_cleanup_recoverable: false,
            platform_vpn_control_updated_at: 0,
            runtime_ui_cache_writes_enabled: true,
            mode: RuntimeMode::Rule,
            profiles,
            tunnel: None,
            sniffer_config: SnifferConfig::default(),
            proxy_groups,
            providers: Vec::new(),
            runtime_rules: Vec::new(),
            provider_refresh: HashMap::new(),
            resource_operation_sequences: HashMap::new(),
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
            traffic_history: VecDeque::with_capacity(MAX_TRAFFIC_HISTORY),
            dns,
            connections: Vec::new(),
            platform_logs: None,
            platform_profile_traffic: None,
            geodata,
            last_traffic_sample: None,
            last_meow_traffic_sample: None,
            logs,
            request_history: VecDeque::with_capacity(MAX_REQUEST_HISTORY),
            vpn_options,
            controller_access: ControllerAccessConfig::default(),
            network_ports: NetworkPortConfig::default(),
            exit_location: ExitLocationSnapshot::default(),
            last_exit_location_check: None,
            exit_location_revision: 0,
            api_controller: None,
            controller_diagnostics: ControllerDiagnostics::default(),
            controller_config_sync_count: 0,
            last_controller_config_sync_at: None,
            last_controller_config_sync_error: None,
        }
    }
}

pub struct CoreHandle {
    state: Mutex<CoreState>,
    platform_ipc: Mutex<Option<Arc<PlatformIpc>>>,
    platform_start_tx: tokio::sync::watch::Sender<PlatformStartEvent>,
    platform_vpn_event_sequence: AtomicU64,
    platform_vpn_event_tx: tokio::sync::watch::Sender<u64>,
    runtime_revision_tx: tokio::sync::watch::Sender<RuntimeRevisions>,
    runtime_services_task_started: AtomicBool,
    config_reload_lock: tokio::sync::Mutex<()>,
    vpn_operation_lock: tokio::sync::Mutex<()>,
    exit_location_refresh_lock: tokio::sync::Mutex<()>,
    mixed_listener: Mutex<Option<MixedListenerRuntime>>,
    vpn: TunSession,
    api_controller_enabled: bool,
    api_controller_addr_override: Option<SocketAddr>,
    #[cfg(test)]
    fail_next_config_reload: AtomicBool,
    #[cfg(test)]
    fail_next_profile_rollback: AtomicBool,
    #[cfg(test)]
    fail_next_platform_vpn_publish: AtomicBool,
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformSharedMemoryFds {
    pub ashmem_fd: i32,
    pub notification_fd: i32,
}

/// Configure the process-wide application data root before the shared Core is
/// initialized. Repeating the same absolute path is idempotent; changing it
/// later is rejected so one process cannot silently split its state across
/// two profile stores.
pub fn configure_app_home(home_dir: &Path) -> Result<(), PawsError> {
    validate_app_home_path(home_dir)?;
    let _configuration_guard = APP_HOME_CONFIGURATION_LOCK
        .lock()
        .map_err(|_| PawsError::Core("app home configuration lock poisoned".to_owned()))?;

    if let Some(configured) = std::env::var_os("PAWS_HOME") {
        let configured = PathBuf::from(configured);
        if configured != home_dir {
            return Err(PawsError::Core(format!(
                "PAWS_HOME is already configured as {}; refusing to replace it with {}",
                configured.display(),
                home_dir.display()
            )));
        }
    }

    if let Some(core) = Lazy::get(&CORE) {
        let state = core
            .state
            .lock()
            .map_err(|_| PawsError::Core("core state lock poisoned".to_owned()))?;
        if state.profiles.root() != home_dir {
            return Err(PawsError::Core(format!(
                "core is already initialized at {}; refusing late app home {}",
                state.profiles.root().display(),
                home_dir.display()
            )));
        }
    }

    std::env::set_var("PAWS_HOME", home_dir);
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeRevisions {
    pub revision: u64,
    pub config_revision: u64,
    pub telemetry_revision: u64,
    pub status_revision: u64,
    pub resource_revision: u64,
    pub observed_at_unix_nanos: u128,
}

struct RuntimeServicesTaskGuard {
    core: std::sync::Weak<CoreHandle>,
}

impl Drop for RuntimeServicesTaskGuard {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            core.runtime_services_task_started
                .store(false, Ordering::Release);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProjection {
    pub revisions: RuntimeRevisions,
    pub active_profile: Option<String>,
    pub mode: RuntimeMode,
    pub vpn_options: VpnOptions,
    pub controller_access: ControllerAccessConfig,
    pub network_ports: NetworkPortConfig,
    pub profiles: Vec<ProfileSummary>,
    pub rules: Vec<paws_model::RuleSummary>,
    pub providers: Vec<ProviderSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryProjection {
    pub revisions: RuntimeRevisions,
    pub traffic: TrafficSnapshot,
    pub traffic_history: Vec<TrafficHistoryPoint>,
    pub active_profile_usage: Option<ActiveProfileUsage>,
    pub controller_diagnostics: ControllerDiagnostics,
    pub dns: DnsSnapshot,
    pub logs: Vec<LogEntry>,
    pub log_recording_error: Option<String>,
    pub connections: Vec<ConnectionSummary>,
    pub request_history: Vec<RequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProfileUsage {
    pub profile_id: String,
    pub upload_bytes: u64,
    pub download_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusProjection {
    pub revisions: RuntimeRevisions,
    pub vpn_lifecycle: VpnLifecycle,
    pub engine_loaded: bool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProjection {
    pub revisions: RuntimeRevisions,
    pub proxy_groups: Vec<ProxyGroup>,
    pub providers: Vec<ProviderSummary>,
    pub geodata: Vec<GeodataFileSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileImportReceipt {
    pub profile_id: String,
    pub config: ConfigProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleImportReceipt {
    pub imported_rule_ids: Vec<String>,
    pub config: ConfigProjection,
}

#[derive(Debug)]
pub struct PreparedProfileImport {
    name: String,
    source: String,
    raw_yaml: String,
    subscription_url: Option<String>,
    subscription_user_info: Option<paws_model::SubscriptionUserInfo>,
    subscription_metadata: Option<paws_model::SubscriptionMetadata>,
}

#[derive(Debug)]
pub struct PreparedProfileRefresh {
    raw_yaml: String,
    subscription_user_info: Option<paws_model::SubscriptionUserInfo>,
    subscription_metadata: Option<paws_model::SubscriptionMetadata>,
}

#[derive(Debug)]
pub struct PreparedRuleImport {
    source: String,
    rules_text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PlatformStartEvent {
    attempt_id: String,
    outcome: PlatformStartOutcome,
    delivery_observed: bool,
    extension_attached: bool,
    cleanup_complete: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ManualRuleApplyResult {
    pub mutation: ManualRuleMutation,
    pub live_updated: bool,
    pub rule_mode_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleLookupInputKind {
    Domain,
    Ip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleLookupResult {
    pub query: String,
    pub input_kind: RuleLookupInputKind,
    pub resolved_ip: Option<String>,
    pub resolution_attempted: bool,
    pub matched: bool,
    pub rule_type: Option<String>,
    pub rule_payload: Option<String>,
    pub target: String,
    pub rule_line: Option<String>,
}

impl CoreHandle {
    fn new() -> Self {
        // Serialize the first profile-store read with configure_app_home so a
        // concurrent lazy initialization cannot capture the old process
        // directory immediately before PAWS_HOME is installed.
        let _home_guard = APP_HOME_CONFIGURATION_LOCK
            .lock()
            .expect("app home configuration lock must not fail");
        install_runtime_log_layer();
        let (platform_start_tx, _) = tokio::sync::watch::channel(PlatformStartEvent::default());
        let (platform_vpn_event_tx, _) = tokio::sync::watch::channel(0);
        let initial_state = CoreState::default();
        let (runtime_revision_tx, _) =
            tokio::sync::watch::channel(runtime_revisions(&initial_state));
        Self {
            state: Mutex::new(initial_state),
            platform_ipc: Mutex::new(None),
            platform_start_tx,
            platform_vpn_event_sequence: AtomicU64::new(0),
            platform_vpn_event_tx,
            runtime_revision_tx,
            runtime_services_task_started: AtomicBool::new(false),
            config_reload_lock: tokio::sync::Mutex::new(()),
            vpn_operation_lock: tokio::sync::Mutex::new(()),
            exit_location_refresh_lock: tokio::sync::Mutex::new(()),
            mixed_listener: Mutex::new(None),
            vpn: TunSession::default(),
            api_controller_enabled: true,
            api_controller_addr_override: None,
            #[cfg(test)]
            fail_next_config_reload: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_profile_rollback: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_platform_vpn_publish: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    fn new_with_profile_root(root: impl Into<std::path::PathBuf>) -> Self {
        install_runtime_log_layer();
        let (platform_start_tx, _) = tokio::sync::watch::channel(PlatformStartEvent::default());
        let (platform_vpn_event_tx, _) = tokio::sync::watch::channel(0);
        let observed_at_unix_nanos = now_unix_nanos();
        let initial_revisions = RuntimeRevisions {
            revision: 1,
            config_revision: 1,
            telemetry_revision: 0,
            status_revision: 1,
            resource_revision: 1,
            observed_at_unix_nanos,
        };
        let (runtime_revision_tx, _) = tokio::sync::watch::channel(initial_revisions);
        let profiles = ProfileStore::open(root).expect("test profile store");
        let log_root = profiles.root().to_path_buf();
        log_recording::set_recording_enabled(&log_root, true)
            .expect("enable log recording for core tests");
        let proxy_groups = load_runtime_ui_cache(&profiles)
            .map(|cache| cache.proxy_groups)
            .unwrap_or_default();
        let geodata = profiles.geodata_files();
        let vpn_options = VpnOptions::default();
        let dns = dns_snapshot(&vpn_options, None);
        Self {
            state: Mutex::new(CoreState {
                revision: 1,
                config_revision: 1,
                telemetry_revision: 0,
                status_revision: 1,
                resource_revision: 1,
                observed_at_unix_nanos,
                engine_loaded: false,
                platform_vpn_starting: false,
                platform_vpn_running: false,
                platform_vpn_intent_epoch: 0,
                platform_os_stop_epoch: 0,
                platform_os_stop_attempt_id: String::new(),
                platform_os_stop_in_flight: false,
                platform_vpn_issuer_lease: None,
                platform_vpn_extension_lease: None,
                platform_start_sequence: 0,
                platform_start_attempt_id: String::new(),
                platform_start_outcome: PlatformStartOutcome::Idle,
                platform_start_delivery_observed: false,
                platform_extension_attached: false,
                platform_stop_requested: false,
                platform_extension_owner_pid: 0,
                platform_extension_owner_start_time: 0,
                platform_vpn_cleanup_complete: false,
                platform_network_protected: false,
                platform_network_protect_error: None,
                platform_vpn_state_updated_at: 0,
                platform_remote_state_updated_at: 0,
                platform_remote_state_seen_at: None,
                platform_remote_stale_since: None,
                platform_watchdog_cleanup_recoverable: false,
                platform_vpn_control_updated_at: 0,
                runtime_ui_cache_writes_enabled: true,
                mode: RuntimeMode::Rule,
                profiles,
                tunnel: None,
                sniffer_config: SnifferConfig::default(),
                proxy_groups,
                providers: Vec::new(),
                runtime_rules: Vec::new(),
                provider_refresh: HashMap::new(),
                resource_operation_sequences: HashMap::new(),
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
                traffic_history: VecDeque::with_capacity(MAX_TRAFFIC_HISTORY),
                dns,
                connections: Vec::new(),
                platform_logs: None,
                platform_profile_traffic: None,
                geodata,
                last_traffic_sample: None,
                last_meow_traffic_sample: None,
                logs: RecordedLogBuffer::new(log_root),
                request_history: VecDeque::with_capacity(MAX_REQUEST_HISTORY),
                vpn_options,
                controller_access: ControllerAccessConfig::default(),
                network_ports: NetworkPortConfig::default(),
                exit_location: ExitLocationSnapshot::default(),
                last_exit_location_check: None,
                exit_location_revision: 0,
                api_controller: None,
                controller_diagnostics: ControllerDiagnostics::default(),
                controller_config_sync_count: 0,
                last_controller_config_sync_at: None,
                last_controller_config_sync_error: None,
            }),
            platform_ipc: Mutex::new(None),
            platform_start_tx,
            platform_vpn_event_sequence: AtomicU64::new(0),
            platform_vpn_event_tx,
            runtime_revision_tx,
            runtime_services_task_started: AtomicBool::new(false),
            config_reload_lock: tokio::sync::Mutex::new(()),
            vpn_operation_lock: tokio::sync::Mutex::new(()),
            exit_location_refresh_lock: tokio::sync::Mutex::new(()),
            mixed_listener: Mutex::new(None),
            vpn: TunSession::default(),
            api_controller_enabled: false,
            api_controller_addr_override: None,
            fail_next_config_reload: AtomicBool::new(false),
            fail_next_profile_rollback: AtomicBool::new(false),
            fail_next_platform_vpn_publish: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    fn new_with_profile_root_and_controller(
        root: impl Into<std::path::PathBuf>,
        addr: SocketAddr,
    ) -> Self {
        let mut core = Self::new_with_profile_root(root);
        core.api_controller_enabled = true;
        core.api_controller_addr_override = Some(addr);
        core
    }

    pub fn shared() -> Arc<Self> {
        let core = CORE.clone();
        core.ensure_runtime_services();
        core
    }

    pub fn subscribe_runtime_revisions(&self) -> tokio::sync::watch::Receiver<RuntimeRevisions> {
        self.runtime_revision_tx.subscribe()
    }

    pub fn config_projection(&self) -> Result<ConfigProjection, PawsError> {
        let state = self.lock_state()?;
        Ok(Self::config_projection_locked(&state))
    }

    fn config_projection_locked(state: &CoreState) -> ConfigProjection {
        ConfigProjection {
            revisions: runtime_revisions(&state),
            active_profile: state.profiles.active_profile().map(ToOwned::to_owned),
            mode: state.mode,
            vpn_options: state.vpn_options.clone(),
            controller_access: state.controller_access.clone(),
            network_ports: state.network_ports,
            profiles: state.profiles.summaries(),
            rules: state.profiles.active_rules(),
            providers: state.providers.clone(),
        }
    }

    pub fn telemetry_projection(&self) -> Result<TelemetryProjection, PawsError> {
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
        Ok(TelemetryProjection {
            revisions: runtime_revisions(&state),
            traffic: state.traffic.clone(),
            traffic_history: state.traffic_history.iter().cloned().collect(),
            active_profile_usage,
            controller_diagnostics: state.controller_diagnostics.clone(),
            dns: state.dns.clone(),
            logs: projected_logs(&state),
            log_recording_error: log_recording_error(&state),
            connections: state.connections.clone(),
            request_history: state.request_history.iter().cloned().rev().collect(),
        })
    }

    pub fn runtime_status_projection(&self) -> Result<RuntimeStatusProjection, PawsError> {
        let state = self.lock_state()?;
        let native_vpn_running = self.vpn.is_running();
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
            vpn_running: native_vpn_running || state.platform_vpn_running,
            vpn_session_id: platform_vpn_session_id(&state),
            network_protected: state.platform_network_protected,
            network_protect_error: state.platform_network_protect_error.clone(),
            controller_running: state.api_controller.is_some(),
            controller_addr: state
                .api_controller
                .as_ref()
                .map(|controller| controller.bind_addr.to_string()),
            controller_diagnostics: state.controller_diagnostics.clone(),
            exit_location: state.exit_location.clone(),
            proxy_groups: state.proxy_groups.clone(),
            geodata: state.geodata.clone(),
            about: about_snapshot(),
        })
    }

    pub fn resource_projection(&self) -> Result<ResourceProjection, PawsError> {
        let state = self.lock_state()?;
        Ok(ResourceProjection {
            revisions: runtime_revisions(&state),
            proxy_groups: state.proxy_groups.clone(),
            providers: state.providers.clone(),
            geodata: state.geodata.clone(),
        })
    }

    fn ensure_runtime_services(self: &Arc<Self>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .runtime_services_task_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let weak = Arc::downgrade(self);
        // Construct the reset guard before spawning. If the runtime accepts and
        // then drops this future without polling it, dropping the captured guard
        // still makes a later `shared()` call able to restart the services.
        let task_guard = RuntimeServicesTaskGuard { core: weak.clone() };
        runtime.spawn(async move {
            let _task_guard = task_guard;
            let telemetry_loop = {
                let weak = weak.clone();
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(1));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        let Some(core) = weak.upgrade() else {
                            break;
                        };
                        if let Err(error) = core.refresh_telemetry() {
                            tracing::warn!(
                                target: "paws_core::telemetry",
                                "runtime telemetry refresh failed: {error}"
                            );
                        }
                    }
                }
            };
            let status_loop = {
                let weak = weak.clone();
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(1));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        let Some(core) = weak.upgrade() else {
                            break;
                        };
                        if let Err(error) = core.refresh_exit_location_if_due().await {
                            tracing::warn!(
                                target: "paws_core::status",
                                "exit-location refresh failed: {error}"
                            );
                        }
                    }
                }
            };
            let controller_sync_loop = async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let Some(core) = weak.upgrade() else {
                        break;
                    };
                    if let Err(error) = core.sync_external_controller_config().await {
                        tracing::warn!(
                            target: "paws_core::config",
                            "external-controller synchronization failed: {error}"
                        );
                    }
                }
            };
            tokio::join!(telemetry_loop, status_loop, controller_sync_loop);
        });
    }

    fn publish_runtime_change_locked(
        &self,
        state: &mut CoreState,
        config_changed: bool,
        telemetry_changed: bool,
    ) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        if config_changed {
            state.config_revision = state.config_revision.saturating_add(1);
        }
        if telemetry_changed {
            state.telemetry_revision = state.telemetry_revision.saturating_add(1);
        }
        if !config_changed && !telemetry_changed {
            state.status_revision = state.status_revision.saturating_add(1);
        }
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn publish_resource_change_locked(&self, state: &mut CoreState) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        state.resource_revision = state.resource_revision.saturating_add(1);
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn publish_config_and_resource_change_locked(&self, state: &mut CoreState) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        state.config_revision = state.config_revision.saturating_add(1);
        state.status_revision = state.status_revision.saturating_add(1);
        state.resource_revision = state.resource_revision.saturating_add(1);
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn publish_telemetry_and_resource_change_locked(
        &self,
        state: &mut CoreState,
    ) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        state.telemetry_revision = state.telemetry_revision.saturating_add(1);
        state.resource_revision = state.resource_revision.saturating_add(1);
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn publish_status_and_resource_change_locked(&self, state: &mut CoreState) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        state.status_revision = state.status_revision.saturating_add(1);
        state.resource_revision = state.resource_revision.saturating_add(1);
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn publish_status_and_telemetry_change_locked(
        &self,
        state: &mut CoreState,
    ) -> RuntimeRevisions {
        state.revision = state.revision.saturating_add(1);
        state.status_revision = state.status_revision.saturating_add(1);
        state.telemetry_revision = state.telemetry_revision.saturating_add(1);
        state.observed_at_unix_nanos = now_unix_nanos();
        let revisions = runtime_revisions(state);
        self.runtime_revision_tx.send_replace(revisions);
        revisions
    }

    fn ensure_config_revision_locked(
        state: &CoreState,
        expected_revision: u64,
    ) -> Result<(), PawsError> {
        if state.config_revision != expected_revision {
            return Err(PawsError::StaleConfigRevision {
                expected: expected_revision,
                current: state.config_revision,
            });
        }
        Ok(())
    }

    fn begin_resource_operation_locked(
        state: &mut CoreState,
        operation_key: &str,
        expected_resource_revision: Option<u64>,
    ) -> Result<(u64, u64), PawsError> {
        if let Some(expected) = expected_resource_revision {
            if state.resource_revision != expected {
                return Err(PawsError::StaleResourceRevision {
                    expected,
                    current: state.resource_revision,
                });
            }
        }
        let sequence = state
            .resource_operation_sequences
            .entry(operation_key.to_owned())
            .or_default();
        *sequence = sequence.saturating_add(1);
        Ok((*sequence, state.config_revision))
    }

    fn ensure_resource_operation_current_locked(
        state: &CoreState,
        operation_key: &str,
        operation_sequence: u64,
        config_revision: u64,
    ) -> Result<(), PawsError> {
        Self::ensure_config_revision_locked(state, config_revision)?;
        let current = state
            .resource_operation_sequences
            .get(operation_key)
            .copied()
            .unwrap_or_default();
        if current != operation_sequence {
            return Err(PawsError::Core(format!(
                "stale resource operation for {operation_key}: expected sequence {operation_sequence}, current {current}"
            )));
        }
        Ok(())
    }

    fn try_config_transaction(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, PawsError> {
        self.config_reload_lock.try_lock().map_err(|_| {
            PawsError::Core(
                "another configuration transaction is in progress; retry this action".to_owned(),
            )
        })
    }

    pub fn initialize_platform_shared_memory(
        self: &Arc<Self>,
    ) -> Result<PlatformSharedMemoryFds, PawsError> {
        {
            let platform = self
                .platform_ipc
                .lock()
                .map_err(|_| PawsError::Core("platform IPC lock poisoned".to_owned()))?;
            if let Some(platform) = platform.as_ref() {
                let fds = platform.ui_fds().map_err(platform_ipc_error)?;
                return Ok(PlatformSharedMemoryFds {
                    ashmem_fd: fds.ashmem,
                    notification_fd: fds.notification,
                });
            }
        }

        let log_root = {
            let state = self.lock_state()?;
            state.profiles.root().to_path_buf()
        };
        log_recording::reset_recording(&log_root)?;
        if let Ok(mut logs) = RUNTIME_LOGS.lock() {
            logs.clear();
        }
        let (platform, fds) = platform_ipc::PlatformIpc::create_ui().map_err(platform_ipc_error)?;
        {
            let mut slot = self
                .platform_ipc
                .lock()
                .map_err(|_| PawsError::Core("platform IPC lock poisoned".to_owned()))?;
            *slot = Some(Arc::clone(&platform));
        }
        self.start_platform_vpn_event_pump(platform)?;
        let mut state = self.lock_state()?;
        state.runtime_ui_cache_writes_enabled = true;
        self.persist_platform_vpn_state_locked(&mut state)?;
        let mode = state.mode;
        let global_proxy = current_global_proxy(&state);
        self.persist_platform_vpn_control_locked(&mut state, mode, global_proxy)?;
        Ok(PlatformSharedMemoryFds {
            ashmem_fd: fds.ashmem,
            notification_fd: fds.notification,
        })
    }

    pub fn attach_platform_shared_memory(
        &self,
        ashmem_fd: i32,
        notification_fd: i32,
    ) -> Result<(), PawsError> {
        let platform = platform_ipc::PlatformIpc::attach_vpn_raw(ashmem_fd, notification_fd)
            .map_err(platform_ipc_error)?;
        let previous = {
            let mut slot = self
                .platform_ipc
                .lock()
                .map_err(|_| PawsError::Core("platform IPC lock poisoned".to_owned()))?;
            slot.replace(platform)
        };
        // A VPN Extension process can outlive and be reused by the UI
        // process. Always replace the old ashmem session with the descriptors
        // from the latest Want so state is published back to the current UI.
        if let Some(previous) = previous {
            previous.cancel_event_waits();
        }
        // Wake any waiter parked on the replaced session so the subscription
        // loop re-enters the wait against the latest descriptors.
        let mut state = self.lock_state()?;
        state.runtime_ui_cache_writes_enabled = false;
        state.platform_remote_state_updated_at = 0;
        state.platform_remote_state_seen_at = None;
        state.platform_remote_stale_since = None;
        state.platform_watchdog_cleanup_recoverable = false;
        // Ownership is deliberately not adopted while attaching descriptors.
        // A reused Extension may still be cleaning up the previous session;
        // only bind_platform_vpn_start may transfer ownership from a verified
        // Want to this process.
        Ok(())
    }

    /// Validate a delivered Want against the UI lane it carries without
    /// replacing this process's current IPC binding or session owner.
    pub fn validate_platform_vpn_start_request(
        &self,
        ashmem_fd: i32,
        notification_fd: i32,
        attempt_id: &str,
    ) -> Result<(), PawsError> {
        self.validate_platform_owner_journal_for_want(attempt_id)?;
        let platform = platform_ipc::PlatformIpc::attach_vpn_raw(ashmem_fd, notification_fd)
            .map_err(platform_ipc_error)?;
        let envelope = platform
            .read_remote()
            .map_err(platform_ipc_error)?
            .ok_or_else(|| PawsError::Core("platform VPN start has no UI state".to_owned()))?;
        validate_platform_start_envelope(&envelope, attempt_id)
    }

    fn validate_platform_owner_journal_for_want(&self, attempt_id: &str) -> Result<(), PawsError> {
        let (journal_path, issuer_lease_path) = {
            let state = self.lock_state()?;
            (
                platform_owner_journal_path(&state),
                platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Issuer),
            )
        };
        let journal = match platform_owner::read(&journal_path)? {
            JournalRead::Missing => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal is missing for delivered attempt {attempt_id}"
                )))
            }
            JournalRead::Present(journal) if journal.attempt_id == attempt_id => journal,
            JournalRead::Present(journal) => {
                return Err(PawsError::Core(format!(
                    "stale platform VPN start attempt {attempt_id}; owner journal belongs to {}",
                    journal.attempt_id
                )))
            }
        };
        match journal.phase {
            PlatformVpnOwnerPhase::Pending => {
                let expected = platform_owner_lease_record(
                    attempt_id,
                    journal.issuer,
                    PlatformVpnOwnerLeaseRole::Issuer,
                );
                match platform_owner::observe_owner_lease_exact(&issuer_lease_path, &expected)? {
                    PlatformVpnOwnerLeaseObservation::HeldExact => {}
                    PlatformVpnOwnerLeaseObservation::Released => {
                        let message = format!(
                            "platform VPN start issuer lease for delivered attempt {attempt_id} was released"
                        );
                        return Err(PawsError::Core(message));
                    }
                    PlatformVpnOwnerLeaseObservation::HeldOther => {
                        return Err(PawsError::Core(format!(
                            "cannot verify the exact platform VPN start issuer lease for delivered attempt {attempt_id}"
                        )))
                    }
                }
            }
            PlatformVpnOwnerPhase::Attached => {}
            PlatformVpnOwnerPhase::Stopping => {
                return Err(PawsError::Core(format!(
                    "platform VPN start attempt {attempt_id} was fenced by a stop intent"
                )))
            }
        }
        Ok(())
    }

    /// Publish proof that HarmonyOS delivered the exact terminal attempt to
    /// an Extension process without adopting its IPC binding or reviving the
    /// session. This closes the late-start barrier when the platform's start
    /// Promise remains pending even after actual Want delivery.
    pub fn acknowledge_terminal_platform_vpn_start_delivery(
        &self,
        ashmem_fd: i32,
        notification_fd: i32,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        if attempt_id.is_empty() {
            return Ok(false);
        }
        let platform = platform_ipc::PlatformIpc::attach_vpn_raw(ashmem_fd, notification_fd)
            .map_err(platform_ipc_error)?;
        let envelope = platform
            .read_remote()
            .map_err(platform_ipc_error)?
            .ok_or_else(|| PawsError::Core("platform VPN start has no UI state".to_owned()))?;
        let Some(mut state) = envelope.state else {
            return Ok(false);
        };
        if !acknowledge_terminal_delivery_state(&mut state, attempt_id) {
            return Ok(false);
        }
        platform.publish_state(state).map_err(platform_ipc_error)?;
        Ok(true)
    }

    pub fn sync_platform_changes(&self) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(())
    }

    /// Block until the peer process publishes a platform frame.
    ///
    /// Fully event driven: the wait parks on the session notification socket
    /// together with a process-local cancellation socket. It resolves when a
    /// frame arrives (`Ok(true)`) or when [`Self::cancel_platform_change_wait`]
    /// is invoked (`Ok(false)`); it never polls.
    pub async fn wait_for_platform_change_event(&self) -> Result<bool, PawsError> {
        let Some(platform) = self.platform_ipc()? else {
            return Ok(false);
        };
        tokio::task::spawn_blocking(move || platform.wait_for_change_event_cancellable())
            .await
            .map_err(|error| {
                PawsError::Core(format!("platform subscription task failed: {error}"))
            })?
            .map_err(platform_ipc_error)
    }

    /// Wake the in-process waiter parked in [`Self::wait_for_platform_change_event`].
    pub fn cancel_platform_change_wait(&self) {
        if let Ok(Some(platform)) = self.platform_ipc() {
            platform.cancel_event_waits();
        }
    }

    /// Current in-process VPN state event revision.
    ///
    /// UI consumers keep this revision and await the next one. The revision
    /// is independent from the cross-process shared-memory generation so
    /// local transitions such as `starting` are delivered through the same
    /// event stream as remote Extension transitions.
    pub fn platform_vpn_event_revision(&self) -> u64 {
        self.platform_vpn_event_sequence.load(Ordering::Acquire)
    }

    /// Await the first VPN state event newer than `after_revision`.
    pub async fn await_platform_vpn_event(&self, after_revision: u64) -> Result<u64, PawsError> {
        let mut receiver = self.platform_vpn_event_tx.subscribe();
        loop {
            let revision = *receiver.borrow_and_update();
            if revision > after_revision {
                return Ok(revision);
            }
            receiver
                .changed()
                .await
                .map_err(|_| PawsError::Core("platform VPN event stream closed".to_owned()))?;
        }
    }

    pub fn is_platform_vpn_session_current(&self, session_id: &str) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(!session_id.is_empty()
            && state.platform_start_attempt_id == session_id
            && state.platform_vpn_running
            && state.platform_start_outcome == PlatformStartOutcome::Connected)
    }

    pub fn current_platform_vpn_session_id(&self) -> Result<String, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(platform_vpn_session_id(&state).unwrap_or_default())
    }

    pub fn is_platform_vpn_session_current_at_revision(
        &self,
        session_id: &str,
        expected_config_revision: u64,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(!session_id.is_empty()
            && state.platform_start_attempt_id == session_id
            && state.platform_vpn_running
            && state.platform_start_outcome == PlatformStartOutcome::Connected
            && state.config_revision == expected_config_revision)
    }

    pub fn is_runtime_config_revision_current(
        &self,
        expected_config_revision: u64,
    ) -> Result<bool, PawsError> {
        Ok(self.lock_state()?.config_revision == expected_config_revision)
    }

    fn start_platform_vpn_event_pump(
        self: &Arc<Self>,
        platform: Arc<PlatformIpc>,
    ) -> Result<(), PawsError> {
        let core = Arc::clone(self);
        std::thread::Builder::new()
            .name("paws-platform-vpn-events".to_owned())
            .spawn(move || loop {
                if let Err(error) = platform.wait_for_change_event() {
                    core.record_platform_vpn_event_pump_failure(error.to_string());
                    break;
                }
                if let Err(error) = core.sync_platform_changes() {
                    core.record_platform_vpn_event_pump_failure(error.to_string());
                    break;
                }
            })
            .map(|_| ())
            .map_err(|error| {
                PawsError::Core(format!("start platform VPN event pump failed: {error}"))
            })
    }

    fn record_platform_vpn_event_pump_failure(&self, error: String) {
        let Ok(mut state) = self.lock_state() else {
            return;
        };
        let message = format!("platform VPN event pump failed: {error}");
        if state.platform_start_outcome == PlatformStartOutcome::Pending
            || state.platform_vpn_running
        {
            apply_platform_failure(&mut state, message.clone());
        }
        state.logs.push(warning_log(message));
        let _ = self.persist_platform_vpn_state_locked(&mut state);
    }

    fn platform_ipc(&self) -> Result<Option<Arc<PlatformIpc>>, PawsError> {
        self.platform_ipc
            .lock()
            .map(|platform| platform.clone())
            .map_err(|_| PawsError::Core("platform IPC lock poisoned".to_owned()))
    }

    pub fn external_http_route(
        &self,
    ) -> Result<(Option<String>, http_client::ExternalHttpRoute), PawsError> {
        let state = self.lock_state()?;
        if self.vpn.is_running() || state.platform_vpn_running {
            Ok((
                Some(format!(
                    "http://127.0.0.1:{}",
                    state.network_ports.mixed_port
                )),
                http_client::ExternalHttpRoute::CurrentProxy,
            ))
        } else {
            Ok((None, http_client::ExternalHttpRoute::Direct))
        }
    }

    async fn download_subscription(
        &self,
        url: &str,
        context: &str,
    ) -> Result<http_client::ExternalTextResponse, PawsError> {
        let (proxy_url, route) = self.external_http_route()?;
        let client = http_client::shared_external_http_client()
            .map_err(|error| PawsError::Core(error.to_string()))?;
        let request = client
            .request(
                reqwest::Method::GET,
                url,
                proxy_url.as_deref(),
                http_client::ExternalRequestKind::Subscription,
            )
            .map_err(|error| PawsError::Core(error.to_string()))?;
        let response = request.send().await.map_err(|error| {
            PawsError::Core(format!(
                "{context} request via {route} failed: {}",
                error.without_url()
            ))
        })?;
        http_client::read_external_text_response(
            response,
            context,
            route,
            http_client::EXTERNAL_HTTP_MAX_BODY_BYTES,
        )
        .await
        .map_err(|error| PawsError::Core(error.to_string()))
    }

    pub async fn prepare_profile_import_from_url(
        &self,
        url: &str,
        name: Option<String>,
    ) -> Result<PreparedProfileImport, PawsError> {
        let response = self.download_subscription(url, "profile download").await?;
        let subscription_user_info = subscription_userinfo_from_headers(&response.headers);
        let subscription_metadata = subscription_metadata_from_headers(&response.headers);
        let header_name = subscription_profile_name_from_headers(&response.headers).or_else(|| {
            subscription_metadata
                .as_ref()
                .and_then(|metadata| metadata.title.clone())
        });
        let source_yaml = response.body;
        let subscription_user_info = subscription_user_info
            .or_else(|| paws_profile::parse_subscription_userinfo_comment(&source_yaml));
        let subscription_metadata = paws_profile::merge_subscription_metadata(
            subscription_metadata,
            paws_profile::parse_subscription_metadata_comment(&source_yaml),
        );
        let name = name
            .or(header_name)
            .unwrap_or_else(|| profile_name_from_url(url));
        let raw_yaml = normalize_profile_content(&source_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        Ok(PreparedProfileImport {
            name,
            source: url.to_owned(),
            raw_yaml,
            subscription_url: Some(url.to_owned()),
            subscription_user_info,
            subscription_metadata,
        })
    }

    pub async fn prepare_profile_import_from_content(
        &self,
        name: &str,
        source: &str,
        raw_yaml: &str,
        subscription_url: Option<String>,
    ) -> Result<PreparedProfileImport, PawsError> {
        let subscription_user_info = paws_profile::parse_subscription_userinfo_comment(raw_yaml);
        let subscription_metadata = paws_profile::parse_subscription_metadata_comment(raw_yaml);
        let raw_yaml = normalize_profile_content(raw_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        Ok(PreparedProfileImport {
            name: name.to_owned(),
            source: source.to_owned(),
            raw_yaml,
            subscription_url,
            subscription_user_info,
            subscription_metadata,
        })
    }

    pub async fn prepare_profile_refresh_from_url(
        &self,
        url: &str,
    ) -> Result<PreparedProfileRefresh, PawsError> {
        let response = self.download_subscription(url, "profile refresh").await?;
        let subscription_user_info = subscription_userinfo_from_headers(&response.headers);
        let subscription_metadata = subscription_metadata_from_headers(&response.headers);
        let source_yaml = response.body;
        let subscription_user_info = subscription_user_info
            .or_else(|| paws_profile::parse_subscription_userinfo_comment(&source_yaml));
        let subscription_metadata = paws_profile::merge_subscription_metadata(
            subscription_metadata,
            paws_profile::parse_subscription_metadata_comment(&source_yaml),
        );
        let raw_yaml = normalize_profile_content(&source_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        Ok(PreparedProfileRefresh {
            raw_yaml,
            subscription_user_info,
            subscription_metadata,
        })
    }

    pub async fn import_profile_from_url(
        &self,
        url: &str,
        name: Option<String>,
    ) -> Result<String, PawsError> {
        let prepared = self.prepare_profile_import_from_url(url, name).await?;
        let _reload_guard = self.config_reload_lock.lock().await;
        let mut state = self.lock_state()?;
        let id = state
            .profiles
            .import_profile_content_with_subscription_metadata(
                prepared.name.clone(),
                prepared.source,
                prepared.raw_yaml,
                prepared.subscription_url,
                prepared.subscription_user_info,
                prepared.subscription_metadata,
            )?;
        state
            .logs
            .push(info_log(format!("profile imported: {}", prepared.name)));
        self.publish_runtime_change_locked(&mut state, true, false);
        Ok(id)
    }

    pub async fn import_profile_from_url_and_activate_checked(
        &self,
        url: &str,
        name: Option<String>,
        expected_config_revision: u64,
    ) -> Result<ProfileImportReceipt, PawsError> {
        let prepared = self.prepare_profile_import_from_url(url, name).await?;
        self.commit_prepared_profile_import_and_activate_checked(prepared, expected_config_revision)
            .await
    }

    pub async fn import_profile_from_content(
        &self,
        name: &str,
        source: &str,
        raw_yaml: &str,
        subscription_url: Option<String>,
    ) -> Result<String, PawsError> {
        let prepared = self
            .prepare_profile_import_from_content(name, source, raw_yaml, subscription_url)
            .await?;
        let _reload_guard = self.config_reload_lock.lock().await;
        let mut state = self.lock_state()?;
        let id = state
            .profiles
            .import_profile_content_with_subscription_metadata(
                prepared.name.clone(),
                prepared.source,
                prepared.raw_yaml,
                prepared.subscription_url,
                prepared.subscription_user_info,
                prepared.subscription_metadata,
            )?;
        state
            .logs
            .push(info_log(format!("profile imported: {}", prepared.name)));
        self.publish_runtime_change_locked(&mut state, true, false);
        Ok(id)
    }

    pub async fn import_profile_from_content_and_activate_checked(
        &self,
        name: &str,
        source: &str,
        raw_yaml: &str,
        subscription_url: Option<String>,
        expected_config_revision: u64,
    ) -> Result<ProfileImportReceipt, PawsError> {
        let prepared = self
            .prepare_profile_import_from_content(name, source, raw_yaml, subscription_url)
            .await?;
        self.commit_prepared_profile_import_and_activate_checked(prepared, expected_config_revision)
            .await
    }

    pub async fn commit_prepared_profile_import_and_activate_checked(
        &self,
        prepared: PreparedProfileImport,
        expected_config_revision: u64,
    ) -> Result<ProfileImportReceipt, PawsError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (profile_id, profile_name, previous_profiles, previous_active, previous_engine_loaded) = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            let previous_profiles = state.profiles.clone();
            let previous_active = state.profiles.active_profile().map(ToOwned::to_owned);
            let previous_engine_loaded = state.engine_loaded;
            let profile_name = prepared.name.clone();
            let profile_id = state
                .profiles
                .import_profile_content_with_subscription_metadata(
                    prepared.name,
                    prepared.source,
                    prepared.raw_yaml,
                    prepared.subscription_url,
                    prepared.subscription_user_info,
                    prepared.subscription_metadata,
                )?;
            (
                profile_id,
                profile_name,
                previous_profiles,
                previous_active,
                previous_engine_loaded,
            )
        };

        if let Err(primary) = self.reload_config_inner(&profile_id).await {
            let (error, rollback_succeeded) = self
                .rollback_profile_import_after_failure(
                    &profile_id,
                    previous_profiles,
                    previous_active.as_deref(),
                    previous_engine_loaded,
                    primary,
                )
                .await;
            if !rollback_succeeded {
                let mut state = self.lock_state()?;
                self.publish_config_and_resource_change_locked(&mut state);
            }
            return Err(error);
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "profile imported and activated: {profile_name}"
        )));
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(ProfileImportReceipt {
            profile_id,
            config: Self::config_projection_locked(&state),
        })
    }

    pub async fn refresh_profile(&self, profile_id: &str) -> Result<(), PawsError> {
        let (subscription_url, expected_config_revision) = {
            let state = self.lock_state()?;
            (
                state.profiles.profile(profile_id)?.subscription_url.clone(),
                state.config_revision,
            )
        };
        let Some(url) = subscription_url else {
            return Err(PawsError::Core(format!(
                "profile {profile_id} has no subscription URL"
            )));
        };
        let result = self
            .refresh_profile_from_url(profile_id, &url, expected_config_revision)
            .await;
        if let Err(error) = &result {
            if !matches!(error, PawsError::StaleConfigRevision { .. }) {
                let _reload_guard = self.config_reload_lock.lock().await;
                if let Ok(mut state) = self.lock_state() {
                    if state.config_revision == expected_config_revision
                        && state
                            .profiles
                            .mark_profile_refresh_failed(profile_id, error.to_string())
                            .is_ok()
                    {
                        self.publish_runtime_change_locked(&mut state, true, false);
                    }
                }
            }
        }
        result
    }

    pub async fn refresh_profile_and_activate_checked(
        &self,
        profile_id: &str,
        name: Option<String>,
        subscription_url: Option<String>,
        expected_config_revision: u64,
    ) -> Result<ConfigProjection, PawsError> {
        let url = {
            let state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            let profile = state.profiles.profile(profile_id)?;
            subscription_url
                .clone()
                .or_else(|| profile.subscription_url.clone())
                .ok_or_else(|| {
                    PawsError::Core(format!("profile {profile_id} has no subscription URL"))
                })?
        };
        let prepared = self.prepare_profile_refresh_from_url(&url).await?;
        self.commit_prepared_profile_refresh_and_activate_checked(
            profile_id,
            name,
            subscription_url,
            prepared,
            expected_config_revision,
        )
        .await
    }

    pub async fn commit_prepared_profile_refresh_and_activate_checked(
        &self,
        profile_id: &str,
        name: Option<String>,
        subscription_url: Option<String>,
        prepared: PreparedProfileRefresh,
        expected_config_revision: u64,
    ) -> Result<ConfigProjection, PawsError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (checkpoint, previous_active, previous_engine_loaded) = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            let profile = state.profiles.profile(profile_id)?;
            let subscription_identity = match (name, subscription_url) {
                (Some(name), Some(url)) => Some((name, url)),
                (Some(name), None) => profile.subscription_url.clone().map(|url| (name, url)),
                (None, Some(url)) => Some((profile.name.clone(), url)),
                (None, None) => None,
            };
            let checkpoint = state.profiles.checkpoint_profile(profile_id)?;
            let previous_active = state.profiles.active_profile().map(ToOwned::to_owned);
            let previous_engine_loaded = state.engine_loaded;
            state
                .profiles
                .replace_profile_content_and_subscription_metadata(
                    profile_id,
                    prepared.raw_yaml,
                    prepared.subscription_user_info,
                    prepared.subscription_metadata,
                    subscription_identity,
                )?;
            (checkpoint, previous_active, previous_engine_loaded)
        };

        if let Err(primary) = self.reload_config_inner(profile_id).await {
            let (error, rollback_succeeded) = self
                .rollback_profile_activation_after_failure(
                    checkpoint,
                    previous_active.as_deref(),
                    previous_engine_loaded,
                    primary,
                )
                .await;
            if !rollback_succeeded {
                let mut state = self.lock_state()?;
                self.publish_config_and_resource_change_locked(&mut state);
            }
            return Err(error);
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "profile refreshed and activated: {profile_id}"
        )));
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(Self::config_projection_locked(&state))
    }

    async fn refresh_profile_from_url(
        &self,
        profile_id: &str,
        url: &str,
        expected_config_revision: u64,
    ) -> Result<(), PawsError> {
        let prepared = self.prepare_profile_refresh_from_url(url).await?;
        let _reload_guard = self.config_reload_lock.lock().await;
        let (checkpoint, active) = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            let checkpoint = state.profiles.checkpoint_profile(profile_id)?;
            state
                .profiles
                .replace_profile_content_with_subscription_metadata(
                    profile_id,
                    prepared.raw_yaml,
                    prepared.subscription_user_info,
                    prepared.subscription_metadata,
                )?;
            (
                checkpoint,
                state.profiles.active_profile() == Some(profile_id),
            )
        };
        if active {
            if let Err(primary) = self.reload_config_inner(profile_id).await {
                let (error, rollback_succeeded) = self
                    .rollback_profile_after_failure(profile_id, checkpoint, primary)
                    .await;
                if !rollback_succeeded {
                    let mut state = self.lock_state()?;
                    self.publish_config_and_resource_change_locked(&mut state);
                }
                return Err(error);
            }
        }
        let mut state = self.lock_state()?;
        state
            .logs
            .push(info_log(format!("profile refreshed: {profile_id}")));
        if active {
            self.publish_config_and_resource_change_locked(&mut state);
        } else {
            self.publish_runtime_change_locked(&mut state, true, false);
        }
        Ok(())
    }

    pub async fn refresh_all_profiles(&self) -> Result<(), PawsError> {
        let profiles: Vec<ProfileSummary> = {
            let state = self.lock_state()?;
            state
                .profiles
                .summaries()
                .into_iter()
                .filter(|profile| profile.subscription_url.is_some())
                .collect()
        };
        if profiles.is_empty() {
            let mut state = self.lock_state()?;
            state
                .logs
                .push(info_log("profile refresh skipped: no subscriptions"));
            return Ok(());
        }

        let total = profiles.len();
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut last_error = None;
        for profile in profiles {
            match self.refresh_profile(&profile.id).await {
                Ok(()) => succeeded += 1,
                Err(error) => {
                    failed += 1;
                    last_error = Some(error.to_string());
                    let mut state = self.lock_state()?;
                    state.logs.push(warning_log(format!(
                        "profile refresh failed: {} ({})",
                        profile.name, error
                    )));
                }
            }
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "profile refresh all finished: {succeeded} succeeded, {failed} failed"
        )));
        if succeeded == 0 {
            return Err(PawsError::Core(format!(
                "all {total} subscription refreshes failed: {}",
                last_error.unwrap_or_else(|| "unknown error".to_owned())
            )));
        }
        Ok(())
    }

    pub async fn refresh_due_profiles(&self) -> Result<(), PawsError> {
        let profiles = {
            let state = self.lock_state()?;
            state.profiles.due_subscription_summaries()
        };
        if profiles.is_empty() {
            let mut state = self.lock_state()?;
            state.logs.push(info_log(
                "profile due refresh skipped: no due subscriptions",
            ));
            return Ok(());
        }

        let total = profiles.len();
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut last_error = None;
        for profile in profiles {
            match self.refresh_profile(&profile.id).await {
                Ok(()) => succeeded += 1,
                Err(error) => {
                    failed += 1;
                    last_error = Some(error.to_string());
                    let mut state = self.lock_state()?;
                    state.logs.push(warning_log(format!(
                        "profile due refresh failed: {} ({})",
                        profile.name, error
                    )));
                }
            }
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "profile due refresh finished: {succeeded} succeeded, {failed} failed"
        )));
        if failed == total {
            return Err(PawsError::Core(format!(
                "all {total} due subscription refreshes failed: {}",
                last_error.unwrap_or_else(|| "unknown error".to_owned())
            )));
        }
        Ok(())
    }

    pub async fn activate_profile(&self, profile_id: &str) -> Result<(), PawsError> {
        self.reload_config(profile_id).await
    }

    pub async fn activate_profile_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
    ) -> Result<ConfigProjection, PawsError> {
        self.reload_config_at_revision(profile_id, expected_config_revision)
            .await
    }

    pub async fn delete_profile(&self, profile_id: &str) -> Result<(), PawsError> {
        let expected_config_revision = {
            let state = self.lock_state()?;
            state.profiles.profile(profile_id)?;
            state.config_revision
        };
        let _reload_guard = self.config_reload_lock.lock().await;
        let tun_stats = self.vpn.stats();
        let (was_active, next_active, previous_engine_loaded) = {
            let state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            state.profiles.profile(profile_id)?;
            let was_active = state.profiles.active_profile() == Some(profile_id);
            let next_active = was_active.then(|| {
                state
                    .profiles
                    .summaries()
                    .into_iter()
                    .find(|profile| profile.id != profile_id)
                    .map(|profile| profile.id)
            });
            (was_active, next_active.flatten(), state.engine_loaded)
        };

        if let Some(next_profile_id) = next_active.as_deref() {
            if let Err(primary) = self.reload_config_inner(next_profile_id).await {
                return if previous_engine_loaded {
                    match self.reload_config_inner(profile_id).await {
                        Ok(()) => Err(PawsError::Core(format!(
                            "profile deletion was cancelled because the replacement profile could not be activated; the previous runtime was restored: {primary}"
                        ))),
                        Err(rollback) => Err(PawsError::Core(format!(
                            "profile deletion was cancelled because the replacement profile could not be activated: {primary}; restoring the previous runtime also failed: {rollback}"
                        ))),
                    }
                } else {
                    Err(primary)
                };
            }
        }

        let deletion = {
            let mut state = self.lock_state()?;
            if was_active && next_active.is_none() {
                settle_traffic_before_profile_switch(&mut state, tun_stats.as_ref())?;
            }
            state.profiles.delete_profile(profile_id)
        };
        if let Err(primary) = deletion {
            if was_active && next_active.is_some() && previous_engine_loaded {
                return match self.reload_config_inner(profile_id).await {
                    Ok(()) => Err(PawsError::Core(format!(
                        "profile deletion failed and the previous runtime was restored: {primary}"
                    ))),
                    Err(rollback) => Err(PawsError::Core(format!(
                        "profile deletion failed: {primary}; restoring the previous runtime also failed: {rollback}"
                    ))),
                };
            }
            return Err(primary);
        }

        let previous_controller = {
            let mut state = self.lock_state()?;
            if was_active && next_active.is_none() {
                state.tunnel = None;
                state.proxy_groups.clear();
                state.providers.clear();
                state.runtime_rules.clear();
                state.engine_loaded = false;
            }
            let previous_controller = (was_active && next_active.is_none())
                .then(|| state.api_controller.take())
                .flatten();
            state.controller_diagnostics = sample_controller_diagnostics(&state);
            state
                .logs
                .push(info_log(format!("profile deleted: {profile_id}")));
            if was_active {
                self.publish_config_and_resource_change_locked(&mut state);
            } else {
                self.publish_runtime_change_locked(&mut state, true, false);
            }
            previous_controller
        };
        if let Some(mut controller) = previous_controller {
            controller.shutdown().await;
        }
        Ok(())
    }

    pub async fn import_rules_from_content_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        source: &str,
        rules_text: &str,
    ) -> Result<RuleImportReceipt, PawsError> {
        let prepared = Self::prepare_rule_import(source, rules_text)?;
        self.commit_prepared_rule_import_checked(profile_id, expected_config_revision, prepared)
            .await
    }

    pub fn prepare_rule_import(
        source: &str,
        rules_text: &str,
    ) -> Result<PreparedRuleImport, PawsError> {
        paws_profile::parse_imported_rule_lines(rules_text)?;
        Ok(PreparedRuleImport {
            source: source.to_owned(),
            rules_text: rules_text.to_owned(),
        })
    }

    pub async fn commit_prepared_rule_import_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        prepared: PreparedRuleImport,
    ) -> Result<RuleImportReceipt, PawsError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (previous_profiles, imported_rule_ids, previous_engine_loaded) = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            if state.profiles.active_profile() != Some(profile_id) {
                return Err(PawsError::Core(format!(
                    "active profile changed before importing rules for {profile_id}"
                )));
            }
            let previous_profiles = state.profiles.clone();
            let imported_rule_ids = state.profiles.import_rules_for_profile(
                profile_id,
                prepared.source,
                &prepared.rules_text,
            )?;
            (previous_profiles, imported_rule_ids, state.engine_loaded)
        };

        if let Err(primary) = self.reload_config_inner(profile_id).await {
            let (error, rollback_succeeded) = self
                .rollback_profile_store_after_failure(
                    profile_id,
                    previous_profiles,
                    previous_engine_loaded,
                    primary,
                )
                .await;
            if !rollback_succeeded {
                let mut state = self.lock_state()?;
                self.publish_config_and_resource_change_locked(&mut state);
            }
            return Err(error);
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "imported {} rules for {profile_id}",
            imported_rule_ids.len()
        )));
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(RuleImportReceipt {
            imported_rule_ids,
            config: Self::config_projection_locked(&state),
        })
    }

    pub async fn apply_manual_rule(
        &self,
        profile_id: &str,
        spec: &ManualRuleSpec,
    ) -> Result<ManualRuleApplyResult, PawsError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (
            candidate_profiles,
            old_runtime_yaml,
            runtime_yaml,
            mutation,
            mode,
            expected_config_revision,
        ) = {
            let state = self.lock_state()?;
            if state.profiles.active_profile() != Some(profile_id) {
                return Err(PawsError::Core(
                    "manual activity rules can only be added to the active profile".to_owned(),
                ));
            }
            let mut candidate_profiles = state.profiles.clone();
            let old_runtime_yaml =
                state
                    .profiles
                    .render_runtime_yaml(profile_id, state.mode, &state.vpn_options)?;
            let mutation = candidate_profiles.stage_manual_rule(profile_id, spec)?;
            let runtime_yaml = candidate_profiles.render_runtime_yaml(
                profile_id,
                state.mode,
                &state.vpn_options,
            )?;
            (
                candidate_profiles,
                old_runtime_yaml,
                runtime_yaml,
                mutation,
                state.mode,
                state.config_revision,
            )
        };

        let config = load_meow_config(&runtime_yaml).await?;
        let target = mutation.line.split(',').nth(2).unwrap_or_default().trim();
        if !target.eq_ignore_ascii_case("DIRECT") {
            let is_group = config
                .proxies
                .get(target)
                .is_some_and(|proxy| proxy.members().is_some());
            if !is_group {
                return Err(PawsError::Core(format!(
                    "manual rule target is not an available proxy group: {target}"
                )));
            }
        }

        let raw_config = config.raw.clone();
        let loaded_rule_lines = raw_config.rules.clone().unwrap_or_default();
        let editable_rules = candidate_profiles.rules_for_profile(profile_id);
        let runtime_rules = runtime_rule_summaries(profile_id, &loaded_rule_lines, &editable_rules);

        let mut state = self.lock_state()?;
        Self::ensure_config_revision_locked(&state, expected_config_revision)?;
        if state.profiles.active_profile() != Some(profile_id) {
            return Err(PawsError::Core(
                "active profile changed while applying the manual rule".to_owned(),
            ));
        }
        candidate_profiles.write_runtime_yaml(profile_id, &runtime_yaml)?;
        if let Err(primary) = candidate_profiles.persist() {
            return match state
                .profiles
                .write_runtime_yaml(profile_id, &old_runtime_yaml)
            {
                Ok(()) => Err(PawsError::Core(format!(
                    "manual rule persistence failed and runtime YAML was rolled back: {primary}"
                ))),
                Err(rollback) => Err(PawsError::Core(format!(
                    "manual rule persistence failed: {primary}; runtime YAML rollback also failed: {rollback}"
                ))),
            };
        }

        let live_updated = if let Some(tunnel) = &state.tunnel {
            tunnel.update_rules(config.rules);
            true
        } else {
            false
        };
        if let Some(controller) = state.api_controller.as_mut() {
            *controller.raw_config.write() = raw_config.clone();
            controller.baseline_raw_config = raw_config;
            controller.synced_revision = controller.config_revision.load(Ordering::Acquire);
        }
        state.profiles = candidate_profiles;
        state.runtime_rules = runtime_rules;
        state.logs.push(info_log(format!(
            "manual activity rule applied: {} ({:?})",
            mutation.line, mutation.kind
        )));
        self.publish_runtime_change_locked(&mut state, true, false);

        Ok(ManualRuleApplyResult {
            mutation,
            live_updated,
            rule_mode_active: mode == RuntimeMode::Rule,
        })
    }

    pub async fn set_rule_enabled_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        rule_id: &str,
        enabled: bool,
    ) -> Result<ConfigProjection, PawsError> {
        self.mutate_profile_store_config(
            profile_id,
            expected_config_revision,
            format!(
                "rule {rule_id} {}",
                if enabled { "enabled" } else { "disabled" }
            ),
            move |profiles| profiles.set_rule_enabled(profile_id, rule_id, enabled),
        )
        .await
    }

    pub async fn reorder_rules_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        ordered_rule_ids: Vec<String>,
    ) -> Result<ConfigProjection, PawsError> {
        self.mutate_profile_store_config(
            profile_id,
            expected_config_revision,
            format!("rules reordered for {profile_id}"),
            move |profiles| profiles.reorder_rules(profile_id, &ordered_rule_ids),
        )
        .await
    }

    pub async fn delete_rule_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        rule_id: &str,
    ) -> Result<ConfigProjection, PawsError> {
        self.mutate_profile_store_config(
            profile_id,
            expected_config_revision,
            format!("rule deleted: {rule_id}"),
            move |profiles| {
                let rule = profiles
                    .rules_for_profile(profile_id)
                    .into_iter()
                    .find(|rule| rule.id == rule_id)
                    .ok_or_else(|| PawsError::RuleNotFound(rule_id.to_owned()))?;
                profiles.delete_rule(&rule.id)
            },
        )
        .await
    }

    pub fn clear_request_history(&self) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        state.request_history.clear();
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }

    pub fn clear_logs(&self) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        state.logs.clear();
        if let Ok(mut logs) = RUNTIME_LOGS.lock() {
            logs.clear();
        }
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }

    pub fn log_recording_status(&self) -> Result<LogRecordingStatus, PawsError> {
        let state = self.lock_state()?;
        let mut status = log_recording::recording_status(state.profiles.root())?;
        status.last_error = log_recording_error(&state);
        Ok(status)
    }

    pub fn set_log_recording_enabled(
        &self,
        enabled: bool,
    ) -> Result<LogRecordingStatus, PawsError> {
        let mut state = self.lock_state()?;
        let root = state.profiles.root().to_path_buf();
        let was_enabled = match log_recording::recording_status(&root) {
            Ok(status) => status.enabled,
            Err(error) => {
                state.logs.record_control_error(&error);
                self.publish_runtime_change_locked(&mut state, false, true);
                return Err(error);
            }
        };
        if was_enabled == enabled {
            let mut status = match log_recording::recording_status(&root) {
                Ok(status) => status,
                Err(error) => {
                    state.logs.record_control_error(&error);
                    self.publish_runtime_change_locked(&mut state, false, true);
                    return Err(error);
                }
            };
            state.logs.clear_control_error();
            status.last_error = log_recording_error(&state);
            return Ok(status);
        }

        let change_result = if enabled {
            state.logs.clear();
            if let Ok(mut logs) = RUNTIME_LOGS.lock() {
                logs.clear();
            }
            log_recording::set_recording_enabled(&root, true).map(|()| {
                state.logs.sync_session();
                state.logs.push(info_log("log recording enabled"));
            })
        } else {
            state.logs.push(info_log("log recording disabled"));
            log_recording::set_recording_enabled(&root, false).map(|()| {
                state.logs.sync_session();
                if let Ok(mut logs) = RUNTIME_LOGS.lock() {
                    logs.clear();
                }
            })
        };
        if let Err(error) = change_result {
            state.logs.record_control_error(&error);
            self.publish_runtime_change_locked(&mut state, false, true);
            return Err(error);
        }
        state.logs.clear_control_error();
        let mut status = match log_recording::recording_status(&root) {
            Ok(status) => status,
            Err(error) => {
                state.logs.record_control_error(&error);
                self.publish_runtime_change_locked(&mut state, false, true);
                return Err(error);
            }
        };
        status.last_error = log_recording_error(&state);
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(status)
    }

    pub fn read_log_archive(&self, file_name: &str) -> Result<String, PawsError> {
        let state = self.lock_state()?;
        log_recording::read_archive(state.profiles.root(), file_name)
    }

    pub fn delete_log_archive(&self, file_name: &str) -> Result<LogRecordingStatus, PawsError> {
        let state = self.lock_state()?;
        log_recording::delete_archive(state.profiles.root(), file_name)?;
        let mut status = log_recording::recording_status(state.profiles.root())?;
        status.last_error = log_recording_error(&state);
        Ok(status)
    }

    pub async fn start_vpn(&self, fd: i32, options_json: &str) -> Result<(), PawsError> {
        self.start_vpn_inner(fd, options_json, None)
            .await
            .map(|_| ())
    }

    pub async fn start_platform_vpn(
        self: &Arc<Self>,
        fd: i32,
        options_json: &str,
        attempt_id: &str,
    ) -> Result<(), PawsError> {
        let generation = self
            .start_vpn_inner(fd, options_json, Some(attempt_id))
            .await?;
        let weak = Arc::downgrade(self);
        let vpn = self.vpn.clone();
        let attempt_id = attempt_id.to_owned();
        tokio::spawn(async move {
            let Ok(exit) = vpn.await_exit(generation).await else {
                return;
            };
            if let Some(core) = weak.upgrade() {
                core.publish_native_vpn_exit(&attempt_id, exit.generation, exit.error)
                    .await;
            }
        });
        Ok(())
    }

    async fn start_vpn_inner(
        &self,
        fd: i32,
        options_json: &str,
        attempt_id: Option<&str>,
    ) -> Result<u64, PawsError> {
        let _operation_guard = self.vpn_operation_lock.lock().await;
        if let Some(attempt_id) = attempt_id {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            ensure_platform_attempt_active(&state, attempt_id)?;
        }
        let options: VpnOptions = from_json(options_json)?;
        validate_supported_vpn_options(&options)?;
        self.prepare_active_vpn().await?;
        let (tunnel, sniffer_config, mixed_port) = {
            let mut state = self.lock_state()?;
            if let Some(attempt_id) = attempt_id {
                self.sync_platform_vpn_state_locked(&mut state);
                ensure_platform_attempt_active(&state, attempt_id)?;
            }
            (
                state.tunnel.clone(),
                state.sniffer_config.clone(),
                state.network_ports.mixed_port,
            )
        };
        let tunnel = tunnel
            .ok_or_else(|| PawsError::Core("activate a profile before starting VPN".to_owned()))?;
        let generation = self
            .vpn
            .start(fd, options.clone(), tunnel.clone(), sniffer_config)
            .await?;
        if let Some(attempt_id) = attempt_id {
            let attempt_result = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                ensure_platform_attempt_active(&state, attempt_id)
            };
            if let Err(error) = attempt_result {
                self.vpn.stop().await?;
                return Err(error);
            }
        }
        let mixed_listener = self.restart_mixed_listener(tunnel, mixed_port).await;
        if let Some(attempt_id) = attempt_id {
            let attempt_result = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                ensure_platform_attempt_active(&state, attempt_id)
            };
            if let Err(error) = attempt_result {
                self.vpn.stop().await?;
                self.stop_mixed_listener()?;
                return Err(error);
            }
        }
        match self.vpn.lifecycle() {
            NativeVpnLifecycle::Running {
                generation: active, ..
            } if active == generation => {}
            NativeVpnLifecycle::Failed { error, .. } => {
                self.stop_mixed_listener()?;
                return Err(PawsError::Core(error));
            }
            _ => {
                self.stop_mixed_listener()?;
                return Err(PawsError::Core(
                    "native VPN worker exited during startup".to_owned(),
                ));
            }
        }
        let commit_result = {
            let mut state = self.lock_state()?;
            let attempt_result = if let Some(attempt_id) = attempt_id {
                self.sync_platform_vpn_state_locked(&mut state);
                ensure_platform_attempt_active(&state, attempt_id)
            } else {
                Ok(())
            };
            match attempt_result {
                Err(error) => Err(error),
                Ok(()) => {
                    state.vpn_options = options;
                    state.engine_loaded = true;
                    state.platform_vpn_starting = false;
                    state.platform_vpn_running = true;
                    invalidate_exit_location(&mut state);
                    if state.platform_start_outcome == PlatformStartOutcome::Pending {
                        state.platform_start_outcome = PlatformStartOutcome::Connected;
                    }
                    state
                        .logs
                        .push(info_log(format!("vpn started with tun fd {fd}")));
                    match mixed_listener {
                        Ok(true) => state.logs.push(info_log(format!(
                            "meow mixed listener ready on 127.0.0.1:{mixed_port}"
                        ))),
                        Ok(false) => {}
                        Err(error) => state.logs.push(warning_log(format!(
                            "meow mixed listener failed to start: {error}"
                        ))),
                    }
                    self.persist_platform_vpn_state_locked(&mut state)
                }
            }
        };
        if let Err(error) = commit_result {
            self.vpn.stop().await?;
            self.stop_mixed_listener()?;
            return Err(error);
        }
        Ok(generation)
    }

    async fn restart_mixed_listener(
        &self,
        tunnel: Tunnel,
        mixed_port: u16,
    ) -> Result<bool, PawsError> {
        self.stop_mixed_listener()?;
        let Some(platform) = self.platform_ipc()? else {
            return Ok(false);
        };
        if platform.is_ui() {
            return Ok(false);
        }
        let network_protected = self.lock_state()?.platform_network_protected;
        if !network_protected {
            return Err(PawsError::Core(
                "VPN extension process network is not protected".to_owned(),
            ));
        }

        let addr = SocketAddr::from(([127, 0, 0, 1], mixed_port));
        let listener =
            MixedListener::new(tunnel, addr, "paws-mixed".to_owned()).with_max_connections(64);
        let task = tokio::spawn(async move {
            if let Err(error) = listener.run().await {
                tracing::warn!(
                    target: "paws_core::listener",
                    "meow mixed listener stopped: {error}"
                );
            }
        });
        *self
            .mixed_listener
            .lock()
            .map_err(|_| PawsError::Core("mixed listener lock poisoned".to_owned()))? =
            Some(MixedListenerRuntime { task });

        let deadline = tokio::time::Instant::now() + MIXED_LISTENER_READY_TIMEOUT;
        loop {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                return Ok(true);
            }
            let stopped = self
                .mixed_listener
                .lock()
                .map_err(|_| PawsError::Core("mixed listener lock poisoned".to_owned()))?
                .as_ref()
                .is_none_or(|runtime| runtime.task.is_finished());
            if stopped || tokio::time::Instant::now() >= deadline {
                self.stop_mixed_listener()?;
                return Err(PawsError::Core(format!(
                    "127.0.0.1:{mixed_port} did not become ready"
                )));
            }
            tokio::time::sleep(MIXED_LISTENER_READY_RETRY).await;
        }
    }

    fn stop_mixed_listener(&self) -> Result<(), PawsError> {
        self.mixed_listener
            .lock()
            .map_err(|_| PawsError::Core("mixed listener lock poisoned".to_owned()))?
            .take();
        Ok(())
    }

    /// Ensure the active meow tunnel is ready before the platform supplies a
    /// TUN descriptor. VPN Extension can run this concurrently with native
    /// `VpnConnection::create`, removing config parsing from the serial start
    /// path. Returns `true` when a cold config load was required.
    pub async fn prepare_active_vpn(&self) -> Result<bool, PawsError> {
        let tunnel = {
            let state = self.lock_state()?;
            state.tunnel.clone()
        };
        if tunnel
            .as_ref()
            .is_some_and(|tunnel| !tunnel.route_snapshot().proxies.is_empty())
        {
            return Ok(false);
        }

        // UI bootstrap and a fast user tap can reach this path together. Let
        // the first task finish the expensive meow config build, then reuse
        // its tunnel instead of parsing the subscription a second time.
        let _reload_guard = self.config_reload_lock.lock().await;
        let (active_profile, ready) = {
            let state = self.lock_state()?;
            (
                state.profiles.active_profile().map(ToOwned::to_owned),
                state
                    .tunnel
                    .as_ref()
                    .is_some_and(|tunnel| !tunnel.route_snapshot().proxies.is_empty()),
            )
        };
        if ready {
            return Ok(false);
        }
        let active_profile = active_profile
            .ok_or_else(|| PawsError::Core("activate a profile before starting VPN".to_owned()))?;
        self.reload_config_inner(&active_profile).await?;
        let ready = {
            let mut state = self.lock_state()?;
            let ready = state
                .tunnel
                .as_ref()
                .is_some_and(|tunnel| !tunnel.route_snapshot().proxies.is_empty());
            self.publish_status_and_resource_change_locked(&mut state);
            ready
        };
        if !ready {
            return Err(PawsError::Core(
                "active meow tunnel has no proxies; reload the profile first".to_owned(),
            ));
        }
        Ok(true)
    }

    /// Prepare native state only for the currently owned platform request.
    /// The second check prevents a superseded asynchronous prepare from being
    /// treated as authorization to continue into TUN creation.
    pub async fn prepare_platform_vpn(&self, attempt_id: &str) -> Result<bool, PawsError> {
        let _operation_guard = self.vpn_operation_lock.lock().await;
        {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            ensure_platform_attempt_active(&state, attempt_id)?;
        }
        let loaded = self.prepare_active_vpn().await?;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        ensure_platform_attempt_active(&state, attempt_id)?;
        Ok(loaded)
    }

    /// Evaluate a domain or IP against the active profile's compiled rules.
    ///
    /// This intentionally bypasses the tunnel's mode dispatch and statistics:
    /// the result describes Rule-mode routing without counting an inspection
    /// as real traffic.
    pub async fn lookup_rule(&self, query: &str) -> Result<RuleLookupResult, PawsError> {
        let (query, input_kind, mut metadata) = rule_lookup_metadata(query)?;
        self.prepare_active_vpn().await?;
        let tunnel = {
            let state = self.lock_state()?;
            state.tunnel.clone()
        }
        .ok_or_else(|| PawsError::Core("active meow tunnel is not loaded".to_owned()))?;

        let route = tunnel.route_snapshot();
        let mut resolution_attempted = false;
        let mut resolved_ip = None;
        let matched = match route
            .compiled_rules
            .match_rules_lazy(&metadata, route.rules.as_ref())
        {
            LazyMatchOutcome::Matched(matched) => Some((
                matched.rule_index,
                matched.rule_type.to_string(),
                matched.rule_payload.to_owned(),
                matched.adapter_name.to_owned(),
            )),
            LazyMatchOutcome::NeedsEnrichment {
                needs_ip,
                needs_process: _,
            } => {
                if needs_ip {
                    resolution_attempted = true;
                    metadata.dst_ip = tunnel.resolver().resolve_ip_real(&metadata.host).await;
                    resolved_ip = metadata.dst_ip.map(|ip| ip.to_string());
                }
                route
                    .compiled_rules
                    .match_rules(&metadata, route.rules.as_ref())
                    .map(|matched| {
                        (
                            matched.rule_index,
                            matched.rule_type.to_string(),
                            matched.rule_payload.to_owned(),
                            matched.adapter_name.to_owned(),
                        )
                    })
            }
            LazyMatchOutcome::NoMatch => None,
        };

        let Some((rule_index, rule_type, rule_payload, target)) = matched else {
            return Ok(RuleLookupResult {
                query,
                input_kind,
                resolved_ip,
                resolution_attempted,
                matched: false,
                rule_type: None,
                rule_payload: None,
                target: "DIRECT".to_owned(),
                rule_line: None,
            });
        };
        let rule_line = {
            let state = self.lock_state()?;
            state
                .runtime_rules
                .iter()
                .find(|rule| rule.enabled && rule.order as usize == rule_index)
                .map(|rule| rule.line.clone())
        }
        .or_else(|| {
            if rule_payload.is_empty() {
                Some(format!("{rule_type},{target}"))
            } else {
                Some(format!("{rule_type},{rule_payload},{target}"))
            }
        });

        Ok(RuleLookupResult {
            query,
            input_kind,
            resolved_ip,
            resolution_attempted,
            matched: true,
            rule_type: Some(rule_type),
            rule_payload: Some(rule_payload),
            target,
            rule_line,
        })
    }

    pub fn active_vpn_options_json(&self) -> Result<String, PawsError> {
        let state = self.lock_state()?;
        to_json(&state.profiles.active_vpn_options()?)
    }

    /// Issue a process-wide ordering token for a typed VPN intent. Bridge
    /// instances share this Core, so a later Start/Stop supersedes work still
    /// queued by an older Plugin nonce before it can touch the platform.
    pub fn advance_platform_vpn_intent(&self) -> Result<u64, PawsError> {
        let mut state = self.lock_state()?;
        state.platform_vpn_intent_epoch = state
            .platform_vpn_intent_epoch
            .checked_add(1)
            .ok_or_else(|| PawsError::Core("platform VPN intent epoch exhausted".to_owned()))?;
        Ok(state.platform_vpn_intent_epoch)
    }

    pub fn is_platform_vpn_intent_current(&self, intent_epoch: u64) -> Result<bool, PawsError> {
        let state = self.lock_state()?;
        Ok(intent_epoch > 0 && state.platform_vpn_intent_epoch == intent_epoch)
    }

    /// Check that a queued Plugin operation still owns the exact, not-yet-
    /// dispatched OS stop fence. The final check is followed synchronously by
    /// `begin_platform_vpn_os_stop`, which closes the cross-realm race before
    /// ArkTS invokes the system API.
    pub fn is_platform_vpn_stop_current(
        &self,
        intent_epoch: u64,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        let state = self.lock_state()?;
        let exact_fence = state.platform_os_stop_epoch == intent_epoch
            && state.platform_os_stop_attempt_id == attempt_id;
        Ok(intent_epoch > 0
            && state.platform_vpn_intent_epoch == intent_epoch
            && exact_fence
            && !state.platform_os_stop_in_flight)
    }

    /// Atomically mark the exact OS stop as dispatched. Once this succeeds a
    /// newer intent cannot begin a VPN start until the system stop Promise is
    /// reported as confirmed or failed.
    pub fn begin_platform_vpn_os_stop(
        &self,
        intent_epoch: u64,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        if intent_epoch == 0
            || state.platform_vpn_intent_epoch != intent_epoch
            || state.platform_os_stop_in_flight
        {
            return Ok(false);
        }
        if state.platform_os_stop_epoch == 0 {
            if !attempt_id.is_empty() {
                return Ok(false);
            }
            state.platform_os_stop_epoch = intent_epoch;
            state.platform_os_stop_attempt_id.clear();
        } else if state.platform_os_stop_epoch != intent_epoch
            || state.platform_os_stop_attempt_id != attempt_id
        {
            return Ok(false);
        }
        state.platform_os_stop_in_flight = true;
        Ok(true)
    }

    /// Release only the exact in-flight fence after HarmonyOS confirms the OS
    /// Extension stop. This deliberately does not require the intent to remain
    /// newest: a later request cannot revoke a system call already dispatched.
    pub fn complete_platform_vpn_os_stop(
        &self,
        intent_epoch: u64,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        if state.platform_os_stop_epoch != intent_epoch
            || state.platform_os_stop_attempt_id != attempt_id
            || !state.platform_os_stop_in_flight
        {
            return Ok(false);
        }
        state.platform_os_stop_epoch = 0;
        state.platform_os_stop_attempt_id.clear();
        state.platform_os_stop_in_flight = false;
        Ok(true)
    }

    /// A rejected/failed system Promise is not stop confirmation. Retain the
    /// exact fence but make it claimable by a later explicit Stop retry.
    pub fn fail_platform_vpn_os_stop(
        &self,
        intent_epoch: u64,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        if state.platform_os_stop_epoch != intent_epoch
            || state.platform_os_stop_attempt_id != attempt_id
            || !state.platform_os_stop_in_flight
        {
            return Ok(false);
        }
        state.platform_os_stop_in_flight = false;
        Ok(true)
    }

    /// Begin one platform VPN start transaction for the latest typed intent.
    ///
    /// The system ability-start Promise is only a dispatch acknowledgement.
    /// Completion is determined by the matching VPN Extension terminal state
    /// published through shared memory.
    #[cfg(test)]
    pub fn begin_platform_vpn_start(&self) -> Result<String, PawsError> {
        let intent_epoch = self.advance_platform_vpn_intent()?;
        self.begin_platform_vpn_start_for_intent(intent_epoch)
    }

    pub fn begin_platform_vpn_start_for_intent(
        &self,
        intent_epoch: u64,
    ) -> Result<String, PawsError> {
        let issuer = current_process_identity()?;
        let mut state = self.lock_state()?;
        if intent_epoch == 0 || state.platform_vpn_intent_epoch != intent_epoch {
            return Err(PawsError::Core(format!(
                "platform VPN start intent {intent_epoch} was superseded"
            )));
        }
        if state.platform_os_stop_epoch != 0 {
            return Err(PawsError::Core(format!(
                "platform VPN OS stop for attempt {} is still pending confirmation",
                state.platform_os_stop_attempt_id
            )));
        }
        self.sync_platform_vpn_state_locked(&mut state);
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            return Err(PawsError::Core(
                "platform VPN start is already pending".to_owned(),
            ));
        }
        if !state.platform_start_attempt_id.is_empty() && !state.platform_vpn_cleanup_complete {
            return Err(PawsError::Core(
                "previous platform VPN connection cleanup is still pending".to_owned(),
            ));
        }
        if state.platform_vpn_running || self.vpn.is_running() {
            return Err(PawsError::Core(
                "platform VPN is already connected".to_owned(),
            ));
        }

        let next_sequence = state.platform_start_sequence.saturating_add(1);
        let attempt_id = format!("{}-{next_sequence}", now_unix_nanos());
        let journal_path = platform_owner_journal_path(&state);
        match platform_owner::read(&journal_path)? {
            JournalRead::Missing => {}
            JournalRead::Present(owner) => {
                return Err(PawsError::Core(format!(
                    "previous platform VPN owner journal for attempt {} is still pending cleanup",
                    owner.attempt_id
                )))
            }
        }
        let issuer_lease = platform_owner::acquire_owner_lease_exact(
            &platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Issuer),
            platform_owner_lease_record(
                &attempt_id,
                issuer.clone(),
                PlatformVpnOwnerLeaseRole::Issuer,
            ),
        )?;
        platform_owner::create_pending_exact(
            &journal_path,
            PlatformVpnOwnerJournal {
                attempt_id: attempt_id.clone(),
                issuer,
                extension: None,
                phase: PlatformVpnOwnerPhase::Pending,
            },
        )?;
        state.platform_vpn_issuer_lease = Some(issuer_lease);
        state.platform_vpn_extension_lease = None;
        state.platform_start_sequence = next_sequence;
        state.platform_start_attempt_id = attempt_id.clone();
        state.platform_start_outcome = PlatformStartOutcome::Pending;
        state.platform_start_delivery_observed = false;
        state.platform_extension_attached = false;
        state.platform_stop_requested = false;
        state.platform_extension_owner_pid = 0;
        state.platform_extension_owner_start_time = 0;
        // Dispatch may race a stop before the Extension's bind frame reaches
        // the UI. Require an explicit cleanup acknowledgement once an attempt
        // exists; only a confirmed unattached dispatch failure may waive it.
        state.platform_vpn_cleanup_complete = false;
        state.platform_vpn_starting = true;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = None;
        // Remote revisions are scoped to one attempt. A replacement
        // Extension can start its process-local timestamp below the previous
        // owner's last value (for example after a wall-clock correction); do
        // not discard every heartbeat of the new session as stale.
        state.platform_remote_state_updated_at = 0;
        state.platform_remote_state_seen_at = None;
        state.platform_remote_stale_since = None;
        state.platform_watchdog_cleanup_recoverable = false;
        invalidate_exit_location(&mut state);
        state.logs.push(info_log(format!(
            "platform VPN start transaction {attempt_id}"
        )));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(attempt_id)
    }

    /// Bind the VPN Extension process to the transaction delivered in its Want.
    pub fn bind_platform_vpn_start(&self, attempt_id: &str) -> Result<String, PawsError> {
        if attempt_id.is_empty() {
            return Err(PawsError::Core(
                "platform VPN start attempt id is empty".to_owned(),
            ));
        }
        let extension_owner = current_process_identity()?;
        let mut state = self.lock_state()?;
        self.sync_platform_for_binding_locked(&mut state, attempt_id)?;
        if state.platform_start_attempt_id != attempt_id {
            return Err(PawsError::Core(format!(
                "stale platform VPN start attempt {attempt_id}"
            )));
        }
        if matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        ) {
            return Err(PawsError::Core(format!(
                "platform VPN start attempt {attempt_id} is already terminal"
            )));
        }
        if state.platform_start_outcome == PlatformStartOutcome::Connected && !self.vpn.is_running()
        {
            // HarmonyOS may recreate the Extension process after a crash and
            // redeliver the same still-connected Want. The fresh native core
            // has no worker, so reopen only this non-terminal attempt's start
            // phase. UI-side synchronization keeps the same owner identity;
            // Failed/Cancelled attempts remain irrevocably terminal above.
            state.platform_start_outcome = PlatformStartOutcome::Pending;
            state.platform_vpn_starting = true;
            state.platform_vpn_running = false;
            state.platform_vpn_cleanup_complete = false;
            state.platform_network_protected = false;
            state.platform_network_protect_error = None;
            state.platform_watchdog_cleanup_recoverable = false;
            state.logs.push(info_log(format!(
                "platform VPN extension recovering connected attempt {attempt_id}"
            )));
        }
        let journal_path = platform_owner_journal_path(&state);
        let journal = match platform_owner::read(&journal_path)? {
            JournalRead::Missing => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal is missing for attempt {attempt_id}"
                )))
            }
            JournalRead::Present(journal) if journal.attempt_id == attempt_id => journal,
            JournalRead::Present(journal) => {
                return Err(PawsError::Core(format!(
                    "stale platform VPN start attempt {attempt_id}; owner journal belongs to {}",
                    journal.attempt_id
                )))
            }
        };
        let issuer_lease_path =
            platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Issuer);
        let extension_lease_path =
            platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Extension);
        let extension_lease_record = platform_owner_lease_record(
            attempt_id,
            extension_owner.clone(),
            PlatformVpnOwnerLeaseRole::Extension,
        );
        let acquired_extension_lease = match (journal.phase, journal.extension.as_ref()) {
            (PlatformVpnOwnerPhase::Pending, None) => {
                let issuer_lease_record = platform_owner_lease_record(
                    attempt_id,
                    journal.issuer.clone(),
                    PlatformVpnOwnerLeaseRole::Issuer,
                );
                match platform_owner::observe_owner_lease_exact(
                    &issuer_lease_path,
                    &issuer_lease_record,
                )? {
                    PlatformVpnOwnerLeaseObservation::HeldExact => {}
                    PlatformVpnOwnerLeaseObservation::Released => {
                        return Err(PawsError::Core(format!(
                            "platform VPN start issuer lease for attempt {attempt_id} was released"
                        )))
                    }
                    PlatformVpnOwnerLeaseObservation::HeldOther => {
                        return Err(PawsError::Core(format!(
                            "cannot verify the exact platform VPN start issuer lease for attempt {attempt_id}"
                        )))
                    }
                }
                let lease = platform_owner::acquire_owner_lease_exact(
                    &extension_lease_path,
                    extension_lease_record.clone(),
                )?;
                platform_owner::upgrade_attached_exact(
                    &journal_path,
                    attempt_id,
                    journal.issuer.clone(),
                    extension_owner.clone(),
                )?;
                Some(lease)
            }
            (PlatformVpnOwnerPhase::Attached, Some(current_owner))
                if current_owner == &extension_owner =>
            {
                if state.platform_vpn_extension_lease.is_none() {
                    Some(platform_owner::acquire_owner_lease_exact(
                        &extension_lease_path,
                        extension_lease_record,
                    )?)
                } else {
                    None
                }
            }
            (PlatformVpnOwnerPhase::Attached, Some(current_owner)) => {
                let previous_lease_record = platform_owner_lease_record(
                    attempt_id,
                    current_owner.clone(),
                    PlatformVpnOwnerLeaseRole::Extension,
                );
                match platform_owner::observe_owner_lease_exact(
                    &extension_lease_path,
                    &previous_lease_record,
                )? {
                    PlatformVpnOwnerLeaseObservation::Released => {}
                    PlatformVpnOwnerLeaseObservation::HeldExact => {
                        return Err(PawsError::Core(format!(
                            "platform VPN attempt {attempt_id} is still owned by another Extension process"
                        )))
                    }
                    PlatformVpnOwnerLeaseObservation::HeldOther => {
                        return Err(PawsError::Core(format!(
                            "cannot verify the exact previous Extension lease for attempt {attempt_id}"
                        )))
                    }
                }
                let lease = platform_owner::acquire_owner_lease_exact(
                    &extension_lease_path,
                    extension_lease_record,
                )?;
                platform_owner::rebind_attached_exact(
                    &journal_path,
                    attempt_id,
                    journal.issuer.clone(),
                    current_owner.clone(),
                    extension_owner.clone(),
                )?;
                Some(lease)
            }
            _ => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal has an invalid phase for attempt {attempt_id}"
                )))
            }
        };
        if let Some(lease) = acquired_extension_lease {
            state.platform_vpn_extension_lease = Some(lease);
        }
        self.publish_platform_vpn_binding_locked(&mut state, attempt_id, &extension_owner)
    }

    fn publish_platform_vpn_binding_locked(
        &self,
        state: &mut CoreState,
        attempt_id: &str,
        extension_owner: &ProcessIdentity,
    ) -> Result<String, PawsError> {
        // The Stop side publishes its terminal lane state before racing the
        // journal Pending→Stopping CAS. If this Extension won Pending→Attached,
        // refresh that lane now so the stale pre-CAS snapshot cannot revive a
        // cancelled attempt when ownership is published below.
        self.sync_platform_vpn_state_locked(state);
        if state.platform_start_attempt_id != attempt_id {
            return Err(PawsError::Core(format!(
                "platform VPN start attempt {attempt_id} was superseded after owner attachment"
            )));
        }
        let owner_pid = extension_owner.pid;
        let owner_start_time = extension_owner.start_time;
        let owner_identity_changed = state.platform_extension_owner_pid != owner_pid
            || state.platform_extension_owner_start_time != owner_start_time;
        state.platform_extension_owner_pid = owner_pid;
        state.platform_extension_owner_start_time = owner_start_time;
        if !state.platform_extension_attached {
            state.platform_start_delivery_observed = true;
            state.platform_extension_attached = true;
            state.platform_vpn_cleanup_complete = false;
            state.platform_watchdog_cleanup_recoverable = false;
            state.logs.push(info_log(format!(
                "platform VPN extension attached to {attempt_id} with owner process {owner_pid}:{owner_start_time}"
            )));
            self.persist_platform_vpn_state_locked(state)?;
        } else if state.platform_start_outcome == PlatformStartOutcome::Pending
            && state.platform_vpn_starting
            && !state.platform_vpn_running
        {
            state.platform_start_delivery_observed = true;
            self.persist_platform_vpn_state_locked(state)?;
        } else if owner_identity_changed {
            // A system-recreated Extension process may rebind the same exact
            // attempt. Fence any later orphan recovery to this new process,
            // not the PID/start identity of the crashed predecessor.
            self.persist_platform_vpn_state_locked(state)?;
        }
        Ok(if owner_start_time > 0 {
            format!("{owner_pid}:{owner_start_time}")
        } else {
            format!("{owner_pid}:unavailable")
        })
    }

    pub async fn await_platform_vpn_start(
        &self,
        attempt_id: &str,
    ) -> Result<PlatformStartOutcome, PawsError> {
        self.await_platform_vpn_start_with_deadline(attempt_id, PLATFORM_VPN_START_DEADLINE)
            .await
    }

    /// Wait until the exact platform start owner has attached, or until its
    /// lifecycle can no longer attach without first completing the outstanding
    /// system dispatch. This is deliberately separate from start completion:
    /// HarmonyOS may leave `startVpnExtensionAbility` pending even though the
    /// Extension is already running.
    #[cfg(test)]
    pub async fn await_platform_vpn_attach(
        &self,
        attempt_id: &str,
    ) -> Result<PlatformAttachOutcome, PawsError> {
        if attempt_id.is_empty() {
            return Ok(PlatformAttachOutcome::Superseded);
        }
        let mut receiver = self.platform_start_tx.subscribe();
        loop {
            let outcome = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                if state.platform_start_attempt_id != attempt_id {
                    Some(PlatformAttachOutcome::Superseded)
                } else if state.platform_extension_attached
                    || state.platform_start_outcome == PlatformStartOutcome::Connected
                {
                    Some(PlatformAttachOutcome::Attached)
                } else if state.platform_start_delivery_observed {
                    Some(PlatformAttachOutcome::Delivered)
                } else if matches!(
                    state.platform_start_outcome,
                    PlatformStartOutcome::Idle
                        | PlatformStartOutcome::Failed
                        | PlatformStartOutcome::Cancelled
                ) {
                    Some(PlatformAttachOutcome::Terminal)
                } else {
                    None
                }
            };
            if let Some(outcome) = outcome {
                return Ok(outcome);
            }
            receiver.changed().await.map_err(|_| {
                PawsError::Core("platform VPN attach coordinator closed".to_owned())
            })?;
        }
    }

    pub async fn await_platform_vpn_stop(&self, attempt_id: &str) -> Result<bool, PawsError> {
        if attempt_id.is_empty() {
            return Ok(true);
        }
        let mut receiver = self.platform_start_tx.subscribe();
        let deadline = tokio::time::Instant::now() + PLATFORM_VPN_START_DEADLINE;
        loop {
            let stopped = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                if state.platform_start_attempt_id != attempt_id {
                    return Ok(false);
                }
                !state.platform_vpn_running
                    && !state.platform_vpn_starting
                    && state.platform_vpn_cleanup_complete
                    && matches!(
                        state.platform_start_outcome,
                        PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
                    )
            };
            if stopped {
                return Ok(true);
            }
            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| PawsError::Core(
                        "platform VPN stop coordinator closed".to_owned()
                    ))?;
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(PawsError::Core(format!(
                        "platform VPN attempt {attempt_id} did not stop before the cleanup deadline"
                    )));
                }
            }
        }
    }

    /// Acknowledge that the exact HarmonyOS VpnConnection owner has finished
    /// every native/platform operation and its destroy Promise has resolved.
    pub fn complete_platform_vpn_cleanup(&self, attempt_id: &str) -> Result<bool, PawsError> {
        let acknowledging_owner = current_process_identity()?;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if attempt_id.is_empty() || state.platform_start_attempt_id != attempt_id {
            return Ok(false);
        }
        if state.platform_vpn_running
            || state.platform_vpn_starting
            || !matches!(
                state.platform_start_outcome,
                PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
            )
        {
            return Ok(false);
        }
        if state.platform_vpn_cleanup_complete {
            return Ok(true);
        }
        if state.platform_vpn_extension_lease.is_none() {
            return Err(PawsError::Core(format!(
                "platform VPN Extension lease is missing before cleanup acknowledgement for {attempt_id}"
            )));
        }
        let journal_path = platform_owner_journal_path(&state);
        let journal = match platform_owner::read(&journal_path)? {
            JournalRead::Missing => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal is missing before cleanup acknowledgement for {attempt_id}"
                )))
            }
            JournalRead::Present(journal) if journal.attempt_id == attempt_id => journal,
            JournalRead::Present(_) => return Ok(false),
        };
        let expected_extension = match (journal.phase, journal.extension) {
            (PlatformVpnOwnerPhase::Attached, Some(extension))
                if extension == acknowledging_owner
                    && extension.pid == state.platform_extension_owner_pid
                    && extension.start_time == state.platform_extension_owner_start_time =>
            {
                extension
            }
            _ => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal does not match the Extension acknowledging cleanup for {attempt_id}"
                )))
            }
        };
        if !platform_owner::delete_exact(&journal_path, attempt_id, Some(expected_extension))? {
            return Err(PawsError::Core(format!(
                "platform VPN owner journal changed before cleanup acknowledgement for {attempt_id}"
            )));
        }
        state.platform_vpn_cleanup_complete = true;
        state.platform_extension_attached = false;
        state.platform_stop_requested = true;
        state.platform_extension_owner_pid = 0;
        state.platform_extension_owner_start_time = 0;
        state.platform_watchdog_cleanup_recoverable = false;
        state.logs.push(info_log(format!(
            "platform VPN connection cleanup completed for {attempt_id}"
        )));
        // Exact journal deletion is the cleanup linearization point. Once it
        // succeeds, retaining either ownership lease cannot make a failed IPC
        // notification safer and can self-deadlock the next same-process start.
        state.platform_vpn_issuer_lease = None;
        state.platform_vpn_extension_lease = None;
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    /// Return the exact terminal owner whose missing cleanup acknowledgement
    /// may be recovered after the OS confirms the Extension has stopped.
    /// This covers both a watchdog-proven orphan and a terminal request whose
    /// Want was delivered but rejected before the Extension adopted it.
    #[cfg(test)]
    pub fn current_recoverable_platform_vpn_session_id(&self) -> Result<String, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(can_attempt_platform_cleanup_recovery(&state)
            .then(|| state.platform_start_attempt_id.clone())
            .unwrap_or_default())
    }

    /// Release a cleanup barrier only after the caller has awaited the OS
    /// Extension-stop operation. Recovery additionally requires either an
    /// exact delivered-but-never-attached request, or release of the fenced
    /// Extension's exact ownership lease. Release can follow full cleanup or
    /// process exit; a quiet heartbeat alone proves neither.
    pub async fn recover_platform_vpn_cleanup_after_confirmed_stop(
        &self,
        attempt_id: &str,
    ) -> Result<bool, PawsError> {
        if attempt_id.is_empty() {
            return Ok(false);
        }
        let deadline = tokio::time::Instant::now() + PLATFORM_OS_STOP_RECOVERY_DEADLINE;
        loop {
            let (journal_path, issuer_lease_path, extension_lease_path, local_unattached_fence) = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                if state.platform_start_attempt_id == attempt_id
                    && state.platform_vpn_cleanup_complete
                {
                    return Ok(true);
                }
                let local_unattached_fence = state.platform_start_attempt_id == attempt_id
                    && !state.platform_extension_attached
                    && state.platform_stop_requested
                    && matches!(
                        state.platform_start_outcome,
                        PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
                    );
                (
                    platform_owner_journal_path(&state),
                    platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Issuer),
                    platform_owner_lease_path(&state, PlatformVpnOwnerLeaseRole::Extension),
                    local_unattached_fence,
                )
            };
            let journal = match platform_owner::read(&journal_path)? {
                JournalRead::Missing => {
                    let mut state = self.lock_state()?;
                    self.sync_platform_vpn_state_locked(&mut state);
                    if state.platform_start_attempt_id == attempt_id
                        && state.platform_vpn_cleanup_complete
                    {
                        return Ok(true);
                    }
                    return Err(PawsError::Core(format!(
                        "platform VPN owner journal disappeared before cleanup was confirmed for {attempt_id}"
                    )));
                }
                JournalRead::Present(journal) if journal.attempt_id == attempt_id => journal,
                JournalRead::Present(_) => return Ok(false),
            };
            let (expected_extension, proof, recovery_reason) =
                match (journal.phase, journal.extension.as_ref()) {
                    (PlatformVpnOwnerPhase::Stopping, None) => (
                        None,
                        CleanupRecoveryProof::Proven,
                        "the exact pending owner was durably fenced before confirmed OS stop"
                            .to_owned(),
                    ),
                    (PlatformVpnOwnerPhase::Pending, None) if local_unattached_fence => (
                        None,
                        CleanupRecoveryProof::Proven,
                        "the exact local attempt was terminal before an Extension adopted it"
                            .to_owned(),
                    ),
                    (PlatformVpnOwnerPhase::Pending, None) => {
                        let expected = platform_owner_lease_record(
                            attempt_id,
                            journal.issuer.clone(),
                            PlatformVpnOwnerLeaseRole::Issuer,
                        );
                        let proof = match platform_owner::observe_owner_lease_exact(
                            &issuer_lease_path,
                            &expected,
                        )? {
                            PlatformVpnOwnerLeaseObservation::Released => {
                                CleanupRecoveryProof::Proven
                            }
                            PlatformVpnOwnerLeaseObservation::HeldExact => {
                                CleanupRecoveryProof::OwnerAlive
                            }
                            PlatformVpnOwnerLeaseObservation::HeldOther => {
                                CleanupRecoveryProof::OwnerLivenessUnknown
                            }
                        };
                        (
                            None,
                            proof,
                            format!(
                                "exact pending issuer lease {}:{} was released",
                                journal.issuer.pid, journal.issuer.start_time
                            ),
                        )
                    }
                    (PlatformVpnOwnerPhase::Attached, Some(extension)) => {
                        let expected = platform_owner_lease_record(
                            attempt_id,
                            extension.clone(),
                            PlatformVpnOwnerLeaseRole::Extension,
                        );
                        let proof = match platform_owner::observe_owner_lease_exact(
                            &extension_lease_path,
                            &expected,
                        )? {
                            PlatformVpnOwnerLeaseObservation::Released => {
                                CleanupRecoveryProof::Proven
                            }
                            PlatformVpnOwnerLeaseObservation::HeldExact => {
                                CleanupRecoveryProof::OwnerAlive
                            }
                            PlatformVpnOwnerLeaseObservation::HeldOther => {
                                CleanupRecoveryProof::OwnerLivenessUnknown
                            }
                        };
                        (
                            Some(extension.clone()),
                            proof,
                            format!(
                                "exact Extension owner lease {}:{} was released",
                                extension.pid, extension.start_time
                            ),
                        )
                    }
                    _ => {
                        return Err(PawsError::Core(format!(
                            "platform VPN owner journal has an invalid phase for {attempt_id}"
                        )))
                    }
                };
            match proof {
                CleanupRecoveryProof::Proven => {
                    if !platform_owner::delete_exact(&journal_path, attempt_id, expected_extension)?
                    {
                        // A Pending owner may have attached, or a released
                        // ownership lease may have been atomically rebound,
                        // between read and delete. Re-read the exact journal
                        // instead of letting stale proof clear its replacement.
                        continue;
                    }
                    let mut state = self.lock_state()?;
                    self.sync_platform_vpn_state_locked(&mut state);
                    if state.platform_start_attempt_id == attempt_id {
                        if state.platform_vpn_cleanup_complete {
                            return Ok(true);
                        }
                        state.platform_vpn_starting = false;
                        state.platform_vpn_running = false;
                        if state.platform_start_outcome != PlatformStartOutcome::Failed {
                            state.platform_start_outcome = PlatformStartOutcome::Cancelled;
                            state.platform_network_protect_error = None;
                        }
                        state.platform_network_protected = false;
                        state.platform_vpn_cleanup_complete = true;
                        state.platform_extension_attached = false;
                        state.platform_stop_requested = true;
                        state.platform_extension_owner_pid = 0;
                        state.platform_extension_owner_start_time = 0;
                        state.platform_watchdog_cleanup_recoverable = false;
                        state.logs.push(warning_log(format!(
                            "released orphaned platform VPN cleanup barrier for {attempt_id} after confirmed OS stop: {recovery_reason}"
                        )));
                        state.platform_vpn_issuer_lease = None;
                        state.platform_vpn_extension_lease = None;
                        self.persist_platform_vpn_state_locked(&mut state)?;
                    }
                    return Ok(true);
                }
                CleanupRecoveryProof::OwnerAlive => {
                    if tokio::time::Instant::now() >= deadline {
                        return Ok(false);
                    }
                    tokio::time::sleep(PLATFORM_OS_STOP_RECOVERY_POLL_INTERVAL).await;
                }
                CleanupRecoveryProof::OwnerLivenessUnknown => {
                    let identity = journal.extension.as_ref().unwrap_or(&journal.issuer);
                    return Err(PawsError::Core(format!(
                        "cannot verify the exact platform VPN ownership lease for {}:{}",
                        identity.pid, identity.start_time,
                    )));
                }
            }
        }
    }

    async fn await_platform_vpn_start_with_deadline(
        &self,
        attempt_id: &str,
        timeout: Duration,
    ) -> Result<PlatformStartOutcome, PawsError> {
        if attempt_id.is_empty() {
            return Err(PawsError::Core(
                "platform VPN start attempt id is empty".to_owned(),
            ));
        }
        let mut receiver = self.platform_start_tx.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let event = {
                let state = self.lock_state()?;
                self.platform_start_event_locked(&state)
            };
            if event.attempt_id != attempt_id {
                return Err(PawsError::Core(format!(
                    "platform VPN start attempt {attempt_id} was superseded"
                )));
            }
            match event.outcome {
                PlatformStartOutcome::Connected => {
                    return Ok(PlatformStartOutcome::Connected);
                }
                PlatformStartOutcome::Failed => {
                    return Err(PawsError::Core(
                        event
                            .error
                            .unwrap_or_else(|| "VPN extension failed to start".to_owned()),
                    ));
                }
                PlatformStartOutcome::Cancelled => {
                    return Err(PawsError::Core(
                        "VPN extension start was cancelled".to_owned(),
                    ));
                }
                PlatformStartOutcome::Idle => {
                    return Err(PawsError::Core(format!(
                        "platform VPN start attempt {attempt_id} is not active"
                    )));
                }
                PlatformStartOutcome::Pending => {}
            }

            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| PawsError::Core(
                        "platform VPN start coordinator closed".to_owned()
                    ))?;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    self.fail_platform_vpn_start(
                        attempt_id,
                        "VPN extension did not reach a terminal state before the startup deadline".to_owned(),
                    )?;
                }
            }
        }
    }

    /// Fail a matching request only before the VPN Extension accepts its Want.
    pub fn fail_unattached_platform_vpn_start(
        &self,
        attempt_id: &str,
        error: String,
    ) -> Result<bool, PawsError> {
        self.fail_platform_vpn_start_if(attempt_id, error, true)
    }

    /// Publish exactly one failure for the matching start transaction.
    pub fn fail_platform_vpn_start(
        &self,
        attempt_id: &str,
        error: String,
    ) -> Result<bool, PawsError> {
        self.fail_platform_vpn_start_if(attempt_id, error, false)
    }

    fn fail_platform_vpn_start_if(
        &self,
        attempt_id: &str,
        error: String,
        require_unattached: bool,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !self.platform_start_is_pending_locked(&state, attempt_id)
            || (require_unattached && state.platform_extension_attached)
        {
            return Ok(false);
        }
        if require_unattached {
            let journal_path = platform_owner_journal_path(&state);
            if !platform_owner::delete_pending_exact(&journal_path, attempt_id)? {
                // The Extension may have upgraded the journal before its
                // attached frame reached this UI lane, or Stop may have
                // persisted a Stopping tombstone. Never let a late dispatch
                // failure waive cleanup for either fenced owner.
                return Ok(false);
            }
        }
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = Some(error.clone());
        invalidate_exit_location(&mut state);
        state.platform_start_outcome = PlatformStartOutcome::Failed;
        if require_unattached {
            state.platform_vpn_cleanup_complete = true;
        }
        state.logs.push(warning_log(format!(
            "platform VPN start transaction {attempt_id} failed: {error}"
        )));
        if state.platform_vpn_cleanup_complete {
            state.platform_vpn_issuer_lease = None;
            state.platform_vpn_extension_lease = None;
        }
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    /// Cancel only the matching pending start transaction.
    pub fn cancel_platform_vpn_start(&self, attempt_id: &str) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !self.platform_start_is_pending_locked(&state, attempt_id) {
            return Ok(false);
        }
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = None;
        invalidate_exit_location(&mut state);
        state.platform_start_outcome = PlatformStartOutcome::Cancelled;
        state.logs.push(info_log(format!(
            "platform VPN start transaction {attempt_id} cancelled"
        )));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    /// Ask the exact attached Extension owner to tear down its native worker
    /// and VpnConnection while the Ability process is still alive. This is a
    /// control intent only: lifecycle and cleanup remain unchanged until the
    /// Extension publishes its real stop/join and destroy acknowledgements.
    pub fn request_platform_vpn_stop(&self, attempt_id: &str) -> Result<bool, PawsError> {
        if attempt_id.is_empty() {
            return Ok(false);
        }
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if state.platform_start_attempt_id != attempt_id {
            return Ok(false);
        }
        if state.platform_vpn_cleanup_complete {
            return Ok(true);
        }
        if !state.platform_extension_attached {
            return Ok(false);
        }
        if !state.platform_stop_requested {
            state.platform_stop_requested = true;
            state.logs.push(info_log(format!(
                "platform VPN cooperative stop requested for {attempt_id}"
            )));
            self.persist_platform_vpn_state_locked(&mut state)?;
        }
        Ok(true)
    }

    /// Atomically fence and return any exact platform owner that still owes
    /// cleanup. This survives Bridge/Plugin recreation: a pending start is
    /// made terminal before its late Want can attach, while a connected owner
    /// keeps reporting Connected until the Extension publishes real teardown.
    pub fn claim_current_platform_vpn_stop(&self, intent_epoch: u64) -> Result<String, PawsError> {
        let mut state = self.lock_state()?;
        if intent_epoch == 0 || state.platform_vpn_intent_epoch != intent_epoch {
            return Err(PawsError::Core(format!(
                "platform VPN stop intent {intent_epoch} was superseded"
            )));
        }
        if state.platform_os_stop_in_flight {
            return Err(PawsError::Core(format!(
                "platform VPN OS stop for attempt {} is already in flight",
                state.platform_os_stop_attempt_id
            )));
        }
        let retained_stop_attempt =
            (state.platform_os_stop_epoch != 0).then(|| state.platform_os_stop_attempt_id.clone());
        self.sync_platform_vpn_state_locked(&mut state);
        let journal_path = platform_owner_journal_path(&state);
        let journal = match platform_owner::read(&journal_path)? {
            JournalRead::Present(journal) => {
                if retained_stop_attempt
                    .as_ref()
                    .is_some_and(|attempt| attempt != &journal.attempt_id)
                {
                    return Err(PawsError::Core(format!(
                        "platform VPN OS stop fence owns {} while the owner journal owns {}",
                        retained_stop_attempt.as_deref().unwrap_or_default(),
                        journal.attempt_id
                    )));
                }
                if !state.platform_start_attempt_id.is_empty()
                    && !state.platform_vpn_cleanup_complete
                    && state.platform_start_attempt_id != journal.attempt_id
                {
                    return Err(PawsError::Core(format!(
                        "platform VPN state owns {} while the owner journal owns {}",
                        state.platform_start_attempt_id, journal.attempt_id
                    )));
                }
                journal
            }
            JournalRead::Missing => {
                if !state.platform_start_attempt_id.is_empty()
                    && !state.platform_vpn_cleanup_complete
                {
                    return Err(PawsError::Core(format!(
                        "platform VPN owner journal is missing for active attempt {}",
                        state.platform_start_attempt_id
                    )));
                }
                if let Some(attempt_id) = retained_stop_attempt {
                    state.platform_os_stop_epoch = intent_epoch;
                    state.platform_os_stop_attempt_id = attempt_id.clone();
                    state.platform_os_stop_in_flight = false;
                    return Ok(attempt_id);
                }
                return Ok(String::new());
            }
        };
        let attempt_id = journal.attempt_id.clone();
        if state.platform_start_attempt_id.is_empty() || state.platform_vpn_cleanup_complete {
            state.logs.push(info_log(format!(
                "platform VPN stop claimed cold owner journal {attempt_id}"
            )));
        } else {
            if state.platform_start_outcome == PlatformStartOutcome::Pending {
                state.platform_start_outcome = PlatformStartOutcome::Cancelled;
                state.platform_vpn_starting = false;
                state.platform_vpn_running = false;
                state.platform_network_protected = false;
                state.platform_network_protect_error = None;
            }
            state.platform_stop_requested = true;
            state.logs.push(info_log(format!(
                "platform VPN stop claimed exact owner {attempt_id}"
            )));
            // This terminal/stop state must be visible before Pending races
            // Attached below. An Extension which wins attachment re-syncs the
            // lane before it publishes ownership or starts native work.
            self.persist_platform_vpn_state_locked(&mut state)?;
        }
        match (journal.phase, journal.extension.as_ref()) {
            (PlatformVpnOwnerPhase::Stopping, None)
            | (PlatformVpnOwnerPhase::Attached, Some(_)) => {}
            (PlatformVpnOwnerPhase::Pending, None) => {
                if platform_owner::fence_pending_stop_exact(
                    &journal_path,
                    &attempt_id,
                    journal.issuer.clone(),
                )? {
                    state.logs.push(info_log(format!(
                        "platform VPN pending owner durably fenced for stop {attempt_id}"
                    )));
                } else {
                    match platform_owner::read(&journal_path)? {
                    JournalRead::Present(current)
                        if current.attempt_id == attempt_id
                            && matches!(
                                (current.phase, current.extension.as_ref()),
                                (PlatformVpnOwnerPhase::Stopping, None)
                                    | (PlatformVpnOwnerPhase::Attached, Some(_))
                            ) =>
                        {}
                        JournalRead::Present(current) => {
                            return Err(PawsError::Core(format!(
                                "platform VPN owner changed from {attempt_id} to {} while claiming stop",
                                current.attempt_id
                            )))
                        }
                        JournalRead::Missing => {
                            return Err(PawsError::Core(format!(
                                "platform VPN owner journal disappeared while claiming stop for {attempt_id}"
                            )))
                        }
                    }
                }
            }
            _ => {
                return Err(PawsError::Core(format!(
                    "platform VPN owner journal has an invalid phase while claiming stop for {attempt_id}"
                )))
            }
        }
        state.platform_os_stop_epoch = intent_epoch;
        state.platform_os_stop_attempt_id = attempt_id.clone();
        state.platform_os_stop_in_flight = false;
        Ok(attempt_id)
    }

    pub async fn stop_vpn(&self) -> Result<(), PawsError> {
        self.stop_vpn_inner(None).await.map(|_| ())
    }

    /// Stop only the platform session identified by `attempt_id`.
    /// A stale cleanup is a no-op and its completion cannot clear a newer
    /// transaction's state.
    pub async fn stop_platform_vpn(&self, attempt_id: &str) -> Result<bool, PawsError> {
        self.stop_vpn_inner(Some(attempt_id)).await
    }

    async fn stop_vpn_inner(&self, attempt_id: Option<&str>) -> Result<bool, PawsError> {
        let _operation_guard = self.vpn_operation_lock.lock().await;
        if let Some(attempt_id) = attempt_id {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            if state.platform_start_attempt_id != attempt_id {
                return Ok(false);
            }
            state.platform_vpn_starting = false;
            state.platform_vpn_running = false;
            state.platform_stop_requested = true;
            if !matches!(state.platform_start_outcome, PlatformStartOutcome::Failed) {
                state.platform_start_outcome = PlatformStartOutcome::Cancelled;
            }
            state.platform_network_protected = false;
            if state.platform_start_outcome != PlatformStartOutcome::Failed {
                state.platform_network_protect_error = None;
            }
            invalidate_exit_location(&mut state);
            self.persist_platform_vpn_state_locked(&mut state)?;
        }
        let stats = self.vpn.stats();
        self.vpn.stop().await?;
        self.stop_mixed_listener()?;
        let mut state = self.lock_state()?;
        if attempt_id.is_some_and(|attempt_id| state.platform_start_attempt_id != attempt_id) {
            return Ok(true);
        }
        if let Some(stats) = stats {
            apply_traffic_sample(&mut state, &stats)?;
        }
        baseline_meow_traffic_sample(&mut state);
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            state.platform_start_outcome = PlatformStartOutcome::Cancelled;
        }
        state.platform_network_protected = false;
        if state.platform_start_outcome != PlatformStartOutcome::Failed {
            state.platform_network_protect_error = None;
        }
        invalidate_exit_location(&mut state);
        state.traffic.upload_speed = 0;
        state.traffic.download_speed = 0;
        state.last_traffic_sample = None;
        state.logs.push(info_log("vpn stopped"));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    #[cfg(test)]
    pub fn set_platform_vpn_starting(&self, starting: bool) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        state.platform_vpn_starting = starting;
        if starting {
            state.platform_vpn_running = false;
            state.platform_network_protected = false;
            state.platform_network_protect_error = None;
        }
        state.logs.push(info_log(if starting {
            "platform vpn start requested"
        } else {
            "platform vpn start request cleared"
        }));
        self.persist_platform_vpn_state_locked(&mut state)
    }

    #[cfg(test)]
    pub fn expire_platform_vpn_start(&self) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        if !state.platform_vpn_starting || state.platform_vpn_running {
            return Ok(false);
        }
        state.platform_vpn_starting = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error =
            Some("VPN extension did not report readiness before the startup timeout".to_owned());
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            state.platform_start_outcome = PlatformStartOutcome::Failed;
        }
        state
            .logs
            .push(warning_log("platform vpn startup timed out"));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    #[cfg(test)]
    pub fn set_platform_vpn_running(&self, running: bool) -> Result<(), PawsError> {
        if !running {
            self.stop_mixed_listener()?;
        }
        let stats = if running { None } else { self.vpn.stats() };
        let mut state = self.lock_state()?;
        state.platform_vpn_starting = false;
        if state.platform_vpn_running != running {
            invalidate_exit_location(&mut state);
        }
        state.platform_vpn_running = running;
        if running {
            if state.platform_start_outcome == PlatformStartOutcome::Pending {
                state.platform_start_outcome = PlatformStartOutcome::Connected;
            }
        } else {
            if state.platform_start_outcome == PlatformStartOutcome::Pending {
                state.platform_start_outcome = PlatformStartOutcome::Cancelled;
            }
            settle_traffic_before_platform_stop(&mut state, stats.as_ref())?;
            state.platform_network_protected = false;
            state.platform_network_protect_error = None;
        }
        state.logs.push(info_log(if running {
            "platform vpn running"
        } else {
            "platform vpn stopped"
        }));
        self.persist_platform_vpn_state_locked(&mut state)
    }

    #[cfg(test)]
    pub fn set_platform_network_protected(
        &self,
        protected: bool,
        error: Option<String>,
    ) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        state.platform_network_protected = protected;
        state.platform_network_protect_error = error.filter(|value| !value.trim().is_empty());
        if protected {
            state.logs.push(info_log(
                "platform process network protected for VPN egress",
            ));
        } else if let Some(error) = state.platform_network_protect_error.clone() {
            state.logs.push(warning_log(format!(
                "platform network protect failed: {error}"
            )));
        } else {
            state
                .logs
                .push(info_log("platform network protection cleared"));
        }
        self.persist_platform_vpn_state_locked(&mut state)
    }

    pub fn set_platform_vpn_starting_for_attempt(
        &self,
        attempt_id: &str,
        starting: bool,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !platform_attempt_accepts_updates(&state, attempt_id) {
            return Ok(false);
        }
        if starting && state.platform_start_outcome != PlatformStartOutcome::Pending {
            return Ok(false);
        }
        state.platform_vpn_starting = starting;
        if starting {
            state.platform_vpn_running = false;
            state.platform_network_protected = false;
            state.platform_network_protect_error = None;
        }
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    pub fn set_platform_vpn_failed_for_attempt(
        &self,
        attempt_id: &str,
        error: String,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !platform_attempt_accepts_updates(&state, attempt_id) {
            return Ok(false);
        }
        apply_platform_failure(&mut state, error);
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    pub fn set_platform_network_protected_for_attempt(
        &self,
        attempt_id: &str,
        protected: bool,
        error: Option<String>,
    ) -> Result<bool, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !platform_attempt_accepts_updates(&state, attempt_id) {
            return Ok(false);
        }
        state.platform_network_protected = protected;
        state.platform_network_protect_error = error.filter(|value| !value.trim().is_empty());
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    /// Publish an Extension heartbeat from the real native worker lifecycle.
    /// An exited worker can no longer leave shared state reporting Connected.
    pub fn extension_tick(&self, attempt_id: &str) -> Result<String, PawsError> {
        let operation_guard = self.vpn_operation_lock.try_lock();
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if state.platform_start_attempt_id != attempt_id {
            return Err(PawsError::Core(format!(
                "stale platform VPN heartbeat for attempt {attempt_id}"
            )));
        }
        if state.platform_stop_requested {
            return Ok("stopping".to_owned());
        }
        if matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        ) {
            return Ok(
                if state.platform_start_outcome == PlatformStartOutcome::Failed {
                    "failed"
                } else {
                    "disconnected"
                }
                .to_owned(),
            );
        }
        if !state.platform_extension_attached {
            return Err(PawsError::Core(format!(
                "platform VPN heartbeat has no owner for attempt {attempt_id}"
            )));
        }
        let Ok(_operation_guard) = operation_guard else {
            // A serialized prepare/start/stop or runtime reconfiguration may
            // briefly transition the native worker internally. Heartbeat must
            // not turn that implementation detail into a terminal session.
            // Keep publishing the last committed platform lifecycle until the
            // owner operation completes and its exact callback is observed.
            let lifecycle = if state.platform_vpn_running
                && state.platform_start_outcome == PlatformStartOutcome::Connected
            {
                "connected"
            } else {
                "starting"
            };
            self.persist_platform_vpn_state_locked(&mut state)?;
            return Ok(lifecycle.to_owned());
        };
        match self.vpn.lifecycle() {
            NativeVpnLifecycle::Running { .. } => {
                state.platform_vpn_starting = false;
                state.platform_vpn_running = true;
                if state.platform_start_outcome == PlatformStartOutcome::Pending {
                    state.platform_start_outcome = PlatformStartOutcome::Connected;
                }
                self.persist_platform_vpn_state_locked(&mut state)?;
                Ok("connected".to_owned())
            }
            NativeVpnLifecycle::Failed { error, .. } => {
                apply_platform_failure(&mut state, error);
                self.persist_platform_vpn_state_locked(&mut state)?;
                Ok("failed".to_owned())
            }
            NativeVpnLifecycle::Stopped => {
                if state.platform_start_outcome == PlatformStartOutcome::Pending
                    && state.platform_vpn_starting
                {
                    self.persist_platform_vpn_state_locked(&mut state)?;
                    Ok("starting".to_owned())
                } else if state.platform_vpn_running {
                    apply_platform_failure(
                        &mut state,
                        "native VPN worker is not running".to_owned(),
                    );
                    self.persist_platform_vpn_state_locked(&mut state)?;
                    Ok("failed".to_owned())
                } else {
                    Ok("disconnected".to_owned())
                }
            }
        }
    }

    async fn publish_native_vpn_exit(&self, attempt_id: &str, generation: u64, error: String) {
        // Serialize the generation check and mixed-listener teardown with all
        // native start/stop/reconfigure operations. Without this guard an old
        // watcher could validate its generation, pause, then tear down a newer
        // session's listener after that session starts.
        let _operation_guard = self.vpn_operation_lock.lock().await;
        if !matches!(
            self.vpn.lifecycle(),
            NativeVpnLifecycle::Failed {
                generation: active,
                ..
            } if active == generation
        ) {
            return;
        }
        let _ = self.stop_mixed_listener();
        let Ok(mut state) = self.lock_state() else {
            return;
        };
        if state.platform_start_attempt_id != attempt_id
            || matches!(
                state.platform_start_outcome,
                PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
            )
        {
            return;
        }
        apply_platform_failure(&mut state, error.clone());
        state.logs.push(warning_log(format!(
            "native VPN worker terminated for {attempt_id}: {error}"
        )));
        let _ = self.persist_platform_vpn_state_locked(&mut state);
    }

    pub async fn reload_config(&self, profile_id: &str) -> Result<(), PawsError> {
        let expected_config_revision = {
            let state = self.lock_state()?;
            state.profiles.profile(profile_id)?;
            state.config_revision
        };
        self.reload_config_at_revision(profile_id, expected_config_revision)
            .await
            .map(|_| ())
    }

    async fn reload_config_at_revision(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
    ) -> Result<ConfigProjection, PawsError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (previous_active, previous_engine_loaded) = {
            let state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            (
                state.profiles.active_profile().map(ToOwned::to_owned),
                state.engine_loaded,
            )
        };
        if let Err(primary) = self.reload_config_inner(profile_id).await {
            if previous_engine_loaded && previous_active.as_deref() != Some(profile_id) {
                if let Some(previous_profile_id) = previous_active {
                    return match self.reload_config_inner(&previous_profile_id).await {
                        Ok(()) => Err(PawsError::Core(format!(
                            "configuration activation failed and the previous runtime was restored: {primary}"
                        ))),
                        Err(rollback) => Err(PawsError::Core(format!(
                            "configuration activation failed: {primary}; restoring the previous runtime also failed: {rollback}"
                        ))),
                    };
                }
            }
            return Err(primary);
        }
        let mut state = self.lock_state()?;
        if previous_active.as_deref() == Some(profile_id) {
            self.publish_status_and_resource_change_locked(&mut state);
        } else {
            self.publish_config_and_resource_change_locked(&mut state);
        }
        Ok(Self::config_projection_locked(&state))
    }

    pub async fn sync_external_controller_config(&self) -> Result<bool, PawsError> {
        // Keep the same lock order as platform startup (VPN operation first,
        // then config reload) before this path may restart the native worker.
        let _vpn_operation_guard = self.vpn_operation_lock.lock().await;
        let _reload_guard = self.config_reload_lock.lock().await;
        let pending = {
            let mut state = self.lock_state()?;
            sync_live_controller_route(&mut state)?;
            let Some(controller) = state.api_controller.as_ref() else {
                return Ok(false);
            };
            let revision = controller.config_revision.load(Ordering::Acquire);
            if revision == controller.synced_revision {
                return Ok(false);
            }
            let current = controller.raw_config.read().clone();
            let baseline = controller.baseline_raw_config.clone();
            if raw_configs_equal(&baseline, &current)? {
                if let Some(controller) = state.api_controller.as_mut() {
                    controller.synced_revision = revision;
                }
                return Ok(false);
            }
            let profile_id = state
                .profiles
                .active_profile()
                .map(ToOwned::to_owned)
                .ok_or_else(|| PawsError::ProfileNotFound("<active>".to_owned()))?;
            let previous_yaml = state.profiles.raw_yaml(&profile_id)?;
            let checkpoint = state.profiles.checkpoint_profile(&profile_id)?;
            let merged_yaml = merge_external_raw_config(&previous_yaml, &baseline, &current)?;
            let tunnel_mode = state
                .tunnel
                .as_ref()
                .map(Tunnel::mode)
                .map(mode_from_tunnel)
                .unwrap_or(state.mode);
            state.mode = tunnel_mode;
            if let Some(stats) = self.vpn.stats().as_ref() {
                settle_traffic_before_profile_switch(&mut state, Some(stats))?;
            }
            state
                .profiles
                .update_profile_content(&profile_id, merged_yaml)?;
            Some((profile_id, checkpoint, self.vpn.fd()))
        };
        let Some((profile_id, checkpoint, running_fd)) = pending else {
            return Ok(false);
        };

        let reload_result = self.reload_config_inner(&profile_id).await;
        if let Err(primary) = reload_result {
            let (error, rollback_succeeded) = self
                .rollback_profile_after_failure(&profile_id, checkpoint, primary)
                .await;
            let mut state = self.lock_state()?;
            state.last_controller_config_sync_error = Some(error.to_string());
            state.logs.push(warning_log(format!(
                "external-controller config sync failed: {error}"
            )));
            state.controller_diagnostics = sample_controller_diagnostics(&state);
            if rollback_succeeded {
                self.publish_runtime_change_locked(&mut state, false, true);
            } else {
                self.publish_config_and_resource_change_locked(&mut state);
            }
            return Err(error);
        }

        if let Some(fd) = running_fd {
            let (tunnel, sniffer_config, vpn_options) = {
                let state = self.lock_state()?;
                (
                    state.tunnel.clone().ok_or_else(|| {
                        PawsError::Core("meow tunnel is not loaded after config sync".to_owned())
                    })?,
                    state.sniffer_config.clone(),
                    state.vpn_options.clone(),
                )
            };
            if let Err(error) = self
                .vpn
                .start(fd, vpn_options, tunnel, sniffer_config)
                .await
            {
                let (mut error, rollback_succeeded) = self
                    .rollback_profile_after_failure(&profile_id, checkpoint, error)
                    .await;
                let native_rollback_succeeded = if rollback_succeeded {
                    let previous_runtime = {
                        let state = self.lock_state()?;
                        state.tunnel.clone().map(|tunnel| {
                            (
                                tunnel,
                                state.sniffer_config.clone(),
                                state.vpn_options.clone(),
                            )
                        })
                    };
                    match previous_runtime {
                        Some((tunnel, sniffer_config, vpn_options)) => self
                            .vpn
                            .start(fd, vpn_options, tunnel, sniffer_config)
                            .await
                            .map(|_| true)
                            .map_err(|rollback| {
                                PawsError::Core(format!(
                                    "{error}; restarting the previous VPN runtime also failed: {rollback}"
                                ))
                            }),
                        None => Err(PawsError::Core(format!(
                            "{error}; restarting the previous VPN runtime also failed: the restored tunnel is not loaded"
                        ))),
                    }
                } else {
                    Ok(false)
                };
                let native_rollback_succeeded = match native_rollback_succeeded {
                    Ok(restored) => {
                        if restored {
                            error = PawsError::Core(format!(
                                "external-controller config synchronization failed and the previous profile and VPN runtime were restored: {error}"
                            ));
                        }
                        restored
                    }
                    Err(rollback) => {
                        error = rollback;
                        false
                    }
                };
                let mut state = self.lock_state()?;
                state.last_controller_config_sync_error = Some(error.to_string());
                state.logs.push(warning_log(format!(
                    "external-controller config synchronization failed: {error}"
                )));
                state.controller_diagnostics = sample_controller_diagnostics(&state);
                if native_rollback_succeeded {
                    self.publish_runtime_change_locked(&mut state, false, true);
                } else if rollback_succeeded {
                    self.publish_status_and_telemetry_change_locked(&mut state);
                } else {
                    self.publish_config_and_resource_change_locked(&mut state);
                }
                return Err(error);
            }
        }

        let mut state = self.lock_state()?;
        state.controller_config_sync_count = state.controller_config_sync_count.saturating_add(1);
        state.last_controller_config_sync_at = Some(unix_timestamp_string());
        state.last_controller_config_sync_error = None;
        state.logs.push(info_log(format!(
            "external-controller config synchronized to profile {profile_id}"
        )));
        state.controller_diagnostics = sample_controller_diagnostics(&state);
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(true)
    }

    async fn reload_config_inner(&self, profile_id: &str) -> Result<(), PawsError> {
        #[cfg(test)]
        if self.fail_next_config_reload.swap(false, Ordering::AcqRel) {
            return Err(PawsError::Core(
                "injected configuration reload failure".to_owned(),
            ));
        }
        let reload_started = Instant::now();
        let tun_stats = self.vpn.stats();
        let (
            runtime_yaml,
            runtime_path,
            mode,
            vpn_options,
            controller_access,
            network_ports,
            selected_proxies,
            preserve_existing_order,
            expected_config_revision,
        ) = {
            let state = self.lock_state()?;
            let same_profile = state.profiles.active_profile() == Some(profile_id);
            let preserve_existing_order = same_profile && !state.proxy_groups.is_empty();
            let vpn_options = state.profiles.vpn_options_for_profile(profile_id)?;
            let controller_access = state.profiles.controller_access_for_profile(profile_id)?;
            let network_ports = state.profiles.network_ports_for_profile(profile_id)?;
            let runtime_yaml =
                state
                    .profiles
                    .render_runtime_yaml(profile_id, state.mode, &vpn_options)?;
            let runtime_path = state.profiles.runtime_yaml_path(profile_id);
            let selected_proxies = state.profiles.selected_proxies(profile_id)?;
            (
                runtime_yaml,
                runtime_path,
                state.mode,
                vpn_options,
                controller_access,
                network_ports,
                selected_proxies,
                preserve_existing_order,
                state.config_revision,
            )
        };
        let yaml_ready = Instant::now();

        let config = load_meow_config_candidate(&runtime_yaml, &runtime_path).await?;
        let meow_ready = Instant::now();
        // Match Meow's mobile integration: proxy upstream hostnames use the
        // configured meow DNS (including proxy-server-nameserver) instead of
        // libc resolution, whose sockets can loop back through an active VPN.
        const VPN_PLATFORM: bool = cfg!(any(
            target_os = "android",
            target_os = "ios",
            target_env = "ohos"
        ));
        if config.dns.enabled || VPN_PLATFORM {
            meow_common::set_host_resolver(Arc::new(
                meow_dns::ResolverHostHook::new_with_proxy_resolver(
                    Arc::clone(&config.dns.resolver),
                    config.dns.proxy_resolver.clone(),
                ),
            ));
        } else {
            meow_common::clear_host_resolver();
        }
        let raw_config = config.raw.clone();
        let loaded_rule_lines = raw_config.rules.clone().unwrap_or_default();
        let proxy_provider_registry = config.proxy_providers.clone();
        let rule_provider_registry = config.rule_providers.clone();
        let listeners = config.listeners.named.clone();
        let sniffer_config = config.sniffer.clone();
        let (mut providers, editable_rules) = {
            let state = self.lock_state()?;
            (
                state.profiles.providers_from_yaml(&runtime_yaml),
                state.profiles.rules_for_profile(profile_id),
            )
        };
        let runtime_rules = runtime_rule_summaries(profile_id, &loaded_rule_lines, &editable_rules);
        let tunnel = tunnel_from_config(config, mode);
        restore_proxy_selections(&tunnel, &selected_proxies);
        let global_proxy = if mode == RuntimeMode::Global {
            ensure_global_proxy_selected(&tunnel, None)?
        } else {
            None
        };
        let mut proxy_groups = proxy_groups_from_tunnel(&tunnel);
        let runtime_ready = Instant::now();
        let previous_controller = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            state.api_controller.take()
        };
        if let Some(mut controller) = previous_controller {
            controller.shutdown().await;
        }
        let next_controller = self.start_api_controller(
            runtime_path,
            raw_config,
            proxy_provider_registry,
            rule_provider_registry,
            listeners,
            &tunnel,
        )?;
        let controller_bind_addr = next_controller
            .as_ref()
            .map(|controller| controller.bind_addr);
        refresh_provider_cache_metadata(&mut providers);
        if let Some(proxy_providers) = next_controller
            .as_ref()
            .map(|controller| Arc::clone(&controller.proxy_providers))
        {
            enrich_proxy_provider_members(&mut providers, &proxy_providers);
        }
        let mut state = self.lock_state()?;
        Self::ensure_config_revision_locked(&state, expected_config_revision)?;
        if preserve_existing_order {
            preserve_proxy_group_member_order(&state.proxy_groups, &mut proxy_groups);
        }
        apply_provider_refresh_states(&mut providers, &state.provider_refresh);
        if let Some(global_proxy) = global_proxy {
            state
                .profiles
                .set_selected_proxy(profile_id, "GLOBAL".to_owned(), global_proxy)?;
        }
        if state.profiles.active_profile() != Some(profile_id) {
            settle_traffic_before_profile_switch(&mut state, tun_stats.as_ref())?;
        }
        state
            .profiles
            .write_runtime_yaml(profile_id, &runtime_yaml)?;
        state.profiles.set_active(profile_id)?;
        state.api_controller = next_controller;
        state.controller_diagnostics = sample_controller_diagnostics(&state);
        state.tunnel = Some(tunnel);
        state.sniffer_config = sniffer_config;
        state.proxy_groups = proxy_groups;
        state.providers = providers;
        state.runtime_rules = runtime_rules;
        state.vpn_options = vpn_options;
        state.dns = dns_snapshot(&state.vpn_options, tun_stats.as_ref());
        state.controller_access = controller_access;
        state.network_ports = network_ports;
        state.engine_loaded = true;
        state.geodata = state.profiles.geodata_files();
        state.last_meow_traffic_sample = None;
        invalidate_exit_location(&mut state);
        persist_runtime_ui_cache_best_effort(&mut state);
        state.logs.push(info_log(format!(
            "config reloaded from profile {profile_id} in {} ms (YAML {} ms, meow {} ms, runtime {} ms; {} bytes)",
            reload_started.elapsed().as_millis(),
            yaml_ready.duration_since(reload_started).as_millis(),
            meow_ready.duration_since(yaml_ready).as_millis(),
            runtime_ready.duration_since(meow_ready).as_millis(),
            runtime_yaml.len(),
        )));
        if let Some(addr) = controller_bind_addr {
            state.logs.push(info_log(format!(
                "meow external-controller listening on {addr}"
            )));
        }
        Ok(())
    }

    pub fn set_mode(&self, mode: RuntimeMode) -> Result<(), PawsError> {
        let _reload_guard = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        if mode == RuntimeMode::Global && state.tunnel.is_none() {
            return Err(PawsError::Core(
                "Global mode requires an active profile with at least one proxy node".to_owned(),
            ));
        }
        let global_proxy = if mode == RuntimeMode::Global {
            apply_global_proxy_policy(&mut state, None, true)?
        } else {
            None
        };
        self.persist_platform_vpn_control_locked(&mut state, mode, global_proxy)?;
        if state.mode != mode {
            invalidate_exit_location(&mut state);
        }
        state.mode = mode;
        if let Some(tunnel) = &state.tunnel {
            tunnel.set_mode(mode_to_tunnel(mode));
        }
        state
            .logs
            .push(info_log(format!("mode switched to {}", mode.as_str())));
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(())
    }

    pub async fn select_proxy(&self, group_name: &str, proxy_name: &str) -> Result<(), PawsError> {
        self.select_proxy_at_revision(group_name, proxy_name, None)
            .await
    }

    async fn select_proxy_at_revision(
        &self,
        group_name: &str,
        proxy_name: &str,
        expected_config_revision: Option<u64>,
    ) -> Result<(), PawsError> {
        let needs_prepare = {
            let state = self.lock_state()?;
            if let Some(expected) = expected_config_revision {
                Self::ensure_config_revision_locked(&state, expected)?;
            }
            state.tunnel.is_none()
        };
        if needs_prepare {
            self.prepare_active_vpn().await?;
        }
        let (tunnel, operation_config_revision) = {
            let state = self.lock_state()?;
            if let Some(expected) = expected_config_revision {
                Self::ensure_config_revision_locked(&state, expected)?;
            }
            (state.tunnel.clone(), state.config_revision)
        };
        let Some(tunnel) = tunnel else {
            return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
        };
        let route = tunnel.route_snapshot();
        let proxies = &route.proxies;
        let Some(group) = proxies.get(group_name) else {
            return Err(PawsError::Core(format!(
                "proxy group not found: {group_name}"
            )));
        };
        let selection = group
            .selection()
            .ok_or_else(|| PawsError::Core(format!("{group_name} is not selectable")))?;
        selection.set(proxy_name).await.map_err(|err| {
            PawsError::Core(format!("cannot select {proxy_name} in {group_name}: {err}"))
        })?;
        self.record_proxy_selection(group_name, proxy_name, false, operation_config_revision)
    }

    pub fn unfix_proxy(&self, group_name: &str) -> Result<(), PawsError> {
        self.unfix_proxy_at_revision(group_name, None)
    }

    fn unfix_proxy_at_revision(
        &self,
        group_name: &str,
        expected_config_revision: Option<u64>,
    ) -> Result<(), PawsError> {
        let (tunnel, operation_config_revision) = {
            let state = self.lock_state()?;
            if let Some(expected) = expected_config_revision {
                Self::ensure_config_revision_locked(&state, expected)?;
            }
            (state.tunnel.clone(), state.config_revision)
        };
        let Some(tunnel) = tunnel else {
            return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
        };
        let route = tunnel.route_snapshot();
        let Some(group) = route.proxies.get(group_name) else {
            return Err(PawsError::Core(format!(
                "proxy group not found: {group_name}"
            )));
        };
        let selection = group
            .selection()
            .filter(|selection| selection.can_unfix())
            .ok_or_else(|| PawsError::Core(format!("{group_name} is not an automatic group")))?;
        selection.force_set(None);
        self.record_proxy_selection(group_name, "", false, operation_config_revision)
    }

    fn record_proxy_selection(
        &self,
        group_name: &str,
        proxy_name: &str,
        via_controller: bool,
        expected_config_revision: u64,
    ) -> Result<(), PawsError> {
        let _reload_guard = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        Self::ensure_config_revision_locked(&state, expected_config_revision)?;
        let tunnel = state
            .tunnel
            .clone()
            .ok_or_else(|| PawsError::Core("meow tunnel is not loaded".to_owned()))?;
        if let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) {
            state.profiles.set_selected_proxy(
                &profile_id,
                group_name.to_owned(),
                proxy_name.to_owned(),
            )?;
        }
        refresh_proxy_groups_preserving_order(&mut state, &tunnel);
        let global_proxy = if state.mode == RuntimeMode::Global {
            apply_global_proxy_policy(&mut state, None, true)?
        } else {
            None
        };
        let mode = state.mode;
        self.persist_platform_vpn_control_locked(&mut state, mode, global_proxy)?;
        invalidate_exit_location(&mut state);
        let source = if via_controller { " via meow API" } else { "" };
        let message = if proxy_name.is_empty() {
            format!("restored automatic selection in {group_name}{source}")
        } else {
            format!("selected {proxy_name} in {group_name}{source}")
        };
        state.logs.push(info_log(message));
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(())
    }

    pub async fn select_proxy_via_controller(
        &self,
        group_name: &str,
        proxy_name: &str,
    ) -> Result<(), PawsError> {
        let (controller, operation_config_revision) = {
            let state = self.lock_state()?;
            (controller_credentials(&state), state.config_revision)
        };
        let Some((addr, secret)) = controller else {
            return self
                .select_proxy_at_revision(group_name, proxy_name, Some(operation_config_revision))
                .await;
        };
        let url = controller_url(addr, &["proxies", group_name])?;
        let client = reqwest::Client::new();
        let mut request = client
            .put(url)
            // The controller is an in-process loopback service. If it has
            // already stopped, fail quickly and use the local selector instead
            // of leaving the UI in a pending state indefinitely.
            .timeout(std::time::Duration::from_secs(2))
            .json(&serde_json::json!({ "name": proxy_name }));
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                self.record_proxy_selection(group_name, proxy_name, true, operation_config_revision)
            }
            Ok(response) => {
                tracing::warn!(
                    group = group_name,
                    proxy = proxy_name,
                    status = %response.status(),
                    "meow API proxy selection failed, falling back to local selector"
                );
                self.select_proxy_at_revision(
                    group_name,
                    proxy_name,
                    Some(operation_config_revision),
                )
                .await
            }
            Err(err) => {
                tracing::warn!(
                    group = group_name,
                    proxy = proxy_name,
                    error = %err,
                    "meow API proxy selection failed, falling back to local selector"
                );
                self.select_proxy_at_revision(
                    group_name,
                    proxy_name,
                    Some(operation_config_revision),
                )
                .await
            }
        }
    }

    pub async fn unfix_proxy_via_controller(&self, group_name: &str) -> Result<(), PawsError> {
        let (controller, operation_config_revision) = {
            let state = self.lock_state()?;
            (controller_credentials(&state), state.config_revision)
        };
        let Some((addr, secret)) = controller else {
            let needs_prepare = {
                let state = self.lock_state()?;
                state.tunnel.is_none()
            };
            if needs_prepare {
                self.prepare_active_vpn().await?;
            }
            return self.unfix_proxy_at_revision(group_name, Some(operation_config_revision));
        };
        let url = controller_url(addr, &["proxies", group_name])?;
        let client = reqwest::Client::new();
        let mut request = client
            .delete(url)
            .timeout(std::time::Duration::from_secs(2));
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                self.record_proxy_selection(group_name, "", true, operation_config_revision)
            }
            Ok(response) => {
                tracing::warn!(
                    group = group_name,
                    status = %response.status(),
                    "meow API proxy unfix failed, falling back to local group"
                );
                self.unfix_proxy_at_revision(group_name, Some(operation_config_revision))
            }
            Err(err) => {
                tracing::warn!(
                    group = group_name,
                    error = %err,
                    "meow API proxy unfix failed, falling back to local group"
                );
                self.unfix_proxy_at_revision(group_name, Some(operation_config_revision))
            }
        }
    }

    pub async fn test_proxy_delay(
        &self,
        proxy_name: &str,
        url: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<u16, PawsError> {
        let (proxy, operation_key, operation_sequence, operation_config_revision) = {
            let mut state = self.lock_state()?;
            let Some(tunnel) = &state.tunnel else {
                return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
            };
            let proxy = tunnel
                .proxy(proxy_name)
                .ok_or_else(|| PawsError::Core(format!("proxy not found: {proxy_name}")))?;
            let operation_key = format!("proxy-delay:{proxy_name}");
            let (operation_sequence, operation_config_revision) =
                Self::begin_resource_operation_locked(&mut state, &operation_key, None)?;
            (
                proxy,
                operation_key,
                operation_sequence,
                operation_config_revision,
            )
        };
        let url = url.unwrap_or("https://www.gstatic.com/generate_204");
        let parsed = reqwest::Url::parse(url)
            .map_err(|err| PawsError::Core(format!("invalid delay test URL: {err}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| PawsError::Core("delay test URL has no host".to_owned()))?
            .to_owned();
        let port = parsed.port_or_known_default().unwrap_or(443);
        let metadata = Metadata {
            network: Network::Tcp,
            conn_type: if parsed.scheme() == "https" {
                ConnType::Https
            } else {
                ConnType::Http
            },
            dst_port: port,
            host: host.into(),
            in_name: "paws-delay".into(),
            in_port: 0,
            ..Metadata::default()
        };
        let started = Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
        let delay = match tokio::time::timeout(timeout, proxy.dial_tcp(&metadata)).await {
            Ok(Ok(_stream)) => started.elapsed().as_millis().min(u128::from(u16::MAX)) as u16,
            Ok(Err(err)) => {
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                proxy.health().record_delay(0);
                if let Some(tunnel) = state.tunnel.clone() {
                    refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                }
                self.publish_resource_change_locked(&mut state);
                return Err(PawsError::Core(format!("delay test failed: {err}")));
            }
            Err(_) => {
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                proxy.health().record_delay(0);
                if let Some(tunnel) = state.tunnel.clone() {
                    refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                }
                self.publish_resource_change_locked(&mut state);
                return Err(PawsError::Core("delay test timed out".to_owned()));
            }
        };
        let mut state = self.lock_state()?;
        Self::ensure_resource_operation_current_locked(
            &state,
            &operation_key,
            operation_sequence,
            operation_config_revision,
        )?;
        proxy.health().record_delay(delay);
        if let Some(tunnel) = state.tunnel.clone() {
            refresh_proxy_groups_preserving_order(&mut state, &tunnel);
        }
        state
            .logs
            .push(info_log(format!("{proxy_name} delay: {delay} ms")));
        self.publish_resource_change_locked(&mut state);
        Ok(delay)
    }

    pub async fn test_proxy_echo(
        &self,
        proxy_name: &str,
        url: &str,
        payload: &str,
        timeout_ms: Option<u64>,
    ) -> Result<String, PawsError> {
        let proxy = {
            let state = self.lock_state()?;
            let Some(tunnel) = &state.tunnel else {
                return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
            };
            tunnel
                .proxy(proxy_name)
                .ok_or_else(|| PawsError::Core(format!("proxy not found: {proxy_name}")))?
        };
        let metadata = proxy_test_metadata(url, "paws-echo")?;
        let payload_bytes = payload.as_bytes().to_vec();
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
        let echoed = tokio::time::timeout(timeout, async move {
            let mut stream = proxy
                .dial_tcp(&metadata)
                .await
                .map_err(|err| PawsError::Core(format!("echo test connect failed: {err}")))?;
            stream
                .write_all(&payload_bytes)
                .await
                .map_err(|err| PawsError::Core(format!("echo test write failed: {err}")))?;
            let mut echoed = vec![0_u8; payload_bytes.len()];
            stream
                .read_exact(&mut echoed)
                .await
                .map_err(|err| PawsError::Core(format!("echo test read failed: {err}")))?;
            Ok::<Vec<u8>, PawsError>(echoed)
        })
        .await
        .map_err(|_| PawsError::Core("echo test timed out".to_owned()))??;
        if echoed != payload.as_bytes() {
            return Err(PawsError::Core("echo test payload mismatch".to_owned()));
        }
        let echoed = String::from_utf8(echoed)
            .map_err(|err| PawsError::Core(format!("echo test response was not UTF-8: {err}")))?;
        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "{proxy_name} echo roundtrip: {} bytes",
            echoed.len()
        )));
        Ok(echoed)
    }

    pub async fn test_proxy_delay_via_controller(
        &self,
        proxy_name: &str,
        url: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<u16, PawsError> {
        let (controller, operation_key, operation_sequence, operation_config_revision) = {
            let mut state = self.lock_state()?;
            let operation_key = format!("proxy-delay:{proxy_name}");
            let (operation_sequence, operation_config_revision) =
                Self::begin_resource_operation_locked(&mut state, &operation_key, None)?;
            (
                controller_credentials(&state),
                operation_key,
                operation_sequence,
                operation_config_revision,
            )
        };
        let Some((addr, secret)) = controller else {
            return self.test_proxy_delay(proxy_name, url, timeout_ms).await;
        };
        let delay_url = url.unwrap_or("https://www.gstatic.com/generate_204");
        let timeout = timeout_ms.unwrap_or(5000).to_string();
        let mut controller_url = controller_url(addr, &["proxies", proxy_name, "delay"])?;
        controller_url
            .query_pairs_mut()
            .append_pair("url", delay_url)
            .append_pair("timeout", &timeout);
        let client = reqwest::Client::new();
        let mut request = client.get(controller_url);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let value: serde_json::Value = response.json().await.map_err(|err| {
                    PawsError::Core(format!("meow API delay response parse failed: {err}"))
                })?;
                let delay = value
                    .get("delay")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|delay| u16::try_from(delay).ok())
                    .ok_or_else(|| {
                        PawsError::Core("meow API delay response missing delay".to_owned())
                    })?;
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                if let Some(tunnel) = state.tunnel.clone() {
                    refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                }
                state.logs.push(info_log(format!(
                    "{proxy_name} delay: {delay} ms via meow API"
                )));
                self.publish_resource_change_locked(&mut state);
                Ok(delay)
            }
            Ok(response) => {
                tracing::warn!(
                    proxy = proxy_name,
                    status = %response.status(),
                    "meow API delay test failed, falling back to local delay"
                );
                self.test_proxy_delay(proxy_name, url, timeout_ms).await
            }
            Err(err) => {
                tracing::warn!(
                    proxy = proxy_name,
                    error = %err,
                    "meow API delay test failed, falling back to local delay"
                );
                self.test_proxy_delay(proxy_name, url, timeout_ms).await
            }
        }
    }

    pub async fn test_proxy_group_via_controller(
        &self,
        group_name: &str,
        url: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<BTreeMap<String, u16>, PawsError> {
        let (controller, operation_key, operation_sequence, operation_config_revision) = {
            let mut state = self.lock_state()?;
            let operation_key = format!("proxy-group-delay:{group_name}");
            let (operation_sequence, operation_config_revision) =
                Self::begin_resource_operation_locked(&mut state, &operation_key, None)?;
            (
                controller_credentials(&state),
                operation_key,
                operation_sequence,
                operation_config_revision,
            )
        };
        let delay_url = url.unwrap_or("https://www.gstatic.com/generate_204");
        let timeout = timeout_ms.unwrap_or(5000);
        if let Some((addr, secret)) = controller {
            let mut url = controller_url(addr, &["group", group_name, "delay"])?;
            url.query_pairs_mut()
                .append_pair("url", delay_url)
                .append_pair("timeout", &timeout.to_string());
            let client = reqwest::Client::new();
            let mut request = client.get(url);
            if let Some(secret) = secret {
                request = request.bearer_auth(secret);
            }
            match request
                .timeout(std::time::Duration::from_millis(
                    timeout.saturating_add(1000),
                ))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    let delays =
                        response
                            .json::<BTreeMap<String, u16>>()
                            .await
                            .map_err(|error| {
                                PawsError::Core(format!(
                                    "meow API group delay response parse failed: {error}"
                                ))
                            })?;
                    let mut state = self.lock_state()?;
                    Self::ensure_resource_operation_current_locked(
                        &state,
                        &operation_key,
                        operation_sequence,
                        operation_config_revision,
                    )?;
                    if let Some(tunnel) = state.tunnel.clone() {
                        refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                    }
                    let persisted_selection_changed = if state
                        .proxy_groups
                        .iter()
                        .find(|group| group.name == group_name)
                        .and_then(|group| group.fixed.as_deref())
                        == Some("")
                    {
                        if let Some(profile_id) =
                            state.profiles.active_profile().map(ToOwned::to_owned)
                        {
                            state.profiles.set_selected_proxy(
                                &profile_id,
                                group_name.to_owned(),
                                String::new(),
                            )?;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    state.logs.push(info_log(format!(
                        "group {group_name} delay tested via meow API: {} members",
                        delays.len()
                    )));
                    if persisted_selection_changed {
                        self.publish_config_and_resource_change_locked(&mut state);
                    } else {
                        self.publish_resource_change_locked(&mut state);
                    }
                    return Ok(delays);
                }
                Ok(response) => tracing::warn!(
                    group = group_name,
                    status = %response.status(),
                    "meow API group delay failed, falling back to member probes"
                ),
                Err(error) => tracing::warn!(
                    group = group_name,
                    %error,
                    "meow API group delay failed, falling back to member probes"
                ),
            }
        }

        let members = {
            let state = self.lock_state()?;
            state
                .proxy_groups
                .iter()
                .find(|group| group.name == group_name)
                .map(|group| {
                    group
                        .proxies
                        .iter()
                        .map(|proxy| proxy.name.clone())
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| PawsError::Core(format!("proxy group not found: {group_name}")))?
        };
        let mut delays = BTreeMap::new();
        for member in members {
            let delay = self
                .test_proxy_delay(&member, Some(delay_url), Some(timeout))
                .await
                .unwrap_or(0);
            delays.insert(member, delay);
        }
        let state = self.lock_state()?;
        Self::ensure_resource_operation_current_locked(
            &state,
            &operation_key,
            operation_sequence,
            operation_config_revision,
        )?;
        Ok(delays)
    }

    pub async fn flush_dns_cache_via_controller(&self) -> Result<(), PawsError> {
        self.vpn.flush_dns_cache()?;
        let (controller, tunnel) = {
            let state = self.lock_state()?;
            (controller_credentials(&state), state.tunnel.clone())
        };
        if let Some((addr, secret)) = controller {
            let client = reqwest::Client::new();
            let mut request = client.post(controller_url(addr, &["cache", "dns", "flush"])?);
            if let Some(secret) = secret {
                request = request.bearer_auth(secret);
            }
            let response = request
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await;
            if matches!(response, Ok(ref response) if response.status().is_success()) {
                let mut state = self.lock_state()?;
                state.logs.push(info_log("DNS caches flushed via meow API"));
                return Ok(());
            }
        }
        let tunnel =
            tunnel.ok_or_else(|| PawsError::Core("meow tunnel is not loaded".to_owned()))?;
        tunnel.resolver().clear_cache();
        let mut state = self.lock_state()?;
        state.logs.push(info_log("DNS caches flushed"));
        Ok(())
    }

    pub async fn flush_fake_ip_cache_via_controller(&self) -> Result<(), PawsError> {
        let (controller, tunnel) = {
            let state = self.lock_state()?;
            (controller_credentials(&state), state.tunnel.clone())
        };
        if let Some((addr, secret)) = controller {
            let client = reqwest::Client::new();
            let mut request = client.post(controller_url(addr, &["cache", "fakeip", "flush"])?);
            if let Some(secret) = secret {
                request = request.bearer_auth(secret);
            }
            let response = request
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await;
            if matches!(response, Ok(ref response) if response.status().is_success()) {
                let mut state = self.lock_state()?;
                state
                    .logs
                    .push(info_log("fake-IP cache flushed via meow API"));
                return Ok(());
            }
        }
        let tunnel =
            tunnel.ok_or_else(|| PawsError::Core("meow tunnel is not loaded".to_owned()))?;
        tunnel
            .resolver()
            .flush_fake_ip()
            .map_err(|error| PawsError::Core(format!("fake-IP cache flush failed: {error}")))?;
        let mut state = self.lock_state()?;
        state.logs.push(info_log("fake-IP cache flushed"));
        Ok(())
    }

    pub async fn healthcheck_proxy_provider_via_controller(
        &self,
        provider_name: &str,
    ) -> Result<(), PawsError> {
        let (controller, operation_key, operation_sequence, operation_config_revision) = {
            let mut state = self.lock_state()?;
            let operation_key = format!("provider-health:proxy:{provider_name}");
            let (operation_sequence, operation_config_revision) =
                Self::begin_resource_operation_locked(&mut state, &operation_key, None)?;
            (
                controller_credentials(&state),
                operation_key,
                operation_sequence,
                operation_config_revision,
            )
        };
        let (addr, secret) = controller
            .ok_or_else(|| PawsError::Core("meow external-controller is not running".to_owned()))?;
        let client = reqwest::Client::new();
        let mut request = client.get(controller_url(
            addr,
            &["providers", "proxies", provider_name, "healthcheck"],
        )?);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| PawsError::Core(format!("provider health check failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PawsError::Core(format!(
                "provider health check failed with HTTP {}",
                response.status()
            )));
        }
        let mut state = self.lock_state()?;
        Self::ensure_resource_operation_current_locked(
            &state,
            &operation_key,
            operation_sequence,
            operation_config_revision,
        )?;
        if let Some(proxy_providers) = state
            .api_controller
            .as_ref()
            .map(|controller| Arc::clone(&controller.proxy_providers))
        {
            enrich_proxy_provider_members(&mut state.providers, &proxy_providers);
        }
        state.logs.push(info_log(format!(
            "proxy provider {provider_name} health checked via meow API"
        )));
        self.publish_resource_change_locked(&mut state);
        Ok(())
    }

    pub async fn healthcheck_provider_proxy_via_controller(
        &self,
        provider_name: &str,
        proxy_name: &str,
        url: &str,
        timeout_ms: Option<u64>,
        expected_status: Option<&str>,
    ) -> Result<u16, PawsError> {
        let (controller, operation_key, operation_sequence, operation_config_revision) = {
            let mut state = self.lock_state()?;
            let operation_key = format!("provider-member-health:{provider_name}:{proxy_name}");
            let (operation_sequence, operation_config_revision) =
                Self::begin_resource_operation_locked(&mut state, &operation_key, None)?;
            (
                controller_credentials(&state),
                operation_key,
                operation_sequence,
                operation_config_revision,
            )
        };
        let (addr, secret) = controller
            .ok_or_else(|| PawsError::Core("meow external-controller is not running".to_owned()))?;
        let timeout = timeout_ms.unwrap_or(5000);
        let mut endpoint = controller_url(
            addr,
            &[
                "providers",
                "proxies",
                provider_name,
                proxy_name,
                "healthcheck",
            ],
        )?;
        endpoint
            .query_pairs_mut()
            .append_pair("url", url)
            .append_pair("timeout", &timeout.to_string());
        if let Some(expected_status) = expected_status.filter(|value| !value.is_empty()) {
            endpoint
                .query_pairs_mut()
                .append_pair("expected", expected_status);
        }
        let client = reqwest::Client::new();
        let mut request = client.get(endpoint);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request
            .timeout(std::time::Duration::from_millis(
                timeout.saturating_add(1000),
            ))
            .send()
            .await
            .map_err(|error| {
                PawsError::Core(format!("provider member health check failed: {error}"))
            })?;
        if !response.status().is_success() {
            let error = PawsError::Core(format!(
                "provider member health check failed with HTTP {}",
                response.status()
            ));
            // The controller records the failed probe in its provider registry.
            // Synchronize that result explicitly: pure snapshot reads must never
            // be responsible for discovering or publishing action outcomes.
            let mut state = self.lock_state()?;
            Self::ensure_resource_operation_current_locked(
                &state,
                &operation_key,
                operation_sequence,
                operation_config_revision,
            )?;
            if let Some(proxy_providers) = state
                .api_controller
                .as_ref()
                .map(|controller| Arc::clone(&controller.proxy_providers))
            {
                enrich_proxy_provider_members(&mut state.providers, &proxy_providers);
            }
            self.publish_resource_change_locked(&mut state);
            return Err(error);
        }
        let value = response
            .json::<serde_json::Value>()
            .await
            .map_err(|error| {
                PawsError::Core(format!(
                    "provider member health response parse failed: {error}"
                ))
            })?;
        let delay = value
            .get("delay")
            .and_then(serde_json::Value::as_u64)
            .and_then(|delay| u16::try_from(delay).ok())
            .ok_or_else(|| {
                PawsError::Core("provider member health response missing delay".to_owned())
            })?;
        let mut state = self.lock_state()?;
        Self::ensure_resource_operation_current_locked(
            &state,
            &operation_key,
            operation_sequence,
            operation_config_revision,
        )?;
        if let Some(proxy_providers) = state
            .api_controller
            .as_ref()
            .map(|controller| Arc::clone(&controller.proxy_providers))
        {
            enrich_proxy_provider_members(&mut state.providers, &proxy_providers);
        }
        state.logs.push(info_log(format!(
            "provider {provider_name}/{proxy_name} health checked via meow API: {delay} ms"
        )));
        self.publish_resource_change_locked(&mut state);
        Ok(delay)
    }

    pub async fn refresh_provider(&self, provider_name: &str) -> Result<(), PawsError> {
        self.refresh_provider_with_type(None, provider_name, None)
            .await
            .map(|_| ())
    }

    pub async fn refresh_provider_of_type(
        &self,
        provider_type: &str,
        provider_name: &str,
    ) -> Result<(), PawsError> {
        self.refresh_provider_with_type(Some(provider_type), provider_name, None)
            .await
            .map(|_| ())
    }

    pub async fn refresh_provider_checked(
        &self,
        provider_type: &str,
        provider_name: &str,
        expected_resource_revision: u64,
    ) -> Result<ResourceProjection, PawsError> {
        self.refresh_provider_with_type(
            Some(provider_type),
            provider_name,
            Some(expected_resource_revision),
        )
        .await
    }

    async fn refresh_provider_with_type(
        &self,
        requested_provider_type: Option<&str>,
        provider_name: &str,
        expected_resource_revision: Option<u64>,
    ) -> Result<ResourceProjection, PawsError> {
        let (controller, provider, active_profile, operation) = {
            let mut state = self.lock_state()?;
            let provider = state
                .providers
                .iter()
                .find(|provider| {
                    provider.name == provider_name
                        && requested_provider_type
                            .map(|provider_type| provider.provider_type == provider_type)
                            .unwrap_or(true)
                })
                .cloned();
            let operation = provider.as_ref().map(|provider| {
                let key = format!("provider:{}:{}", provider.provider_type, provider.name);
                Self::begin_resource_operation_locked(&mut state, &key, expected_resource_revision)
                    .map(|(sequence, config_revision)| (key, sequence, config_revision))
            });
            (
                controller_credentials(&state),
                provider,
                state.profiles.active_profile().map(ToOwned::to_owned),
                operation.transpose()?,
            )
        };
        let Some(provider) = provider else {
            let provider_label = requested_provider_type
                .map(|provider_type| format!("{provider_type}/{provider_name}"))
                .unwrap_or_else(|| provider_name.to_owned());
            let message = format!("provider refresh failed: provider not found: {provider_label}");
            let mut state = self.lock_state()?;
            state.logs.push(warning_log(message.clone()));
            return Err(PawsError::Core(message));
        };
        let (operation_key, operation_sequence, operation_config_revision) =
            operation.expect("provider operation exists for a resolved provider");
        let provider_type = provider.provider_type.clone();
        if provider_is_inline(&provider) {
            let message =
                format!("{provider_type} provider refresh skipped: {provider_name} is inline");
            let mut state = self.lock_state()?;
            Self::ensure_resource_operation_current_locked(
                &state,
                &operation_key,
                operation_sequence,
                operation_config_revision,
            )?;
            mark_provider_refresh(
                &mut state,
                &provider_type,
                provider_name,
                unix_timestamp_string(),
                Some(message.clone()),
            );
            state.logs.push(warning_log(message.clone()));
            self.publish_resource_change_locked(&mut state);
            return Err(PawsError::Core(message));
        }
        let Some((addr, secret)) = controller else {
            let active =
                active_profile.ok_or_else(|| PawsError::ProfileNotFound("<active>".to_owned()))?;
            {
                let state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
            }
            self.reload_config(&active).await?;
            return self.resource_projection();
        };
        let provider_collection = match provider_type.as_str() {
            "proxy" => "proxies",
            "rule" => "rules",
            other => {
                let message = format!("unknown provider type for {provider_name}: {other}");
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                mark_provider_refresh(
                    &mut state,
                    other,
                    provider_name,
                    unix_timestamp_string(),
                    Some(message.clone()),
                );
                self.publish_resource_change_locked(&mut state);
                return Err(PawsError::Core(format!(
                    "unknown provider type for {provider_name}: {other}"
                )));
            }
        };
        let url = controller_url(addr, &["providers", provider_collection, provider_name])?;
        let client = reqwest::Client::new();
        let mut request = client.put(url);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let projection = {
                    let mut state = self.lock_state()?;
                    Self::ensure_resource_operation_current_locked(
                        &state,
                        &operation_key,
                        operation_sequence,
                        operation_config_revision,
                    )?;
                    mark_provider_refresh(
                        &mut state,
                        &provider_type,
                        provider_name,
                        unix_timestamp_string(),
                        None,
                    );
                    if provider_type == "proxy" {
                        if let Some(tunnel) = state.tunnel.clone() {
                            refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                        }
                    }
                    state.logs.push(info_log(format!(
                        "{provider_type} provider refreshed via meow API: {provider_name}"
                    )));
                    self.publish_resource_change_locked(&mut state);
                    ResourceProjection {
                        revisions: runtime_revisions(&state),
                        proxy_groups: state.proxy_groups.clone(),
                        providers: state.providers.clone(),
                        geodata: state.geodata.clone(),
                    }
                };
                if provider_type == "rule" {
                    let active = active_profile
                        .ok_or_else(|| PawsError::ProfileNotFound("<active>".to_owned()))?;
                    self.reload_config(&active).await?;
                    return self.resource_projection();
                }
                Ok(projection)
            }
            Ok(response) => {
                let message = format!(
                    "{provider_type} provider refresh failed via meow API: {provider_name} ({})",
                    response.status()
                );
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                mark_provider_refresh(
                    &mut state,
                    &provider_type,
                    provider_name,
                    unix_timestamp_string(),
                    Some(message.clone()),
                );
                let stale_cache_available =
                    provider_stale_cache_available(&state, &provider_type, provider_name);
                state
                    .logs
                    .push(warning_log(provider_refresh_failure_log_message(
                        &message,
                        stale_cache_available,
                    )));
                self.publish_resource_change_locked(&mut state);
                Err(PawsError::Core(message))
            }
            Err(err) => {
                let message = format!(
                    "{provider_type} provider refresh failed via meow API: {provider_name} ({err})"
                );
                let mut state = self.lock_state()?;
                Self::ensure_resource_operation_current_locked(
                    &state,
                    &operation_key,
                    operation_sequence,
                    operation_config_revision,
                )?;
                mark_provider_refresh(
                    &mut state,
                    &provider_type,
                    provider_name,
                    unix_timestamp_string(),
                    Some(message.clone()),
                );
                let stale_cache_available =
                    provider_stale_cache_available(&state, &provider_type, provider_name);
                state
                    .logs
                    .push(warning_log(provider_refresh_failure_log_message(
                        &message,
                        stale_cache_available,
                    )));
                self.publish_resource_change_locked(&mut state);
                Err(PawsError::Core(message))
            }
        }
    }

    pub async fn refresh_all_providers(&self) -> Result<(), PawsError> {
        let providers = {
            let state = self.lock_state()?;
            state
                .providers
                .iter()
                .filter(|provider| !provider_is_inline(provider))
                .map(|provider| (provider.provider_type.clone(), provider.name.clone()))
                .collect::<Vec<_>>()
        };
        if providers.is_empty() {
            let mut state = self.lock_state()?;
            state.logs.push(info_log(
                "provider refresh skipped: no refreshable providers",
            ));
            return Ok(());
        }

        let total = providers.len();
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut last_error = None;
        for (provider_type, provider_name) in providers {
            match self
                .refresh_provider_of_type(&provider_type, &provider_name)
                .await
            {
                Ok(()) => succeeded += 1,
                Err(error) => {
                    failed += 1;
                    last_error = Some(error.to_string());
                }
            }
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "provider refresh all finished: {succeeded} succeeded, {failed} failed"
        )));
        if succeeded == 0 {
            return Err(PawsError::Core(format!(
                "all {total} provider refreshes failed: {}",
                last_error.unwrap_or_else(|| "unknown error".to_owned())
            )));
        }
        Ok(())
    }

    pub async fn update_profile_content(
        &self,
        profile_id: &str,
        raw_yaml: &str,
    ) -> Result<(), PawsError> {
        let expected_config_revision = {
            let state = self.lock_state()?;
            state.profiles.profile(profile_id)?;
            state.config_revision
        };
        self.validate_meow_config(raw_yaml).await?;
        let raw_yaml = raw_yaml.to_owned();
        self.mutate_profile_config(
            profile_id,
            Some(expected_config_revision),
            format!("profile edited: {profile_id}"),
            move |profiles| profiles.update_profile_content(profile_id, raw_yaml),
        )
        .await
        .map(|_| ())
    }

    pub async fn update_profile_content_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        raw_yaml: &str,
    ) -> Result<(), PawsError> {
        self.validate_meow_config(raw_yaml).await?;
        let raw_yaml = raw_yaml.to_owned();
        self.mutate_profile_config(
            profile_id,
            Some(expected_config_revision),
            format!("profile edited: {profile_id}"),
            move |profiles| profiles.update_profile_content(profile_id, raw_yaml),
        )
        .await
        .map(|_| ())
    }

    pub fn update_profile_subscription(
        &self,
        profile_id: &str,
        name: &str,
        subscription_url: &str,
    ) -> Result<(), PawsError> {
        let _reload_guard = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        state
            .profiles
            .update_profile_subscription(profile_id, name, subscription_url)?;
        state.logs.push(info_log(format!(
            "profile subscription updated: {profile_id}"
        )));
        self.publish_runtime_change_locked(&mut state, true, false);
        Ok(())
    }

    pub fn update_profile_subscription_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        name: &str,
        subscription_url: &str,
    ) -> Result<ConfigProjection, PawsError> {
        let _reload_guard = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        Self::ensure_config_revision_locked(&state, expected_config_revision)?;
        state
            .profiles
            .update_profile_subscription(profile_id, name, subscription_url)?;
        state.logs.push(info_log(format!(
            "profile subscription updated: {profile_id}"
        )));
        self.publish_runtime_change_locked(&mut state, true, false);
        Ok(Self::config_projection_locked(&state))
    }

    pub async fn validate_profile_content(&self, raw_yaml: &str) -> Result<(), PawsError> {
        paws_profile::validate_profile_app_config(raw_yaml)?;
        self.validate_meow_config(raw_yaml).await
    }

    async fn validate_meow_config(&self, raw_yaml: &str) -> Result<(), PawsError> {
        let store_root = {
            let state = self.lock_state()?;
            state.profiles.root().to_path_buf()
        };
        validate_meow_config(raw_yaml, &store_root).await
    }

    pub fn profile_raw_yaml(&self, profile_id: &str) -> Result<String, PawsError> {
        let state = self.lock_state()?;
        state.profiles.raw_yaml(profile_id)
    }

    pub async fn restore_profile_backup(&self, profile_id: &str) -> Result<(), PawsError> {
        self.mutate_profile_config(
            profile_id,
            None,
            format!("profile restored from backup: {profile_id}"),
            move |profiles| profiles.restore_profile_backup(profile_id),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_profile_dns_servers(
        &self,
        profile_id: &str,
        dns_servers: Vec<String>,
    ) -> Result<(), PawsError> {
        self.mutate_profile_config(
            profile_id,
            None,
            format!("DNS servers updated for {profile_id}"),
            move |profiles| profiles.set_profile_dns_servers(profile_id, dns_servers),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_profile_dns_config(
        &self,
        profile_id: &str,
        dns_servers: Vec<String>,
        dns_fallbacks: Vec<String>,
        dns_nameserver_policy: BTreeMap<String, Vec<String>>,
    ) -> Result<(), PawsError> {
        self.mutate_profile_config(
            profile_id,
            None,
            format!("DNS config updated for {profile_id}"),
            move |profiles| {
                profiles.set_profile_dns_config(
                    profile_id,
                    dns_servers,
                    dns_fallbacks,
                    dns_nameserver_policy,
                )
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn set_profile_dns_config_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        dns_servers: Vec<String>,
        dns_fallbacks: Vec<String>,
        dns_nameserver_policy: BTreeMap<String, Vec<String>>,
    ) -> Result<ConfigProjection, PawsError> {
        self.mutate_profile_config(
            profile_id,
            Some(expected_config_revision),
            format!("DNS config updated for {profile_id}"),
            move |profiles| {
                profiles.set_profile_dns_config(
                    profile_id,
                    dns_servers,
                    dns_fallbacks,
                    dns_nameserver_policy,
                )
            },
        )
        .await
    }

    pub async fn set_profile_vpn_config(
        &self,
        profile_id: &str,
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    ) -> Result<(), PawsError> {
        validate_supported_vpn_options(&VpnOptions {
            system_proxy,
            allow_bypass,
            ..VpnOptions::default()
        })?;
        self.mutate_profile_config(
            profile_id,
            None,
            format!("VPN config updated for {profile_id}"),
            move |profiles| {
                profiles.set_profile_vpn_config(profile_id, false, dns_hijacking, false, stack)
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn set_profile_vpn_config_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    ) -> Result<ConfigProjection, PawsError> {
        validate_supported_vpn_options(&VpnOptions {
            system_proxy,
            allow_bypass,
            ..VpnOptions::default()
        })?;
        self.mutate_profile_config(
            profile_id,
            Some(expected_config_revision),
            format!("VPN config updated for {profile_id}"),
            move |profiles| {
                profiles.set_profile_vpn_config(profile_id, false, dns_hijacking, false, stack)
            },
        )
        .await
    }

    pub async fn set_profile_network_config(
        &self,
        profile_id: &str,
        network_ports: NetworkPortConfig,
        allow_lan: bool,
    ) -> Result<(), PawsError> {
        self.mutate_profile_config(
            profile_id,
            None,
            format!(
                "network ports updated for {profile_id}: mixed {}, controller {}, LAN access {}",
                network_ports.mixed_port,
                network_ports.controller_port,
                if allow_lan { "enabled" } else { "disabled" },
            ),
            move |profiles| {
                profiles
                    .set_profile_network_config(profile_id, network_ports, allow_lan)
                    .map(|_| ())
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn set_profile_network_config_checked(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        network_ports: NetworkPortConfig,
        allow_lan: bool,
    ) -> Result<ConfigProjection, PawsError> {
        self.mutate_profile_config(
            profile_id,
            Some(expected_config_revision),
            format!(
                "network ports updated for {profile_id}: mixed {}, controller {}, LAN access {}",
                network_ports.mixed_port,
                network_ports.controller_port,
                if allow_lan { "enabled" } else { "disabled" },
            ),
            move |profiles| {
                profiles
                    .set_profile_network_config(profile_id, network_ports, allow_lan)
                    .map(|_| ())
            },
        )
        .await
    }

    async fn mutate_profile_store_config<F>(
        &self,
        profile_id: &str,
        expected_config_revision: u64,
        success_log: String,
        mutation: F,
    ) -> Result<ConfigProjection, PawsError>
    where
        F: FnOnce(&mut ProfileStore) -> Result<(), PawsError>,
    {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (previous_profiles, active) = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, expected_config_revision)?;
            state.profiles.profile(profile_id)?;
            let previous_profiles = state.profiles.clone();
            mutation(&mut state.profiles)?;
            let active = state.profiles.active_profile() == Some(profile_id);
            (previous_profiles, active)
        };

        if active {
            if let Err(primary) = self.reload_config_inner(profile_id).await {
                let rollback_write = previous_profiles.persist();
                if let Err(rollback) = rollback_write {
                    let mut state = self.lock_state()?;
                    self.publish_runtime_change_locked(&mut state, true, false);
                    return Err(PawsError::Core(format!(
                        "profile configuration update failed: {primary}; restoring the previous profile index also failed: {rollback}"
                    )));
                }
                {
                    let mut state = self.lock_state()?;
                    state.profiles = previous_profiles;
                }
                return match self.reload_config_inner(profile_id).await {
                    Ok(()) => Err(PawsError::Core(format!(
                        "profile configuration update failed and was rolled back: {primary}"
                    ))),
                    Err(rollback) => {
                        let mut state = self.lock_state()?;
                        self.publish_runtime_change_locked(&mut state, false, false);
                        Err(PawsError::Core(format!(
                            "profile configuration update failed: {primary}; restoring the previous runtime also failed: {rollback}"
                        )))
                    }
                };
            }
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(success_log));
        if active {
            self.publish_config_and_resource_change_locked(&mut state);
        } else {
            self.publish_runtime_change_locked(&mut state, true, false);
        }
        Ok(Self::config_projection_locked(&state))
    }

    async fn mutate_profile_config<F>(
        &self,
        profile_id: &str,
        expected_config_revision: Option<u64>,
        success_log: String,
        mutation: F,
    ) -> Result<ConfigProjection, PawsError>
    where
        F: FnOnce(&mut ProfileStore) -> Result<(), PawsError>,
    {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (checkpoint, active) = {
            let mut state = self.lock_state()?;
            if let Some(expected) = expected_config_revision {
                Self::ensure_config_revision_locked(&state, expected)?;
            }
            let checkpoint = state.profiles.checkpoint_profile(profile_id)?;
            mutation(&mut state.profiles)?;
            let active = state.profiles.active_profile() == Some(profile_id);
            (checkpoint, active)
        };

        if active {
            if let Err(primary) = self.reload_config_inner(profile_id).await {
                let (error, rollback_succeeded) = self
                    .rollback_profile_after_failure(profile_id, checkpoint, primary)
                    .await;
                if !rollback_succeeded {
                    let mut state = self.lock_state()?;
                    self.publish_runtime_change_locked(&mut state, true, false);
                }
                return Err(error);
            }
        }

        let mut state = self.lock_state()?;
        state.logs.push(info_log(success_log));
        if active {
            self.publish_config_and_resource_change_locked(&mut state);
        } else {
            self.publish_runtime_change_locked(&mut state, true, false);
        }
        Ok(Self::config_projection_locked(&state))
    }

    async fn rollback_profile_after_failure(
        &self,
        profile_id: &str,
        checkpoint: ProfileCheckpoint,
        primary: PawsError,
    ) -> (PawsError, bool) {
        #[cfg(test)]
        if self
            .fail_next_profile_rollback
            .swap(false, Ordering::AcqRel)
        {
            return (
                PawsError::Core(format!(
                    "profile update failed: {primary}; rollback also failed: injected profile rollback failure"
                )),
                false,
            );
        }
        let rollback_write = self
            .lock_state()
            .and_then(|mut state| state.profiles.restore_profile_checkpoint(checkpoint));
        let rollback_reload = match rollback_write {
            Ok(()) => self.reload_config_inner(profile_id).await,
            Err(error) => Err(error),
        };
        match rollback_reload {
            Ok(()) => (
                PawsError::Core(format!(
                    "profile update failed and was rolled back: {primary}"
                )),
                true,
            ),
            Err(rollback) => (
                PawsError::Core(format!(
                    "profile update failed: {primary}; rollback also failed: {rollback}"
                )),
                false,
            ),
        }
    }

    async fn rollback_profile_store_after_failure(
        &self,
        profile_id: &str,
        previous_profiles: ProfileStore,
        previous_engine_loaded: bool,
        primary: PawsError,
    ) -> (PawsError, bool) {
        let rollback_store = previous_profiles.persist().and_then(|()| {
            let mut state = self.lock_state()?;
            state.profiles = previous_profiles;
            Ok(())
        });
        let rollback_runtime = match rollback_store {
            Ok(()) if previous_engine_loaded => self.reload_config_inner(profile_id).await,
            Ok(()) => Ok(()),
            Err(error) => Err(error),
        };
        match rollback_runtime {
            Ok(()) => (
                PawsError::Core(format!("rule import failed and was rolled back: {primary}")),
                true,
            ),
            Err(rollback) => (
                PawsError::Core(format!(
                    "rule import failed: {primary}; rollback also failed: {rollback}"
                )),
                false,
            ),
        }
    }

    async fn rollback_profile_activation_after_failure(
        &self,
        checkpoint: ProfileCheckpoint,
        previous_active: Option<&str>,
        previous_engine_loaded: bool,
        primary: PawsError,
    ) -> (PawsError, bool) {
        let rollback_store = self
            .lock_state()
            .and_then(|mut state| state.profiles.restore_profile_checkpoint(checkpoint));
        let rollback_runtime = match rollback_store {
            Ok(()) if previous_engine_loaded => match previous_active {
                Some(previous_active) => self.reload_config_inner(previous_active).await,
                None => Err(PawsError::Core(
                    "the previous engine was loaded without an active profile".to_owned(),
                )),
            },
            Ok(()) => Ok(()),
            Err(error) => Err(error),
        };
        match rollback_runtime {
            Ok(()) => (
                PawsError::Core(format!(
                    "profile refresh/activation failed and was rolled back: {primary}"
                )),
                true,
            ),
            Err(rollback) => (
                PawsError::Core(format!(
                    "profile refresh/activation failed: {primary}; rollback also failed: {rollback}"
                )),
                false,
            ),
        }
    }

    async fn rollback_profile_import_after_failure(
        &self,
        profile_id: &str,
        previous_profiles: ProfileStore,
        previous_active: Option<&str>,
        previous_engine_loaded: bool,
        primary: PawsError,
    ) -> (PawsError, bool) {
        let rollback_store = self.lock_state().and_then(|mut state| {
            state
                .profiles
                .rollback_profile_import(profile_id, previous_profiles)
        });
        let rollback_runtime = match rollback_store {
            Ok(()) if previous_engine_loaded => match previous_active {
                Some(previous_active) => self.reload_config_inner(previous_active).await,
                None => Err(PawsError::Core(
                    "the previous engine was loaded without an active profile".to_owned(),
                )),
            },
            Ok(()) => Ok(()),
            Err(error) => Err(error),
        };
        match rollback_runtime {
            Ok(()) => (
                PawsError::Core(format!(
                    "profile import/activation failed and was rolled back: {primary}"
                )),
                true,
            ),
            Err(rollback) => (
                PawsError::Core(format!(
                    "profile import/activation failed: {primary}; rollback also failed: {rollback}"
                )),
                false,
            ),
        }
    }

    pub fn close_connection(&self, id: &str) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        let Some(tunnel) = &state.tunnel else {
            return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
        };
        let connection_id = id
            .parse()
            .map_err(|err| PawsError::Core(format!("invalid connection id {id}: {err}")))?;
        tunnel.statistics().close_connection(connection_id);
        state
            .logs
            .push(info_log(format!("connection closed: {id}")));
        drop(state);
        self.refresh_telemetry()?;
        Ok(())
    }

    pub async fn close_connection_via_controller(&self, id: &str) -> Result<(), PawsError> {
        let controller = {
            let state = self.lock_state()?;
            controller_credentials(&state)
        };
        let Some((addr, secret)) = controller else {
            return self.close_connection(id);
        };
        let url = controller_url(addr, &["connections", id])?;
        let client = reqwest::Client::new();
        let mut request = client.delete(url);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let mut state = self.lock_state()?;
                state
                    .logs
                    .push(info_log(format!("connection closed via meow API: {id}")));
                drop(state);
                self.refresh_telemetry()?;
                Ok(())
            }
            Ok(response) => {
                tracing::warn!(
                    connection_id = id,
                    status = %response.status(),
                    "meow API connection close failed, falling back to local close"
                );
                self.close_connection(id)
            }
            Err(err) => {
                tracing::warn!(
                    connection_id = id,
                    error = %err,
                    "meow API connection close failed, falling back to local close"
                );
                self.close_connection(id)
            }
        }
    }

    pub fn close_all_connections(&self) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        let Some(tunnel) = &state.tunnel else {
            return Err(PawsError::Core("meow tunnel is not loaded".to_owned()));
        };
        let count = tunnel.statistics().active_connection_count();
        tunnel.statistics().close_all_connections();
        state
            .logs
            .push(info_log(format!("all connections closed: {count}")));
        drop(state);
        self.refresh_telemetry()?;
        Ok(())
    }

    pub async fn close_all_connections_via_controller(&self) -> Result<(), PawsError> {
        let (controller, count) = {
            let state = self.lock_state()?;
            (
                controller_credentials(&state),
                state
                    .tunnel
                    .as_ref()
                    .map(|tunnel| tunnel.statistics().active_connection_count())
                    .unwrap_or(0),
            )
        };
        let Some((addr, secret)) = controller else {
            return self.close_all_connections();
        };
        let url = controller_url(addr, &["connections"])?;
        let client = reqwest::Client::new();
        let mut request = client.delete(url);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let mut state = self.lock_state()?;
                state.logs.push(info_log(format!(
                    "all connections closed via meow API: {count}"
                )));
                drop(state);
                self.refresh_telemetry()?;
                Ok(())
            }
            Ok(response) => {
                tracing::warn!(
                    status = %response.status(),
                    "meow API close all connections failed, falling back to local close"
                );
                self.close_all_connections()
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "meow API close all connections failed, falling back to local close"
                );
                self.close_all_connections()
            }
        }
    }

    /// Refresh the public exit IP and country through the active system VPN.
    /// Successful results are cached for five minutes; failures retry sooner
    /// without replacing a previously valid result.
    pub async fn refresh_exit_location_if_due(&self) -> Result<bool, PawsError> {
        let Ok(_refresh_guard) = self.exit_location_refresh_lock.try_lock() else {
            return Ok(false);
        };
        let (revision, mixed_port) = {
            let mut state = self.lock_state()?;
            let connected = self.vpn.is_running() || state.platform_vpn_running;
            if !connected {
                let changed = state.last_exit_location_check.is_some()
                    || state.exit_location != ExitLocationSnapshot::default();
                if changed {
                    invalidate_exit_location(&mut state);
                    self.publish_runtime_change_locked(&mut state, false, false);
                }
                return Ok(false);
            }

            let refresh_interval = if state.exit_location.ip.is_empty() {
                EXIT_LOCATION_RETRY_INTERVAL
            } else {
                EXIT_LOCATION_REFRESH_INTERVAL
            };
            if state
                .last_exit_location_check
                .is_some_and(|checked_at| checked_at.elapsed() < refresh_interval)
            {
                return Ok(false);
            }

            state.tunnel.as_ref().ok_or_else(|| {
                PawsError::Core("cannot query exit location before the tunnel is loaded".to_owned())
            })?;
            (state.exit_location_revision, state.network_ports.mixed_port)
        };

        let result = probe_exit_location(mixed_port).await;
        let mut state = self.lock_state()?;
        if state.exit_location_revision != revision
            || !(self.vpn.is_running() || state.platform_vpn_running)
        {
            return Ok(false);
        }
        state.last_exit_location_check = Some(Instant::now());
        match result {
            Ok(location) => {
                state.exit_location = location;
                let message = format!(
                    "exit location refreshed through {}: {} {}",
                    state
                        .exit_location
                        .provider
                        .as_deref()
                        .unwrap_or("unknown provider"),
                    state.exit_location.country_code,
                    state.exit_location.ip
                );
                state.logs.push(info_log(message));
            }
            Err(error) => {
                state.exit_location.error = Some(error.to_string());
                state.logs.push(warning_log(format!(
                    "exit location refresh failed: {error}"
                )));
            }
        }
        self.publish_runtime_change_locked(&mut state, false, false);
        Ok(true)
    }

    pub fn snapshot(&self) -> Result<RuntimeSnapshot, PawsError> {
        let state = self.lock_state()?;
        let native_vpn_running = self.vpn.is_running();
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let mut profiles = state.profiles.summaries();
        if let Some(profile) = profiles.iter_mut().find(|profile| profile.active) {
            profile.rule_count = state
                .runtime_rules
                .iter()
                .filter(|rule| rule.enabled)
                .count();
        }
        if let Some((profile_id, upload_bytes, download_bytes)) =
            state.platform_profile_traffic.as_ref()
        {
            if let Some(profile) = profiles
                .iter_mut()
                .find(|profile| profile.id == *profile_id)
            {
                profile.upload_bytes = *upload_bytes;
                profile.download_bytes = *download_bytes;
            }
        }
        let controller_diagnostics = state.controller_diagnostics.clone();
        let vpn_running = native_vpn_running || state.platform_vpn_running;
        Ok(RuntimeSnapshot {
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
            running: state.engine_loaded,
            vpn_running,
            network_protected: state.platform_network_protected,
            network_protect_error: state.platform_network_protect_error.clone(),
            controller_running: state.api_controller.is_some(),
            controller_addr: state
                .api_controller
                .as_ref()
                .map(|controller| controller.bind_addr.to_string()),
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

    /// Samples mutable runtime sources into the in-memory telemetry projection.
    /// `snapshot()` intentionally does not call this function, so reads never
    /// update counters, history, files, or provider metadata.
    pub fn refresh_telemetry(&self) -> Result<RuntimeRevisions, PawsError> {
        self.refresh_telemetry_internal(true)
    }

    fn refresh_telemetry_internal(
        &self,
        allow_platform_telemetry: bool,
    ) -> Result<RuntimeRevisions, PawsError> {
        let mut state = self.lock_state()?;
        // This one-second sampler is also the monotonic liveness clock for a
        // remote VPN Extension. snapshot() remains a read-only projection.
        self.sync_platform_vpn_state_locked(&mut state);
        state.logs.sync_session();
        if let Ok(mut runtime_logs) = RUNTIME_LOGS.lock() {
            runtime_logs.sync(state.logs.root());
        }
        let tun_stats = self.vpn.stats();
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let platform_telemetry = if allow_platform_telemetry
            && tun_stats.is_none()
            && platform_remote_session_is_live(&state, Instant::now())
        {
            self.read_platform_vpn_telemetry().filter(|telemetry| {
                telemetry.active_profile.as_deref() == active_profile.as_deref()
            })
        } else {
            None
        };

        if let Some(telemetry) = platform_telemetry.as_ref() {
            state.traffic = telemetry.traffic.clone();
            state.traffic_history = telemetry.traffic_history.iter().cloned().collect();
            state.dns = telemetry.dns.clone();
            state.connections = telemetry.connections.clone();
            state.request_history = telemetry.request_history.iter().cloned().rev().collect();
            state.platform_logs = Some(telemetry.logs.clone());
            state.platform_profile_traffic = telemetry.active_profile.as_ref().map(|profile_id| {
                (
                    profile_id.clone(),
                    telemetry.profile_upload_bytes,
                    telemetry.profile_download_bytes,
                )
            });
            state.last_traffic_sample = None;
            state.last_meow_traffic_sample = None;
        } else {
            state.platform_logs = None;
            state.platform_profile_traffic = None;
            if let Some(stats) = &tun_stats {
                apply_traffic_sample(&mut state, stats)?;
            } else {
                state.traffic.upload_speed = 0;
                state.traffic.download_speed = 0;
                state.traffic.tun_upload_speed = 0;
                state.traffic.tun_download_speed = 0;
                state.last_traffic_sample = None;
            }
            let connections = if let Some(tunnel) = state.tunnel.clone() {
                apply_meow_traffic_sample(&mut state, &tunnel, tun_stats.is_none())?;
                active_connections_from_tunnel(&tunnel)
            } else {
                state.traffic.meow_upload_speed = 0;
                state.traffic.meow_download_speed = 0;
                state.last_meow_traffic_sample = None;
                Vec::new()
            };
            record_request_history(&mut state, &connections);
            record_traffic_history(&mut state);
            state.connections = connections;
            state.dns = dns_snapshot(&state.vpn_options, tun_stats.as_ref());
        }

        state.controller_diagnostics = sample_controller_diagnostics(&state);
        let resources_changed = sample_runtime_resources(&mut state);

        Ok(if resources_changed {
            self.publish_telemetry_and_resource_change_locked(&mut state)
        } else {
            self.publish_runtime_change_locked(&mut state, false, true)
        })
    }

    /// Persists the VPN extension process' live counters for the UI process.
    ///
    /// HarmonyOS runs `VpnExtensionAbility` in a separate process, so the UI
    /// cannot observe the native TUN session through this process' memory.
    pub fn persist_vpn_telemetry(&self) -> Result<(), PawsError> {
        let telemetry = {
            let state = self.lock_state()?;
            platform_vpn_telemetry_projection(&state)
        };
        let Some(platform) = self.platform_ipc()? else {
            return Ok(());
        };
        platform
            .publish_telemetry(telemetry)
            .map_err(platform_ipc_error)
    }

    pub fn snapshot_json(&self) -> Result<String, PawsError> {
        to_json(&self.snapshot()?)
    }

    fn start_api_controller(
        &self,
        runtime_path: PathBuf,
        raw_config: RawConfig,
        proxy_providers: HashMap<String, Arc<ProxyProvider>>,
        rule_providers: HashMap<String, Arc<RuleProvider>>,
        listeners: Vec<NamedListener>,
        tunnel: &Tunnel,
    ) -> Result<Option<ApiControllerRuntime>, PawsError> {
        if !self.api_controller_enabled {
            return Ok(None);
        }
        if self
            .platform_ipc()?
            .is_some_and(|platform| !platform.is_ui())
        {
            return Ok(None);
        }
        let bind_addr = self
            .api_controller_addr_override
            .or_else(|| {
                raw_config
                    .external_controller
                    .as_deref()
                    .and_then(|addr| addr.parse::<SocketAddr>().ok())
            })
            .ok_or_else(|| PawsError::Core("external-controller is not configured".to_owned()))?;
        let client_addr = if bind_addr.ip().is_unspecified() {
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), bind_addr.port())
        } else {
            bind_addr
        };
        let listener = std::net::TcpListener::bind(bind_addr).map_err(|err| {
            PawsError::Core(format!(
                "failed to bind external-controller on {bind_addr}: {err}"
            ))
        })?;
        listener.set_nonblocking(true).map_err(|err| {
            PawsError::Core(format!(
                "failed to configure external-controller listener on {bind_addr}: {err}"
            ))
        })?;
        let listener = tokio::net::TcpListener::from_std(listener).map_err(|err| {
            PawsError::Core(format!(
                "failed to attach external-controller listener on {bind_addr}: {err}"
            ))
        })?;
        let (log_tx, _) = tokio::sync::broadcast::channel(256);
        if let Ok(mut senders) = API_LOG_TXS.lock() {
            senders.push_back(log_tx.clone());
            while senders.len() > MAX_API_LOG_SENDERS {
                senders.pop_front();
            }
        }
        let proxy_provider_map = dashmap::DashMap::new();
        for (name, provider) in proxy_providers {
            proxy_provider_map.insert(name, provider);
        }
        let proxy_providers = Arc::new(proxy_provider_map);
        let shared_raw_config = Arc::new(parking_lot::RwLock::new(raw_config.clone()));
        let config_revision = Arc::new(AtomicU64::new(0));
        let memory_in_use_bytes = Arc::new(AtomicU64::new(0));
        let memory_limit_bytes = Arc::new(AtomicU64::new(0));
        let app_state = Arc::new(meow_api::routes::AppState {
            tunnel: tunnel.clone(),
            secret: raw_config.secret.clone(),
            config_path: runtime_path.to_string_lossy().into_owned(),
            raw_config: Arc::clone(&shared_raw_config),
            log_tx,
            proxy_providers: Arc::clone(&proxy_providers),
            rule_providers: Arc::new(parking_lot::RwLock::new(rule_providers)),
            listeners,
            external_ui: None,
            config_mutation_lock: tokio::sync::Mutex::new(()),
        });
        let task_revision = Arc::clone(&config_revision);
        let task = tokio::spawn(async move {
            if let Err(err) = serve_api_controller(listener, app_state, task_revision).await {
                tracing::warn!("meow external-controller stopped: {err}");
            }
        });
        let memory_task = tokio::spawn(monitor_controller_memory(
            client_addr,
            raw_config.secret.clone(),
            Arc::clone(&memory_in_use_bytes),
            Arc::clone(&memory_limit_bytes),
        ));
        tracing::info!("REST API listening on {bind_addr}");
        Ok(Some(ApiControllerRuntime {
            bind_addr,
            client_addr,
            task,
            memory_task,
            raw_config: shared_raw_config,
            baseline_raw_config: raw_config,
            proxy_providers,
            config_revision,
            synced_revision: 0,
            memory_in_use_bytes,
            memory_limit_bytes,
        }))
    }

    fn persist_platform_vpn_state_locked(&self, state: &mut CoreState) -> Result<(), PawsError> {
        // SystemTime can have coarser resolution than nanoseconds on device.
        // Starting and running may otherwise receive the same timestamp, and
        // the receiver would permanently discard the terminal state because
        // platform synchronization accepts only strictly newer revisions.
        state.platform_vpn_state_updated_at =
            now_unix_nanos().max(state.platform_vpn_state_updated_at.saturating_add(1));
        self.publish_runtime_change_locked(state, false, false);
        let platform = self.platform_ipc()?;
        self.notify_platform_vpn_state_locked(state);
        #[cfg(test)]
        if self
            .fail_next_platform_vpn_publish
            .swap(false, Ordering::AcqRel)
        {
            return Err(PawsError::Core(
                "injected platform publish failure".to_owned(),
            ));
        }
        let Some(platform) = platform else {
            return Ok(());
        };
        platform
            .publish_state(platform_vpn_state(state))
            .map_err(platform_ipc_error)
    }

    fn persist_platform_vpn_control_locked(
        &self,
        state: &mut CoreState,
        mode: RuntimeMode,
        global_proxy: Option<String>,
    ) -> Result<(), PawsError> {
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let proxy_selections = match active_profile.as_deref() {
            Some(profile_id) => state.profiles.selected_proxies(profile_id)?,
            None => BTreeMap::new(),
        };
        self.write_platform_vpn_control_locked(
            state,
            PlatformVpnControl {
                mode,
                global_proxy,
                active_profile,
                proxy_selections,
                updated_at: now_unix_nanos(),
            },
        )
    }

    fn write_platform_vpn_control_locked(
        &self,
        state: &mut CoreState,
        mut control: PlatformVpnControl,
    ) -> Result<(), PawsError> {
        control.updated_at = now_unix_nanos().max(control.updated_at.saturating_add(1));
        state.platform_vpn_control_updated_at = control.updated_at;
        let Some(platform) = self.platform_ipc()? else {
            return Ok(());
        };
        platform
            .publish_control(control)
            .map_err(platform_ipc_error)
    }

    fn read_platform_vpn_telemetry(&self) -> Option<PlatformVpnTelemetry> {
        let platform = self.platform_ipc().ok().flatten()?;
        platform.read_remote().ok().flatten()?.telemetry
    }

    fn sync_platform_vpn_state_locked(&self, state: &mut CoreState) {
        let Some(platform) = self.platform_ipc().ok().flatten() else {
            return;
        };
        let envelope = match platform.read_remote() {
            Ok(Some(envelope)) => envelope,
            Ok(None) => return,
            Err(error) => {
                state.logs.push(warning_log(format!(
                    "read platform shared memory failed: {error}"
                )));
                return;
            }
        };
        let is_ui = platform.is_ui();
        self.apply_platform_envelope_locked(state, is_ui, None, envelope.state);
        if let Some(control) = envelope
            .control
            .filter(|control| control.updated_at > state.platform_vpn_control_updated_at)
        {
            self.sync_platform_vpn_control_locked(state, control);
        }
    }

    fn sync_platform_for_binding_locked(
        &self,
        state: &mut CoreState,
        attempt_id: &str,
    ) -> Result<(), PawsError> {
        let Some(platform) = self.platform_ipc()? else {
            // Keep the coordinator independently testable. Production
            // Extension calls attach shared memory immediately before bind.
            return Ok(());
        };
        if platform.is_ui() {
            return Err(PawsError::Core(
                "platform VPN start binding is only valid in the Extension".to_owned(),
            ));
        }
        let envelope = platform
            .read_remote()
            .map_err(platform_ipc_error)?
            .ok_or_else(|| PawsError::Core("platform VPN start has no UI state".to_owned()))?;
        validate_platform_start_envelope(&envelope, attempt_id)?;
        self.apply_platform_envelope_locked(state, false, Some(attempt_id), envelope.state);
        Ok(())
    }

    fn apply_platform_envelope_locked(
        &self,
        state: &mut CoreState,
        is_ui: bool,
        expected_extension_attempt_id: Option<&str>,
        remote: Option<PlatformVpnState>,
    ) {
        let Some(remote) = remote else {
            return;
        };
        let previous_projection = platform_session_projection(state);
        let mut attempt_matches = !remote.start_attempt_id.is_empty()
            && remote.start_attempt_id == state.platform_start_attempt_id;
        if is_ui && !attempt_matches {
            return;
        }
        let adopt = !is_ui
            && !attempt_matches
            && expected_extension_attempt_id
                .is_some_and(|attempt_id| remote.start_attempt_id == attempt_id)
            && !remote.start_attempt_id.is_empty();
        if !is_ui && !attempt_matches && !adopt {
            return;
        }
        if adopt {
            state.platform_start_attempt_id = remote.start_attempt_id.clone();
            state.platform_start_outcome = remote.start_outcome;
            state.platform_start_delivery_observed = remote.delivery_observed;
            state.platform_extension_attached = remote.extension_attached;
            state.platform_stop_requested = remote.stop_requested;
            state.platform_extension_owner_pid = remote.extension_owner_pid;
            state.platform_extension_owner_start_time = remote.extension_owner_start_time;
            state.platform_vpn_cleanup_complete = remote.cleanup_complete;
            attempt_matches = true;
        }

        let now = Instant::now();
        // Older frames must not refresh liveness or reapply stale state.
        let remote_advanced = remote.updated_at > state.platform_remote_state_updated_at;
        if remote_advanced {
            state.platform_remote_state_updated_at = remote.updated_at;
            state.platform_remote_state_seen_at = Some(now);
            state.platform_remote_stale_since = None;
        }
        if platform_heartbeat_watchdog_expired(
            state,
            now,
            is_ui,
            remote.running,
            attempt_matches,
            remote_advanced,
        ) {
            const MESSAGE: &str = "VPN extension heartbeat stopped";
            apply_platform_failure(state, MESSAGE.to_owned());
            state.platform_watchdog_cleanup_recoverable = true;
            state.logs.push(warning_log(MESSAGE));
            // Tell a still-alive Extension to tear down the orphaned owner.
            let _ = self.persist_platform_vpn_state_locked(state);
            return;
        }
        if !remote_advanced && !adopt {
            return;
        }

        if attempt_matches {
            // Stop intent is UI-owned and sticky for one exact attempt. The
            // Extension owns worker/process proof and publishes it back.
            state.platform_stop_requested |= remote.stop_requested;
            if is_ui {
                state.platform_extension_owner_pid = remote.extension_owner_pid;
                state.platform_extension_owner_start_time = remote.extension_owner_start_time;
            }
        }

        let was_running = state.platform_vpn_running;
        let local_terminal = matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        );
        let remote_terminal = attempt_matches
            && matches!(
                remote.start_outcome,
                PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
            );
        if remote_terminal {
            state.platform_start_outcome = remote.start_outcome;
            state.platform_vpn_starting = false;
            state.platform_vpn_running = false;
            state.platform_vpn_cleanup_complete = remote.cleanup_complete;
        } else if !local_terminal {
            state.platform_vpn_running = remote.running;
            state.platform_vpn_starting = !remote.running && remote.starting;
            // Cleanup is meaningful only after a terminal owner transition.
            // A malformed non-terminal frame cannot declare its live
            // connection destroyed and open the next-start barrier early.
            state.platform_vpn_cleanup_complete = false;
            if remote.running && state.platform_start_outcome == PlatformStartOutcome::Pending {
                state.platform_start_outcome = PlatformStartOutcome::Connected;
            }
        }
        if was_running != state.platform_vpn_running {
            invalidate_exit_location(state);
        }
        if remote_terminal {
            state.platform_extension_attached = remote.extension_attached;
        } else if attempt_matches && remote.extension_attached {
            state.platform_extension_attached = true;
        }
        if attempt_matches && remote.delivery_observed {
            state.platform_start_delivery_observed = true;
        }
        if remote_terminal || !local_terminal {
            state.platform_network_protected = remote.network_protected;
            state.platform_network_protect_error = remote.network_protect_error;
        }
        if state.platform_vpn_cleanup_complete {
            state.platform_vpn_issuer_lease = None;
            state.platform_vpn_extension_lease = None;
        }
        if platform_session_projection(state) != previous_projection {
            self.publish_runtime_change_locked(state, false, false);
        }
        self.notify_platform_vpn_state_locked(state);
    }

    fn platform_start_is_pending_locked(&self, state: &CoreState, attempt_id: &str) -> bool {
        !attempt_id.is_empty()
            && state.platform_start_attempt_id == attempt_id
            && state.platform_start_outcome == PlatformStartOutcome::Pending
    }

    fn platform_start_event_locked(&self, state: &CoreState) -> PlatformStartEvent {
        PlatformStartEvent {
            attempt_id: state.platform_start_attempt_id.clone(),
            outcome: state.platform_start_outcome,
            delivery_observed: state.platform_start_delivery_observed,
            extension_attached: state.platform_extension_attached,
            cleanup_complete: state.platform_vpn_cleanup_complete,
            error: state.platform_network_protect_error.clone(),
        }
    }

    fn notify_platform_start_locked(&self, state: &CoreState) {
        let event = self.platform_start_event_locked(state);
        let changed = {
            let current = self.platform_start_tx.borrow();
            *current != event
        };
        if changed {
            self.platform_start_tx.send_replace(event);
        }
    }

    fn notify_platform_vpn_state_locked(&self, state: &CoreState) {
        self.notify_platform_start_locked(state);
        let revision = self
            .platform_vpn_event_sequence
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.platform_vpn_event_tx.send_replace(revision);
    }

    fn sync_platform_vpn_control_locked(&self, state: &mut CoreState, control: PlatformVpnControl) {
        invalidate_exit_location(state);
        apply_platform_proxy_selections(state, &control);
        let global_proxy = if control.mode == RuntimeMode::Global {
            match apply_global_proxy_policy(state, control.global_proxy.as_deref(), false) {
                Ok(global_proxy) => global_proxy,
                Err(error) => {
                    state.logs.push(warning_log(format!(
                        "global mode synchronization rejected: {error}"
                    )));
                    let corrected = PlatformVpnControl {
                        mode: state.mode,
                        global_proxy: None,
                        updated_at: now_unix_nanos(),
                        ..control
                    };
                    let _ = self.write_platform_vpn_control_locked(state, corrected);
                    return;
                }
            }
        } else {
            None
        };
        if state.mode != control.mode {
            state.mode = control.mode;
            if let Some(tunnel) = &state.tunnel {
                tunnel.set_mode(mode_to_tunnel(control.mode));
            }
            state.logs.push(info_log(format!(
                "mode synchronized from platform control: {}",
                control.mode.as_str()
            )));
        }
        state.platform_vpn_control_updated_at = control.updated_at;
        if control.mode == RuntimeMode::Global
            && control.global_proxy.as_deref() != global_proxy.as_deref()
        {
            let mut normalized = control;
            normalized.global_proxy = global_proxy.clone();
            if let Some(global_proxy) = global_proxy {
                normalized
                    .proxy_selections
                    .insert("GLOBAL".to_owned(), global_proxy);
            }
            let _ = self.write_platform_vpn_control_locked(state, normalized);
        }
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CoreState>, PawsError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PawsError::Core("core state lock poisoned".to_owned()))?;
        if let Some(error) = state.profiles.initialization_error() {
            return Err(PawsError::Core(format!(
                "profile store initialization failed: {error}"
            )));
        }
        Ok(state)
    }
}

fn acknowledge_terminal_delivery_state(state: &mut PlatformVpnState, attempt_id: &str) -> bool {
    if attempt_id.is_empty()
        || state.start_attempt_id != attempt_id
        || !matches!(
            state.start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        )
    {
        return false;
    }
    state.delivery_observed = true;
    // Delivery is not attachment, connection, or cleanup. Preserve the
    // terminal UI owner and publish only the evidence needed to make an exact
    // OS stop safe.
    state.extension_attached = false;
    state.starting = false;
    state.running = false;
    state.network_protected = false;
    state.updated_at = now_unix_nanos().max(state.updated_at.saturating_add(1));
    true
}

fn ensure_platform_attempt_active(state: &CoreState, attempt_id: &str) -> Result<(), PawsError> {
    if attempt_id.is_empty() || state.platform_start_attempt_id != attempt_id {
        return Err(PawsError::Core(format!(
            "stale platform VPN operation for attempt {attempt_id}"
        )));
    }
    if !state.platform_extension_attached
        || matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Idle
                | PlatformStartOutcome::Failed
                | PlatformStartOutcome::Cancelled
        )
    {
        return Err(PawsError::Core(format!(
            "platform VPN attempt {attempt_id} is not active"
        )));
    }
    Ok(())
}

fn validate_app_home_path(home_dir: &Path) -> Result<(), PawsError> {
    if home_dir.as_os_str().is_empty() {
        return Err(PawsError::Core(
            "application home must not be empty".to_owned(),
        ));
    }
    if !home_dir.is_absolute() {
        return Err(PawsError::Core(format!(
            "application home must be absolute: {}",
            home_dir.display()
        )));
    }
    if home_dir.parent().is_none() {
        return Err(PawsError::Core(
            "application home must not be the filesystem root".to_owned(),
        ));
    }
    Ok(())
}

fn platform_attempt_accepts_updates(state: &CoreState, attempt_id: &str) -> bool {
    !attempt_id.is_empty()
        && state.platform_start_attempt_id == attempt_id
        && state.platform_extension_attached
        && matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Pending | PlatformStartOutcome::Connected
        )
}

#[cfg(test)]
fn platform_cleanup_recovery_base(state: &CoreState) -> bool {
    !state.platform_start_attempt_id.is_empty()
        && !state.platform_vpn_starting
        && !state.platform_vpn_running
        && !state.platform_vpn_cleanup_complete
        && matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        )
}

#[cfg(test)]
fn platform_extension_owner_identity(state: &CoreState) -> Option<(u32, u64)> {
    (state.platform_extension_owner_pid > 0 && state.platform_extension_owner_start_time > 0)
        .then_some((
            state.platform_extension_owner_pid,
            state.platform_extension_owner_start_time,
        ))
}

fn platform_owner_journal_path(state: &CoreState) -> PathBuf {
    state
        .profiles
        .root()
        .join("runtime/platform-vpn-owner.json")
}

fn platform_owner_lease_path(state: &CoreState, role: PlatformVpnOwnerLeaseRole) -> PathBuf {
    let file_name = match role {
        PlatformVpnOwnerLeaseRole::Issuer => "platform-vpn-owner.issuer.lease",
        PlatformVpnOwnerLeaseRole::Extension => "platform-vpn-owner.extension.lease",
    };
    state.profiles.root().join("runtime").join(file_name)
}

fn platform_owner_lease_record(
    attempt_id: &str,
    identity: ProcessIdentity,
    role: PlatformVpnOwnerLeaseRole,
) -> PlatformVpnOwnerLeaseRecord {
    PlatformVpnOwnerLeaseRecord {
        attempt_id: attempt_id.to_owned(),
        identity,
        role,
    }
}

#[cfg(test)]
fn can_attempt_platform_cleanup_recovery(state: &CoreState) -> bool {
    if state.platform_start_attempt_id.is_empty() || state.platform_vpn_cleanup_complete {
        return false;
    }
    if platform_cleanup_recovery_base(state)
        && state.platform_start_delivery_observed
        && !state.platform_extension_attached
    {
        return true;
    }
    let terminal = matches!(
        state.platform_start_outcome,
        PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
    );
    state.platform_extension_attached
        && (state.platform_stop_requested
            || state.platform_watchdog_cleanup_recoverable
            || (terminal && platform_extension_owner_identity(state).is_some()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupRecoveryProof {
    Proven,
    OwnerAlive,
    OwnerLivenessUnknown,
}

#[cfg(test)]
fn platform_cleanup_recovery_proof_with_owner_status(
    state: &CoreState,
    owner_status: ProcessIdentityStatus,
) -> CleanupRecoveryProof {
    if platform_cleanup_recovery_base(state)
        && state.platform_start_delivery_observed
        && !state.platform_extension_attached
    {
        return CleanupRecoveryProof::Proven;
    }
    if !state.platform_stop_requested && !state.platform_watchdog_cleanup_recoverable {
        return CleanupRecoveryProof::OwnerAlive;
    }
    let Some((pid, start_time)) = platform_extension_owner_identity(state) else {
        return CleanupRecoveryProof::OwnerLivenessUnknown;
    };
    debug_assert_ne!(pid, 0);
    debug_assert_ne!(start_time, 0);
    match owner_status {
        ProcessIdentityStatus::Dead => CleanupRecoveryProof::Proven,
        ProcessIdentityStatus::Alive => CleanupRecoveryProof::OwnerAlive,
        ProcessIdentityStatus::Unknown => CleanupRecoveryProof::OwnerLivenessUnknown,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessIdentityStatus {
    Alive,
    Dead,
    Unknown,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStartObservation {
    Found(u64),
    MissingWithProcessFsAvailable,
    Unknown,
}

fn current_process_identity() -> Result<ProcessIdentity, PawsError> {
    let pid = std::process::id();
    let start_time = read_process_start_time(pid).map_err(|error| {
        PawsError::Core(format!(
            "cannot read current process start identity for PID {pid}: {error}"
        ))
    })?;
    let boot_id = read_boot_id().map_err(|error| {
        PawsError::Core(format!("cannot read current system boot identity: {error}"))
    })?;
    if start_time == 0 || boot_id.is_empty() {
        return Err(PawsError::Core(
            "current process identity is incomplete".to_owned(),
        ));
    }
    Ok(ProcessIdentity {
        pid,
        start_time,
        boot_id,
    })
}

#[cfg(test)]
fn process_identity_status(identity: &ProcessIdentity) -> ProcessIdentityStatus {
    if identity.pid == 0 || identity.start_time == 0 || identity.boot_id.is_empty() {
        return ProcessIdentityStatus::Unknown;
    }
    let Ok(current_boot_id) = read_boot_id() else {
        return ProcessIdentityStatus::Unknown;
    };
    let observation = match read_process_start_time(identity.pid) {
        Ok(start_time) => ProcessStartObservation::Found(start_time),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if read_process_start_time(std::process::id()).is_ok() {
                ProcessStartObservation::MissingWithProcessFsAvailable
            } else {
                ProcessStartObservation::Unknown
            }
        }
        Err(_) => ProcessStartObservation::Unknown,
    };
    classify_process_identity(identity, &current_boot_id, observation)
}

#[cfg(test)]
fn classify_process_identity(
    identity: &ProcessIdentity,
    current_boot_id: &str,
    observation: ProcessStartObservation,
) -> ProcessIdentityStatus {
    if identity.pid == 0
        || identity.start_time == 0
        || identity.boot_id.is_empty()
        || current_boot_id.is_empty()
    {
        return ProcessIdentityStatus::Unknown;
    }
    if identity.boot_id != current_boot_id {
        return ProcessIdentityStatus::Dead;
    }
    match observation {
        ProcessStartObservation::Found(observed) if observed == identity.start_time => {
            ProcessIdentityStatus::Alive
        }
        ProcessStartObservation::Found(_)
        | ProcessStartObservation::MissingWithProcessFsAvailable => ProcessIdentityStatus::Dead,
        ProcessStartObservation::Unknown => ProcessIdentityStatus::Unknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn read_process_start_time(pid: u32) -> Result<u64, std::io::Error> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let command_end = stat.rfind(')').ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing process command terminator",
        )
    })?;
    // Fields after the command start at process state (field 3); starttime is
    // field 22, hence zero-based index 19 in this suffix. Pairing it with PID
    // fences cleanup recovery against PID reuse.
    stat.get(command_end + 1..)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid process stat suffix",
            )
        })?
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing process start time",
            )
        })?
        .parse()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "macos")]
fn read_process_start_time(pid: u32) -> Result<u64, std::io::Error> {
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let expected = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let observed = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            expected,
        )
    };
    if observed != expected {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("process {pid} does not exist"),
            ))
        } else {
            Err(error)
        };
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|start_time| *start_time > 0)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid zero or overflowing process start time",
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn read_boot_id() -> Result<String, std::io::Error> {
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let boot_id = boot_id.trim();
    if boot_id.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "system boot identity is empty",
        ));
    }
    Ok(boot_id.to_owned())
}

#[cfg(target_os = "macos")]
fn read_boot_id() -> Result<String, std::io::Error> {
    const NAME: &[u8] = b"kern.bootsessionuuid\0";
    let mut size = 0usize;
    let result = unsafe {
        libc::sysctlbyname(
            NAME.as_ptr().cast(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if size <= 1 || size > 128 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid system boot session UUID size",
        ));
    }
    let mut bytes = vec![0u8; size];
    let result = unsafe {
        libc::sysctlbyname(
            NAME.as_ptr().cast(),
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if size == 0 || size > bytes.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid system boot session UUID length",
        ));
    }
    bytes.truncate(size);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid system boot session UUID bytes",
        ));
    }
    let boot_id = std::str::from_utf8(&bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let valid_uuid = boot_id.len() == 36
        && boot_id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    if !valid_uuid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid system boot session UUID",
        ));
    }
    Ok(boot_id.to_owned())
}

fn apply_platform_failure(state: &mut CoreState, error: String) {
    if state.platform_vpn_running {
        invalidate_exit_location(state);
    }
    state.platform_vpn_starting = false;
    state.platform_vpn_running = false;
    state.platform_network_protected = false;
    state.platform_network_protect_error = Some(error);
    state.platform_start_outcome = PlatformStartOutcome::Failed;
    state.platform_remote_stale_since = None;
}

fn platform_session_projection(
    state: &CoreState,
) -> (
    String,
    PlatformStartOutcome,
    bool,
    bool,
    bool,
    u32,
    u64,
    bool,
    bool,
    bool,
    Option<String>,
) {
    (
        state.platform_start_attempt_id.clone(),
        state.platform_start_outcome,
        state.platform_start_delivery_observed,
        state.platform_extension_attached,
        state.platform_stop_requested,
        state.platform_extension_owner_pid,
        state.platform_extension_owner_start_time,
        state.platform_vpn_starting,
        state.platform_vpn_running,
        state.platform_network_protected,
        state.platform_network_protect_error.clone(),
    )
}

async fn load_meow_config_candidate(
    runtime_yaml: &str,
    runtime_path: &Path,
) -> Result<Config, PawsError> {
    let file_name = runtime_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("runtime.yaml");
    let candidate_path =
        runtime_path.with_file_name(format!(".{file_name}.candidate-{}", now_unix_nanos()));
    fs::write(&candidate_path, runtime_yaml).map_err(|error| {
        PawsError::Core(format!(
            "cannot stage runtime configuration {}: {error}",
            candidate_path.display()
        ))
    })?;
    let result = load_meow_config_from_path(runtime_yaml, &candidate_path).await;
    let cleanup = fs::remove_file(&candidate_path);
    match (result, cleanup) {
        (Ok(config), _) => Ok(config),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(PawsError::Core(format!(
            "runtime configuration validation failed: {primary}; candidate cleanup also failed: {cleanup}"
        ))),
    }
}

fn validate_platform_start_envelope(
    envelope: &platform_ipc::PlatformEnvelope,
    attempt_id: &str,
) -> Result<(), PawsError> {
    if attempt_id.is_empty() {
        return Err(PawsError::Core(
            "platform VPN start attempt id is empty".to_owned(),
        ));
    }
    let remote = envelope
        .state
        .as_ref()
        .ok_or_else(|| PawsError::Core("platform VPN start has no UI state".to_owned()))?;
    if remote.start_attempt_id != attempt_id {
        return Err(PawsError::Core(format!(
            "stale platform VPN start attempt {attempt_id}; UI owns {}",
            remote.start_attempt_id
        )));
    }
    if !matches!(
        remote.start_outcome,
        PlatformStartOutcome::Pending | PlatformStartOutcome::Connected
    ) {
        return Err(PawsError::Core(format!(
            "platform VPN start attempt {attempt_id} is not active"
        )));
    }
    Ok(())
}

fn platform_heartbeat_watchdog_expired(
    state: &mut CoreState,
    now: Instant,
    is_ui: bool,
    remote_running: bool,
    attempt_matches: bool,
    remote_advanced: bool,
) -> bool {
    let heartbeat_frozen = is_ui
        && state.platform_vpn_running
        && remote_running
        && attempt_matches
        && !remote_advanced
        && state
            .platform_remote_state_seen_at
            .is_some_and(|seen| now.duration_since(seen) >= PLATFORM_HEARTBEAT_STALE_AFTER);
    if heartbeat_frozen {
        // The second monotonic grace starts only when staleness is first
        // observed. A device waking from sleep therefore gets a complete
        // heartbeat interval to prove the Extension is alive.
        let stale_since = state.platform_remote_stale_since.get_or_insert(now);
        return now.duration_since(*stale_since) >= PLATFORM_HEARTBEAT_WAKE_GRACE;
    }
    if !remote_running || remote_advanced {
        state.platform_remote_stale_since = None;
    }
    false
}

fn platform_remote_session_is_live(state: &CoreState, now: Instant) -> bool {
    if state.platform_start_attempt_id.is_empty()
        || !state.platform_extension_attached
        || state.platform_vpn_cleanup_complete
        || !state.platform_vpn_running
        || state.platform_start_outcome != PlatformStartOutcome::Connected
    {
        return false;
    }
    let Some(last_seen) = state.platform_remote_state_seen_at else {
        return false;
    };
    if now.saturating_duration_since(last_seen) < PLATFORM_HEARTBEAT_STALE_AFTER {
        return true;
    }
    state
        .platform_remote_stale_since
        .is_some_and(|stale_since| {
            now.saturating_duration_since(stale_since) < PLATFORM_HEARTBEAT_WAKE_GRACE
        })
}

#[cfg(test)]
mod tests;
