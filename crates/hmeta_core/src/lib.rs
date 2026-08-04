use axum::extract::{Request, State as AxumState};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response as AxumResponse;
use futures::StreamExt;
use hmeta_model::{
    from_json, to_json, AboutSnapshot, ConnectionSummary, ControllerDiagnostics, DnsSnapshot,
    HMetaError, LogEntry, ManualRuleMutation, ManualRuleSpec, ProfileSummary, ProviderProxySummary,
    ProviderSummary, ProxyGroup, ProxyItem, RequestSummary, RuntimeMode, RuntimeSnapshot,
    TrafficHistoryPoint, TrafficSnapshot, VpnLifecycle, VpnOptions,
};
use hmeta_profile::{normalize_profile_content, ProfileStore};
use hmeta_vpn::{TunSession, TunStats};
use meow_common::sniffer::SnifferConfig;
use meow_common::{AdapterType, ConnType, Metadata, Network, TunnelMode};
use meow_config::{
    proxy_provider::ProxyProvider, raw::RawConfig, rule_provider::RuleProvider, Config,
    NamedListener,
};
use meow_tunnel::rule_ir::LazyMatchOutcome;
use meow_tunnel::Tunnel;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

mod controller;
mod log_recording;
mod logging;
mod platform_ipc;
mod providers;
mod routing;
mod runtime_snapshot;
mod subscription;
mod telemetry;

pub use controller::shared_core;
use controller::*;
pub use log_recording::{LogArchiveSummary, LogRecordingStatus};
use log_recording::{RecordedLogBuffer, RuntimeLogBuffer, MAX_IN_MEMORY_LOGS};
use logging::*;
use platform_ipc::PlatformIpc;
use providers::*;
use routing::*;
use runtime_snapshot::*;
use subscription::*;
use telemetry::*;

static CORE: Lazy<Arc<CoreHandle>> = Lazy::new(|| Arc::new(CoreHandle::new()));
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
const RUNTIME_UI_CACHE_FILE: &str = "runtime/ui-cache.json";
const RUNTIME_UI_CACHE_VERSION: u32 = 1;
const APP_VERSION: &str = "1.0.0";
const MEOW_RS_VERSION: &str = "0.19.0";
const ARKIT_REV: &str = "f5233b5f97c6f7f24b590470fc562d0002e4ba00";
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PlatformVpnState {
    start_attempt_id: String,
    start_outcome: PlatformStartOutcome,
    extension_attached: bool,
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
    addr: SocketAddr,
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

struct CoreState {
    engine_loaded: bool,
    platform_vpn_starting: bool,
    platform_vpn_running: bool,
    platform_start_sequence: u64,
    platform_start_attempt_id: String,
    platform_start_outcome: PlatformStartOutcome,
    platform_extension_attached: bool,
    platform_network_protected: bool,
    platform_network_protect_error: Option<String>,
    platform_vpn_state_updated_at: u128,
    platform_vpn_control_updated_at: u128,
    mode: RuntimeMode,
    profiles: ProfileStore,
    tunnel: Option<Tunnel>,
    sniffer_config: SnifferConfig,
    proxy_groups: Vec<ProxyGroup>,
    providers: Vec<ProviderSummary>,
    runtime_rules: Vec<hmeta_model::RuleSummary>,
    provider_refresh: HashMap<String, ProviderRefreshState>,
    traffic: TrafficSnapshot,
    traffic_history: VecDeque<TrafficHistoryPoint>,
    last_traffic_sample: Option<(Instant, u64, u64)>,
    last_meow_traffic_sample: Option<(Instant, u64, u64)>,
    logs: RecordedLogBuffer,
    request_history: VecDeque<RequestSummary>,
    vpn_options: VpnOptions,
    api_controller: Option<ApiControllerRuntime>,
    controller_config_sync_count: u64,
    last_controller_config_sync_at: Option<String>,
    last_controller_config_sync_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ProviderRefreshState {
    refreshed_at: String,
    error: Option<String>,
}

impl Default for CoreState {
    fn default() -> Self {
        let profiles = ProfileStore::open_default().unwrap_or_else(|_| ProfileStore::seed_empty());
        let proxy_groups = load_runtime_ui_cache(&profiles)
            .map(|cache| cache.proxy_groups)
            .unwrap_or_default();
        let logs = RecordedLogBuffer::new(profiles.root());
        Self {
            engine_loaded: false,
            platform_vpn_starting: false,
            platform_vpn_running: false,
            platform_start_sequence: 0,
            platform_start_attempt_id: String::new(),
            platform_start_outcome: PlatformStartOutcome::Idle,
            platform_extension_attached: false,
            platform_network_protected: false,
            platform_network_protect_error: None,
            platform_vpn_state_updated_at: 0,
            platform_vpn_control_updated_at: 0,
            mode: RuntimeMode::Rule,
            profiles,
            tunnel: None,
            sniffer_config: SnifferConfig::default(),
            proxy_groups,
            providers: Vec::new(),
            runtime_rules: Vec::new(),
            provider_refresh: HashMap::new(),
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
            last_traffic_sample: None,
            last_meow_traffic_sample: None,
            logs,
            request_history: VecDeque::with_capacity(MAX_REQUEST_HISTORY),
            vpn_options: VpnOptions::default(),
            api_controller: None,
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
    config_reload_lock: tokio::sync::Mutex<()>,
    vpn: TunSession,
    api_controller_enabled: bool,
    api_controller_addr_override: Option<SocketAddr>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformSharedMemoryFds {
    pub ashmem_fd: i32,
    pub notification_fd: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PlatformStartEvent {
    attempt_id: String,
    outcome: PlatformStartOutcome,
    extension_attached: bool,
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
        install_runtime_log_layer();
        let (platform_start_tx, _) = tokio::sync::watch::channel(PlatformStartEvent::default());
        Self {
            state: Mutex::new(CoreState::default()),
            platform_ipc: Mutex::new(None),
            platform_start_tx,
            config_reload_lock: tokio::sync::Mutex::new(()),
            vpn: TunSession::default(),
            api_controller_enabled: true,
            api_controller_addr_override: None,
        }
    }

    #[cfg(test)]
    fn new_with_profile_root(root: impl Into<std::path::PathBuf>) -> Self {
        install_runtime_log_layer();
        let (platform_start_tx, _) = tokio::sync::watch::channel(PlatformStartEvent::default());
        let profiles = ProfileStore::open(root).expect("test profile store");
        let log_root = profiles.root().to_path_buf();
        log_recording::set_recording_enabled(&log_root, true)
            .expect("enable log recording for core tests");
        let proxy_groups = load_runtime_ui_cache(&profiles)
            .map(|cache| cache.proxy_groups)
            .unwrap_or_default();
        Self {
            state: Mutex::new(CoreState {
                engine_loaded: false,
                platform_vpn_starting: false,
                platform_vpn_running: false,
                platform_start_sequence: 0,
                platform_start_attempt_id: String::new(),
                platform_start_outcome: PlatformStartOutcome::Idle,
                platform_extension_attached: false,
                platform_network_protected: false,
                platform_network_protect_error: None,
                platform_vpn_state_updated_at: 0,
                platform_vpn_control_updated_at: 0,
                mode: RuntimeMode::Rule,
                profiles,
                tunnel: None,
                sniffer_config: SnifferConfig::default(),
                proxy_groups,
                providers: Vec::new(),
                runtime_rules: Vec::new(),
                provider_refresh: HashMap::new(),
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
                last_traffic_sample: None,
                last_meow_traffic_sample: None,
                logs: RecordedLogBuffer::new(log_root),
                request_history: VecDeque::with_capacity(MAX_REQUEST_HISTORY),
                vpn_options: VpnOptions::default(),
                api_controller: None,
                controller_config_sync_count: 0,
                last_controller_config_sync_at: None,
                last_controller_config_sync_error: None,
            }),
            platform_ipc: Mutex::new(None),
            platform_start_tx,
            config_reload_lock: tokio::sync::Mutex::new(()),
            vpn: TunSession::default(),
            api_controller_enabled: false,
            api_controller_addr_override: None,
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
        CORE.clone()
    }

    pub fn initialize_platform_shared_memory(&self) -> Result<PlatformSharedMemoryFds, HMetaError> {
        {
            let platform = self
                .platform_ipc
                .lock()
                .map_err(|_| HMetaError::Core("platform IPC lock poisoned".to_owned()))?;
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
                .map_err(|_| HMetaError::Core("platform IPC lock poisoned".to_owned()))?;
            *slot = Some(platform);
        }
        let mut state = self.lock_state()?;
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
    ) -> Result<(), HMetaError> {
        let platform = platform_ipc::PlatformIpc::attach_vpn_raw(ashmem_fd, notification_fd)
            .map_err(platform_ipc_error)?;
        let previous = {
            let mut slot = self
                .platform_ipc
                .lock()
                .map_err(|_| HMetaError::Core("platform IPC lock poisoned".to_owned()))?;
            slot.replace(platform)
        };
        // A VPN Extension process can outlive and be reused by the UI
        // process. Always replace the old ashmem session with the descriptors
        // from the latest Want so state is published back to the current UI.
        drop(previous);
        self.sync_platform_changes()
    }

    pub fn sync_platform_changes(&self) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(())
    }

    pub async fn wait_for_platform_change(&self, timeout: Duration) -> Result<bool, HMetaError> {
        let Some(platform) = self.platform_ipc()? else {
            tokio::time::sleep(timeout).await;
            return Ok(false);
        };
        tokio::task::spawn_blocking(move || platform.wait_for_change(timeout))
            .await
            .map_err(|error| {
                HMetaError::Core(format!("platform subscription task failed: {error}"))
            })?
            .map_err(platform_ipc_error)
    }

    fn platform_ipc(&self) -> Result<Option<Arc<PlatformIpc>>, HMetaError> {
        self.platform_ipc
            .lock()
            .map(|platform| platform.clone())
            .map_err(|_| HMetaError::Core("platform IPC lock poisoned".to_owned()))
    }

    pub async fn import_profile_from_url(
        &self,
        url: &str,
        name: Option<String>,
    ) -> Result<String, HMetaError> {
        let response = reqwest::get(url)
            .await
            .map_err(|err| HMetaError::Core(format!("profile download failed: {err}")))?;
        if !response.status().is_success() {
            return Err(HMetaError::Core(format!(
                "profile download failed with HTTP {}",
                response.status()
            )));
        }
        let subscription_user_info = subscription_userinfo_from_headers(response.headers());
        let subscription_metadata = subscription_metadata_from_headers(response.headers());
        let header_name =
            subscription_profile_name_from_headers(response.headers()).or_else(|| {
                subscription_metadata
                    .as_ref()
                    .and_then(|meta| meta.title.clone())
            });
        let raw_yaml = response
            .text()
            .await
            .map_err(|err| HMetaError::Core(format!("profile body read failed: {err}")))?;
        let subscription_user_info = subscription_user_info
            .or_else(|| hmeta_profile::parse_subscription_userinfo_comment(&raw_yaml));
        let subscription_metadata = hmeta_profile::merge_subscription_metadata(
            subscription_metadata,
            hmeta_profile::parse_subscription_metadata_comment(&raw_yaml),
        );
        let name = name
            .or(header_name)
            .unwrap_or_else(|| profile_name_from_url(url));
        let raw_yaml = normalize_profile_content(&raw_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        let mut state = self.lock_state()?;
        let id = state
            .profiles
            .import_profile_content_with_subscription_metadata(
                name.clone(),
                url.to_owned(),
                raw_yaml,
                Some(url.to_owned()),
                subscription_user_info,
                subscription_metadata,
            )?;
        state
            .logs
            .push(info_log(format!("profile imported: {name}")));
        Ok(id)
    }

    pub async fn import_profile_from_content(
        &self,
        name: &str,
        source: &str,
        raw_yaml: &str,
        subscription_url: Option<String>,
    ) -> Result<String, HMetaError> {
        let subscription_user_info = hmeta_profile::parse_subscription_userinfo_comment(raw_yaml);
        let subscription_metadata = hmeta_profile::parse_subscription_metadata_comment(raw_yaml);
        let raw_yaml = normalize_profile_content(raw_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        let mut state = self.lock_state()?;
        let id = state
            .profiles
            .import_profile_content_with_subscription_metadata(
                name.to_owned(),
                source.to_owned(),
                raw_yaml,
                subscription_url,
                subscription_user_info,
                subscription_metadata,
            )?;
        state
            .logs
            .push(info_log(format!("profile imported: {name}")));
        Ok(id)
    }

    pub async fn refresh_profile(&self, profile_id: &str) -> Result<(), HMetaError> {
        let subscription_url = {
            let state = self.lock_state()?;
            state.profiles.profile(profile_id)?.subscription_url.clone()
        };
        let Some(url) = subscription_url else {
            return Err(HMetaError::Core(format!(
                "profile {profile_id} has no subscription URL"
            )));
        };
        let result = self.refresh_profile_from_url(profile_id, &url).await;
        if let Err(error) = &result {
            if let Ok(mut state) = self.lock_state() {
                let _ = state
                    .profiles
                    .mark_profile_refresh_failed(profile_id, error.to_string());
            }
        }
        result
    }

    async fn refresh_profile_from_url(
        &self,
        profile_id: &str,
        url: &str,
    ) -> Result<(), HMetaError> {
        let response = reqwest::get(url)
            .await
            .map_err(|err| HMetaError::Core(format!("profile refresh failed: {err}")))?;
        if !response.status().is_success() {
            return Err(HMetaError::Core(format!(
                "profile refresh failed with HTTP {}",
                response.status()
            )));
        }
        let subscription_user_info = subscription_userinfo_from_headers(response.headers());
        let subscription_metadata = subscription_metadata_from_headers(response.headers());
        let raw_yaml = response
            .text()
            .await
            .map_err(|err| HMetaError::Core(format!("profile body read failed: {err}")))?;
        let subscription_user_info = subscription_user_info
            .or_else(|| hmeta_profile::parse_subscription_userinfo_comment(&raw_yaml));
        let subscription_metadata = hmeta_profile::merge_subscription_metadata(
            subscription_metadata,
            hmeta_profile::parse_subscription_metadata_comment(&raw_yaml),
        );
        let raw_yaml = normalize_profile_content(&raw_yaml)?;
        self.validate_meow_config(&raw_yaml).await?;
        {
            let mut state = self.lock_state()?;
            state
                .profiles
                .replace_profile_content_with_subscription_metadata(
                    profile_id,
                    raw_yaml.clone(),
                    subscription_user_info,
                    subscription_metadata,
                )?;
            state
                .logs
                .push(info_log(format!("profile refreshed: {profile_id}")));
        }
        if self.snapshot()?.active_profile.as_deref() == Some(profile_id) {
            self.reload_config(profile_id).await?;
        }
        Ok(())
    }

    pub async fn refresh_all_profiles(&self) -> Result<(), HMetaError> {
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
            return Err(HMetaError::Core(format!(
                "all {total} subscription refreshes failed: {}",
                last_error.unwrap_or_else(|| "unknown error".to_owned())
            )));
        }
        Ok(())
    }

    pub async fn refresh_due_profiles(&self) -> Result<(), HMetaError> {
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
            return Err(HMetaError::Core(format!(
                "all {total} due subscription refreshes failed: {}",
                last_error.unwrap_or_else(|| "unknown error".to_owned())
            )));
        }
        Ok(())
    }

    pub async fn activate_profile(&self, profile_id: &str) -> Result<(), HMetaError> {
        self.reload_config(profile_id).await
    }

    pub async fn delete_profile(&self, profile_id: &str) -> Result<(), HMetaError> {
        let tun_stats = self.vpn.stats();
        let next_active = {
            let mut state = self.lock_state()?;
            let was_active = state.profiles.active_profile() == Some(profile_id);
            if was_active {
                settle_traffic_before_profile_switch(&mut state, tun_stats.as_ref())?;
            }
            state.profiles.delete_profile(profile_id)?;
            if was_active {
                state.tunnel = None;
                state.proxy_groups.clear();
                state.providers.clear();
                state.runtime_rules.clear();
                state.engine_loaded = false;
                state.api_controller = None;
            }
            state
                .logs
                .push(info_log(format!("profile deleted: {profile_id}")));
            was_active
                .then(|| state.profiles.active_profile().map(ToOwned::to_owned))
                .flatten()
        };
        if let Some(next_active) = next_active {
            self.reload_config(&next_active).await?;
        }
        Ok(())
    }

    pub fn import_rules_from_content(
        &self,
        profile_id: Option<&str>,
        source: &str,
        rules_text: &str,
    ) -> Result<Vec<String>, HMetaError> {
        let mut state = self.lock_state()?;
        let profile_id = profile_id
            .map(ToOwned::to_owned)
            .or_else(|| state.profiles.active_profile().map(ToOwned::to_owned))
            .ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
        let ids =
            state
                .profiles
                .import_rules_for_profile(&profile_id, source.to_owned(), rules_text)?;
        state.logs.push(info_log(format!(
            "imported {} rules for {profile_id}",
            ids.len()
        )));
        Ok(ids)
    }

    pub async fn apply_manual_rule(
        &self,
        profile_id: &str,
        spec: &ManualRuleSpec,
    ) -> Result<ManualRuleApplyResult, HMetaError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        let (candidate_profiles, old_runtime_yaml, runtime_yaml, mutation, mode) = {
            let state = self.lock_state()?;
            if state.profiles.active_profile() != Some(profile_id) {
                return Err(HMetaError::Core(
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
                return Err(HMetaError::Core(format!(
                    "manual rule target is not an available proxy group: {target}"
                )));
            }
        }

        let raw_config = config.raw.clone();
        let loaded_rule_lines = raw_config.rules.clone().unwrap_or_default();
        let editable_rules = candidate_profiles.rules_for_profile(profile_id);
        let runtime_rules = runtime_rule_summaries(profile_id, &loaded_rule_lines, &editable_rules);

        let mut state = self.lock_state()?;
        if state.profiles.active_profile() != Some(profile_id) {
            return Err(HMetaError::Core(
                "active profile changed while applying the manual rule".to_owned(),
            ));
        }
        candidate_profiles.write_runtime_yaml(profile_id, &runtime_yaml)?;
        if let Err(error) = candidate_profiles.persist() {
            let _ = state
                .profiles
                .write_runtime_yaml(profile_id, &old_runtime_yaml);
            return Err(error);
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

        Ok(ManualRuleApplyResult {
            mutation,
            live_updated,
            rule_mode_active: mode == RuntimeMode::Rule,
        })
    }

    pub fn set_rule_enabled(
        &self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state
            .profiles
            .set_rule_enabled(profile_id, rule_id, enabled)?;
        state.logs.push(info_log(format!(
            "rule {rule_id} {}",
            if enabled { "enabled" } else { "disabled" }
        )));
        Ok(())
    }

    pub fn reorder_rules(
        &self,
        profile_id: &str,
        ordered_rule_ids: &[String],
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state.profiles.reorder_rules(profile_id, ordered_rule_ids)?;
        state
            .logs
            .push(info_log(format!("rules reordered for {profile_id}")));
        Ok(())
    }

    pub fn delete_rule(&self, rule_id: &str) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state.profiles.delete_rule(rule_id)?;
        state
            .logs
            .push(info_log(format!("rule deleted: {rule_id}")));
        Ok(())
    }

    pub fn clear_request_history(&self) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state.request_history.clear();
        Ok(())
    }

    pub fn clear_logs(&self) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state.logs.clear();
        if let Ok(mut logs) = RUNTIME_LOGS.lock() {
            logs.clear();
        }
        Ok(())
    }

    pub fn log_recording_status(&self) -> Result<LogRecordingStatus, HMetaError> {
        let state = self.lock_state()?;
        log_recording::recording_status(state.profiles.root())
    }

    pub fn set_log_recording_enabled(
        &self,
        enabled: bool,
    ) -> Result<LogRecordingStatus, HMetaError> {
        let mut state = self.lock_state()?;
        let root = state.profiles.root().to_path_buf();
        let was_enabled = log_recording::recording_status(&root)?.enabled;
        if was_enabled == enabled {
            return log_recording::recording_status(&root);
        }

        if enabled {
            state.logs.clear();
            if let Ok(mut logs) = RUNTIME_LOGS.lock() {
                logs.clear();
            }
            log_recording::set_recording_enabled(&root, true)?;
            state.logs.sync_session();
            state.logs.push(info_log("log recording enabled"));
        } else {
            state.logs.push(info_log("log recording disabled"));
            log_recording::set_recording_enabled(&root, false)?;
            state.logs.sync_session();
            if let Ok(mut logs) = RUNTIME_LOGS.lock() {
                logs.clear();
            }
        }
        log_recording::recording_status(&root)
    }

    pub fn read_log_archive(&self, file_name: &str) -> Result<String, HMetaError> {
        let state = self.lock_state()?;
        log_recording::read_archive(state.profiles.root(), file_name)
    }

    pub fn delete_log_archive(&self, file_name: &str) -> Result<LogRecordingStatus, HMetaError> {
        let state = self.lock_state()?;
        log_recording::delete_archive(state.profiles.root(), file_name)?;
        log_recording::recording_status(state.profiles.root())
    }

    pub async fn start_vpn(&self, fd: i32, options_json: &str) -> Result<(), HMetaError> {
        let options: VpnOptions = from_json(options_json)?;
        self.prepare_active_vpn().await?;
        let (tunnel, sniffer_config) = {
            let state = self.lock_state()?;
            (state.tunnel.clone(), state.sniffer_config.clone())
        };
        let tunnel = tunnel
            .ok_or_else(|| HMetaError::Core("activate a profile before starting VPN".to_owned()))?;
        self.vpn
            .start(fd, options.clone(), tunnel, sniffer_config)?;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        state.vpn_options = options;
        state.engine_loaded = true;
        state.platform_vpn_starting = false;
        state.platform_vpn_running = true;
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            state.platform_start_outcome = PlatformStartOutcome::Connected;
        }
        state
            .logs
            .push(info_log(format!("vpn started with tun fd {fd}")));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(())
    }

    /// Ensure the active meow tunnel is ready before the platform supplies a
    /// TUN descriptor. VPN Extension can run this concurrently with native
    /// `VpnConnection::create`, removing config parsing from the serial start
    /// path. Returns `true` when a cold config load was required.
    pub async fn prepare_active_vpn(&self) -> Result<bool, HMetaError> {
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
            .ok_or_else(|| HMetaError::Core("activate a profile before starting VPN".to_owned()))?;
        self.reload_config_inner(&active_profile).await?;
        let ready = {
            let state = self.lock_state()?;
            state
                .tunnel
                .as_ref()
                .is_some_and(|tunnel| !tunnel.route_snapshot().proxies.is_empty())
        };
        if !ready {
            return Err(HMetaError::Core(
                "active meow tunnel has no proxies; reload the profile first".to_owned(),
            ));
        }
        Ok(true)
    }

    /// Evaluate a domain or IP against the active profile's compiled rules.
    ///
    /// This intentionally bypasses the tunnel's mode dispatch and statistics:
    /// the result describes Rule-mode routing without counting an inspection
    /// as real traffic.
    pub async fn lookup_rule(&self, query: &str) -> Result<RuleLookupResult, HMetaError> {
        let (query, input_kind, mut metadata) = rule_lookup_metadata(query)?;
        self.prepare_active_vpn().await?;
        let tunnel = {
            let state = self.lock_state()?;
            state.tunnel.clone()
        }
        .ok_or_else(|| HMetaError::Core("active meow tunnel is not loaded".to_owned()))?;

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

    pub fn active_vpn_options_json(&self) -> Result<String, HMetaError> {
        let state = self.lock_state()?;
        to_json(&state.profiles.active_vpn_options()?)
    }

    /// Begin one platform VPN start transaction.
    ///
    /// The system ability-start Promise is only a dispatch acknowledgement.
    /// Completion is determined by the matching VPN Extension terminal state
    /// published through shared memory.
    pub fn begin_platform_vpn_start(&self) -> Result<String, HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            return Err(HMetaError::Core(
                "platform VPN start is already pending".to_owned(),
            ));
        }
        if state.platform_vpn_running || self.vpn.is_running() {
            return Err(HMetaError::Core(
                "platform VPN is already connected".to_owned(),
            ));
        }

        state.platform_start_sequence = state.platform_start_sequence.saturating_add(1);
        let attempt_id = format!("{}-{}", now_unix_nanos(), state.platform_start_sequence);
        state.platform_start_attempt_id = attempt_id.clone();
        state.platform_start_outcome = PlatformStartOutcome::Pending;
        state.platform_extension_attached = false;
        state.platform_vpn_starting = true;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = None;
        state.logs.push(info_log(format!(
            "platform VPN start transaction {attempt_id}"
        )));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(attempt_id)
    }

    /// Bind the VPN Extension process to the transaction delivered in its Want.
    pub fn bind_platform_vpn_start(&self, attempt_id: &str) -> Result<(), HMetaError> {
        if attempt_id.is_empty() {
            return Err(HMetaError::Core(
                "platform VPN start attempt id is empty".to_owned(),
            ));
        }
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if state.platform_start_attempt_id != attempt_id {
            return Err(HMetaError::Core(format!(
                "stale platform VPN start attempt {attempt_id}"
            )));
        }
        if matches!(
            state.platform_start_outcome,
            PlatformStartOutcome::Failed | PlatformStartOutcome::Cancelled
        ) {
            return Err(HMetaError::Core(format!(
                "platform VPN start attempt {attempt_id} is already terminal"
            )));
        }
        if !state.platform_extension_attached {
            state.platform_extension_attached = true;
            state.logs.push(info_log(format!(
                "platform VPN extension attached to {attempt_id}"
            )));
            self.persist_platform_vpn_state_locked(&mut state)?;
        }
        Ok(())
    }

    /// Wait briefly for the VPN Extension to accept the matching Want.
    ///
    /// Some HarmonyOS authorization dialogs start the Extension with a new,
    /// parameter-free Want. Callers use this signal to decide whether the
    /// original Want containing the shared-memory descriptors must be sent
    /// again after authorization succeeds.
    pub async fn await_platform_vpn_start_attachment(
        &self,
        attempt_id: &str,
        timeout: Duration,
    ) -> Result<bool, HMetaError> {
        if attempt_id.is_empty() {
            return Err(HMetaError::Core(
                "platform VPN start attempt id is empty".to_owned(),
            ));
        }
        let mut receiver = self.platform_start_tx.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let event = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                self.platform_start_event_locked(&state)
            };
            if event.attempt_id != attempt_id || event.outcome != PlatformStartOutcome::Pending {
                return Ok(event.attempt_id == attempt_id && event.extension_attached);
            }
            if event.extension_attached {
                return Ok(true);
            }

            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| HMetaError::Core(
                        "platform VPN start coordinator closed".to_owned()
                    ))?;
                }
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                _ = tokio::time::sleep_until(deadline) => return Ok(false),
            }
        }
    }

    pub async fn await_platform_vpn_start(
        &self,
        attempt_id: &str,
    ) -> Result<PlatformStartOutcome, HMetaError> {
        self.await_platform_vpn_start_with_deadline(attempt_id, PLATFORM_VPN_START_DEADLINE)
            .await
    }

    async fn await_platform_vpn_start_with_deadline(
        &self,
        attempt_id: &str,
        timeout: Duration,
    ) -> Result<PlatformStartOutcome, HMetaError> {
        if attempt_id.is_empty() {
            return Err(HMetaError::Core(
                "platform VPN start attempt id is empty".to_owned(),
            ));
        }
        let mut receiver = self.platform_start_tx.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let event = {
                let mut state = self.lock_state()?;
                self.sync_platform_vpn_state_locked(&mut state);
                self.platform_start_event_locked(&state)
            };
            if event.attempt_id != attempt_id {
                return Err(HMetaError::Core(format!(
                    "platform VPN start attempt {attempt_id} was superseded"
                )));
            }
            match event.outcome {
                PlatformStartOutcome::Connected => {
                    return Ok(PlatformStartOutcome::Connected);
                }
                PlatformStartOutcome::Failed => {
                    return Err(HMetaError::Core(
                        event
                            .error
                            .unwrap_or_else(|| "VPN extension failed to start".to_owned()),
                    ));
                }
                PlatformStartOutcome::Cancelled => {
                    return Err(HMetaError::Core(
                        "VPN extension start was cancelled".to_owned(),
                    ));
                }
                PlatformStartOutcome::Idle => {
                    return Err(HMetaError::Core(format!(
                        "platform VPN start attempt {attempt_id} is not active"
                    )));
                }
                PlatformStartOutcome::Pending => {}
            }

            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| HMetaError::Core(
                        "platform VPN start coordinator closed".to_owned()
                    ))?;
                }
                changed = self.wait_for_platform_change(Duration::from_secs(1)) => {
                    changed?;
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
    ) -> Result<bool, HMetaError> {
        self.fail_platform_vpn_start_if(attempt_id, error, true)
    }

    /// Publish exactly one failure for the matching start transaction.
    pub fn fail_platform_vpn_start(
        &self,
        attempt_id: &str,
        error: String,
    ) -> Result<bool, HMetaError> {
        self.fail_platform_vpn_start_if(attempt_id, error, false)
    }

    fn fail_platform_vpn_start_if(
        &self,
        attempt_id: &str,
        error: String,
        require_unattached: bool,
    ) -> Result<bool, HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !self.platform_start_is_pending_locked(&state, attempt_id)
            || (require_unattached && state.platform_extension_attached)
        {
            return Ok(false);
        }
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = Some(error.clone());
        state.platform_start_outcome = PlatformStartOutcome::Failed;
        state.logs.push(warning_log(format!(
            "platform VPN start transaction {attempt_id} failed: {error}"
        )));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    /// Cancel only the matching pending start transaction.
    pub fn cancel_platform_vpn_start(&self, attempt_id: &str) -> Result<bool, HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !self.platform_start_is_pending_locked(&state, attempt_id) {
            return Ok(false);
        }
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = None;
        state.platform_start_outcome = PlatformStartOutcome::Cancelled;
        state.logs.push(info_log(format!(
            "platform VPN start transaction {attempt_id} cancelled"
        )));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(true)
    }

    pub fn stop_vpn(&self) -> Result<(), HMetaError> {
        let stats = self.vpn.stats();
        self.vpn.stop()?;
        let mut state = self.lock_state()?;
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
        state.platform_network_protect_error = None;
        state.traffic.upload_speed = 0;
        state.traffic.download_speed = 0;
        state.last_traffic_sample = None;
        state.logs.push(info_log("vpn stopped"));
        self.persist_platform_vpn_state_locked(&mut state)?;
        Ok(())
    }

    pub fn set_platform_vpn_starting(&self, starting: bool) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
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

    pub fn expire_platform_vpn_start(&self) -> Result<bool, HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
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

    pub fn set_platform_vpn_failed(&self, error: String) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = Some(error.clone());
        if state.platform_start_outcome == PlatformStartOutcome::Pending {
            state.platform_start_outcome = PlatformStartOutcome::Failed;
        }
        state
            .logs
            .push(warning_log(format!("platform vpn start failed: {error}")));
        self.persist_platform_vpn_state_locked(&mut state)
    }

    pub fn set_platform_vpn_running(&self, running: bool) -> Result<(), HMetaError> {
        let stats = if running { None } else { self.vpn.stats() };
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        state.platform_vpn_starting = false;
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

    pub fn set_platform_network_protected(
        &self,
        protected: bool,
        error: Option<String>,
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
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

    pub async fn reload_config(&self, profile_id: &str) -> Result<(), HMetaError> {
        let _reload_guard = self.config_reload_lock.lock().await;
        self.reload_config_inner(profile_id).await
    }

    pub async fn sync_external_controller_config(&self) -> Result<bool, HMetaError> {
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
                .ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
            let previous_yaml = state.profiles.raw_yaml(&profile_id)?;
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
            Some((profile_id, previous_yaml, self.vpn.fd()))
        };
        let Some((profile_id, previous_yaml, running_fd)) = pending else {
            return Ok(false);
        };

        let reload_result = self.reload_config_inner(&profile_id).await;
        if let Err(error) = reload_result {
            let mut state = self.lock_state()?;
            let _ = state
                .profiles
                .update_profile_content(&profile_id, previous_yaml);
            state.last_controller_config_sync_error = Some(error.to_string());
            state.logs.push(warning_log(format!(
                "external-controller config sync failed: {error}"
            )));
            return Err(error);
        }

        if let Some(fd) = running_fd {
            let (tunnel, sniffer_config, vpn_options) = {
                let state = self.lock_state()?;
                (
                    state.tunnel.clone().ok_or_else(|| {
                        HMetaError::Core("meow tunnel is not loaded after config sync".to_owned())
                    })?,
                    state.sniffer_config.clone(),
                    state.vpn_options.clone(),
                )
            };
            if let Err(error) = self.vpn.start(fd, vpn_options, tunnel, sniffer_config) {
                let mut state = self.lock_state()?;
                state.last_controller_config_sync_error = Some(error.to_string());
                state.logs.push(warning_log(format!(
                    "external-controller config synced but VPN restart failed: {error}"
                )));
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
        Ok(true)
    }

    async fn reload_config_inner(&self, profile_id: &str) -> Result<(), HMetaError> {
        let reload_started = Instant::now();
        let tun_stats = self.vpn.stats();
        let (runtime_yaml, mode, vpn_options, selected_proxies, preserve_existing_order) = {
            let mut state = self.lock_state()?;
            let same_profile = state.profiles.active_profile() == Some(profile_id);
            let preserve_existing_order = same_profile && !state.proxy_groups.is_empty();
            if !same_profile {
                settle_traffic_before_profile_switch(&mut state, tun_stats.as_ref())?;
            }
            state.profiles.set_active(profile_id)?;
            let vpn_options = state.profiles.vpn_options_for_profile(profile_id)?;
            let runtime_yaml =
                state
                    .profiles
                    .build_runtime_yaml(profile_id, state.mode, &vpn_options)?;
            let selected_proxies = state.profiles.selected_proxies(profile_id)?;
            (
                runtime_yaml,
                state.mode,
                vpn_options,
                selected_proxies,
                preserve_existing_order,
            )
        };
        let yaml_ready = Instant::now();

        let config = load_meow_config(&runtime_yaml).await?;
        let meow_ready = Instant::now();
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
        let mut state = self.lock_state()?;
        if preserve_existing_order && state.profiles.active_profile() == Some(profile_id) {
            preserve_proxy_group_member_order(&state.proxy_groups, &mut proxy_groups);
        }
        apply_provider_refresh_states(&mut providers, &state.provider_refresh);
        if let Some(global_proxy) = global_proxy {
            state
                .profiles
                .set_selected_proxy(profile_id, "GLOBAL".to_owned(), global_proxy)?;
        }
        self.restart_api_controller(
            &mut state,
            profile_id,
            raw_config,
            proxy_provider_registry,
            rule_provider_registry,
            listeners,
            &tunnel,
        )?;
        state.tunnel = Some(tunnel);
        state.sniffer_config = sniffer_config;
        state.proxy_groups = proxy_groups;
        state.providers = providers;
        state.runtime_rules = runtime_rules;
        state.vpn_options = vpn_options;
        state.engine_loaded = true;
        state.last_meow_traffic_sample = None;
        persist_runtime_ui_cache_best_effort(&mut state);
        state.logs.push(info_log(format!(
            "config reloaded from profile {profile_id} in {} ms (YAML {} ms, meow {} ms, runtime {} ms; {} bytes)",
            reload_started.elapsed().as_millis(),
            yaml_ready.duration_since(reload_started).as_millis(),
            meow_ready.duration_since(yaml_ready).as_millis(),
            runtime_ready.duration_since(meow_ready).as_millis(),
            runtime_yaml.len(),
        )));
        Ok(())
    }

    pub fn set_mode(&self, mode: RuntimeMode) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if mode == RuntimeMode::Global && state.tunnel.is_none() {
            return Err(HMetaError::Core(
                "Global mode requires an active profile with at least one proxy node".to_owned(),
            ));
        }
        let global_proxy = if mode == RuntimeMode::Global {
            apply_global_proxy_policy(&mut state, None, true)?
        } else {
            None
        };
        self.persist_platform_vpn_control_locked(&mut state, mode, global_proxy)?;
        state.mode = mode;
        if let Some(tunnel) = &state.tunnel {
            tunnel.set_mode(mode_to_tunnel(mode));
        }
        state
            .logs
            .push(info_log(format!("mode switched to {}", mode.as_str())));
        Ok(())
    }

    pub async fn select_proxy(&self, group_name: &str, proxy_name: &str) -> Result<(), HMetaError> {
        let needs_prepare = {
            let state = self.lock_state()?;
            state.tunnel.is_none()
        };
        if needs_prepare {
            self.prepare_active_vpn().await?;
        }
        let tunnel = {
            let state = self.lock_state()?;
            state.tunnel.clone()
        };
        let Some(tunnel) = tunnel else {
            return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
        };
        let route = tunnel.route_snapshot();
        let proxies = &route.proxies;
        let Some(group) = proxies.get(group_name) else {
            return Err(HMetaError::Core(format!(
                "proxy group not found: {group_name}"
            )));
        };
        let selection = group
            .selection()
            .ok_or_else(|| HMetaError::Core(format!("{group_name} is not selectable")))?;
        selection.set(proxy_name).await.map_err(|err| {
            HMetaError::Core(format!("cannot select {proxy_name} in {group_name}: {err}"))
        })?;
        self.record_proxy_selection(group_name, proxy_name, false)
    }

    pub fn unfix_proxy(&self, group_name: &str) -> Result<(), HMetaError> {
        let tunnel = {
            let state = self.lock_state()?;
            state.tunnel.clone()
        };
        let Some(tunnel) = tunnel else {
            return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
        };
        let route = tunnel.route_snapshot();
        let Some(group) = route.proxies.get(group_name) else {
            return Err(HMetaError::Core(format!(
                "proxy group not found: {group_name}"
            )));
        };
        let selection = group
            .selection()
            .filter(|selection| selection.can_unfix())
            .ok_or_else(|| HMetaError::Core(format!("{group_name} is not an automatic group")))?;
        selection.force_set(None);
        self.record_proxy_selection(group_name, "", false)
    }

    fn record_proxy_selection(
        &self,
        group_name: &str,
        proxy_name: &str,
        via_controller: bool,
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        let tunnel = state
            .tunnel
            .clone()
            .ok_or_else(|| HMetaError::Core("meow tunnel is not loaded".to_owned()))?;
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
        let source = if via_controller { " via meow API" } else { "" };
        let message = if proxy_name.is_empty() {
            format!("restored automatic selection in {group_name}{source}")
        } else {
            format!("selected {proxy_name} in {group_name}{source}")
        };
        state.logs.push(info_log(message));
        Ok(())
    }

    pub async fn select_proxy_via_controller(
        &self,
        group_name: &str,
        proxy_name: &str,
    ) -> Result<(), HMetaError> {
        let controller = {
            let state = self.lock_state()?;
            controller_credentials(&state)
        };
        let Some((addr, secret)) = controller else {
            return self.select_proxy(group_name, proxy_name).await;
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
                self.record_proxy_selection(group_name, proxy_name, true)
            }
            Ok(response) => {
                tracing::warn!(
                    group = group_name,
                    proxy = proxy_name,
                    status = %response.status(),
                    "meow API proxy selection failed, falling back to local selector"
                );
                self.select_proxy(group_name, proxy_name).await
            }
            Err(err) => {
                tracing::warn!(
                    group = group_name,
                    proxy = proxy_name,
                    error = %err,
                    "meow API proxy selection failed, falling back to local selector"
                );
                self.select_proxy(group_name, proxy_name).await
            }
        }
    }

    pub async fn unfix_proxy_via_controller(&self, group_name: &str) -> Result<(), HMetaError> {
        let controller = {
            let state = self.lock_state()?;
            controller_credentials(&state)
        };
        let Some((addr, secret)) = controller else {
            let needs_prepare = {
                let state = self.lock_state()?;
                state.tunnel.is_none()
            };
            if needs_prepare {
                self.prepare_active_vpn().await?;
            }
            return self.unfix_proxy(group_name);
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
                self.record_proxy_selection(group_name, "", true)
            }
            Ok(response) => {
                tracing::warn!(
                    group = group_name,
                    status = %response.status(),
                    "meow API proxy unfix failed, falling back to local group"
                );
                self.unfix_proxy(group_name)
            }
            Err(err) => {
                tracing::warn!(
                    group = group_name,
                    error = %err,
                    "meow API proxy unfix failed, falling back to local group"
                );
                self.unfix_proxy(group_name)
            }
        }
    }

    pub async fn test_proxy_delay(
        &self,
        proxy_name: &str,
        url: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<u16, HMetaError> {
        let proxy = {
            let state = self.lock_state()?;
            let Some(tunnel) = &state.tunnel else {
                return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
            };
            tunnel
                .proxy(proxy_name)
                .ok_or_else(|| HMetaError::Core(format!("proxy not found: {proxy_name}")))?
        };
        let url = url.unwrap_or("https://www.gstatic.com/generate_204");
        let parsed = reqwest::Url::parse(url)
            .map_err(|err| HMetaError::Core(format!("invalid delay test URL: {err}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| HMetaError::Core("delay test URL has no host".to_owned()))?
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
            in_name: "hmeta-delay".into(),
            in_port: 0,
            ..Metadata::default()
        };
        let started = Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
        let delay = match tokio::time::timeout(timeout, proxy.dial_tcp(&metadata)).await {
            Ok(Ok(_stream)) => started.elapsed().as_millis().min(u128::from(u16::MAX)) as u16,
            Ok(Err(err)) => {
                proxy.health().record_delay(0);
                return Err(HMetaError::Core(format!("delay test failed: {err}")));
            }
            Err(_) => {
                proxy.health().record_delay(0);
                return Err(HMetaError::Core("delay test timed out".to_owned()));
            }
        };
        proxy.health().record_delay(delay);
        let mut state = self.lock_state()?;
        if let Some(tunnel) = state.tunnel.clone() {
            refresh_proxy_groups_preserving_order(&mut state, &tunnel);
        }
        state
            .logs
            .push(info_log(format!("{proxy_name} delay: {delay} ms")));
        Ok(delay)
    }

    pub async fn test_proxy_echo(
        &self,
        proxy_name: &str,
        url: &str,
        payload: &str,
        timeout_ms: Option<u64>,
    ) -> Result<String, HMetaError> {
        let proxy = {
            let state = self.lock_state()?;
            let Some(tunnel) = &state.tunnel else {
                return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
            };
            tunnel
                .proxy(proxy_name)
                .ok_or_else(|| HMetaError::Core(format!("proxy not found: {proxy_name}")))?
        };
        let metadata = proxy_test_metadata(url, "hmeta-echo")?;
        let payload_bytes = payload.as_bytes().to_vec();
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
        let echoed = tokio::time::timeout(timeout, async move {
            let mut stream = proxy
                .dial_tcp(&metadata)
                .await
                .map_err(|err| HMetaError::Core(format!("echo test connect failed: {err}")))?;
            stream
                .write_all(&payload_bytes)
                .await
                .map_err(|err| HMetaError::Core(format!("echo test write failed: {err}")))?;
            let mut echoed = vec![0_u8; payload_bytes.len()];
            stream
                .read_exact(&mut echoed)
                .await
                .map_err(|err| HMetaError::Core(format!("echo test read failed: {err}")))?;
            Ok::<Vec<u8>, HMetaError>(echoed)
        })
        .await
        .map_err(|_| HMetaError::Core("echo test timed out".to_owned()))??;
        if echoed != payload.as_bytes() {
            return Err(HMetaError::Core("echo test payload mismatch".to_owned()));
        }
        let echoed = String::from_utf8(echoed)
            .map_err(|err| HMetaError::Core(format!("echo test response was not UTF-8: {err}")))?;
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
    ) -> Result<u16, HMetaError> {
        let controller_addr = {
            let state = self.lock_state()?;
            state
                .api_controller
                .as_ref()
                .map(|controller| controller.addr)
        };
        let Some(addr) = controller_addr else {
            return self.test_proxy_delay(proxy_name, url, timeout_ms).await;
        };
        let delay_url = url.unwrap_or("https://www.gstatic.com/generate_204");
        let timeout = timeout_ms.unwrap_or(5000).to_string();
        let mut controller_url = controller_url(addr, &["proxies", proxy_name, "delay"])?;
        controller_url
            .query_pairs_mut()
            .append_pair("url", delay_url)
            .append_pair("timeout", &timeout);
        let response = reqwest::get(controller_url).await;
        match response {
            Ok(response) if response.status().is_success() => {
                let value: serde_json::Value = response.json().await.map_err(|err| {
                    HMetaError::Core(format!("meow API delay response parse failed: {err}"))
                })?;
                let delay = value
                    .get("delay")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|delay| u16::try_from(delay).ok())
                    .ok_or_else(|| {
                        HMetaError::Core("meow API delay response missing delay".to_owned())
                    })?;
                let mut state = self.lock_state()?;
                if let Some(tunnel) = state.tunnel.clone() {
                    refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                }
                state.logs.push(info_log(format!(
                    "{proxy_name} delay: {delay} ms via meow API"
                )));
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
    ) -> Result<BTreeMap<String, u16>, HMetaError> {
        let controller = {
            let state = self.lock_state()?;
            controller_credentials(&state)
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
                                HMetaError::Core(format!(
                                    "meow API group delay response parse failed: {error}"
                                ))
                            })?;
                    let mut state = self.lock_state()?;
                    if let Some(tunnel) = state.tunnel.clone() {
                        refresh_proxy_groups_preserving_order(&mut state, &tunnel);
                    }
                    if state
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
                        }
                    }
                    state.logs.push(info_log(format!(
                        "group {group_name} delay tested via meow API: {} members",
                        delays.len()
                    )));
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
                .ok_or_else(|| HMetaError::Core(format!("proxy group not found: {group_name}")))?
        };
        let mut delays = BTreeMap::new();
        for member in members {
            let delay = self
                .test_proxy_delay(&member, Some(delay_url), Some(timeout))
                .await
                .unwrap_or(0);
            delays.insert(member, delay);
        }
        Ok(delays)
    }

    pub async fn flush_dns_cache_via_controller(&self) -> Result<(), HMetaError> {
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
            tunnel.ok_or_else(|| HMetaError::Core("meow tunnel is not loaded".to_owned()))?;
        tunnel.resolver().clear_cache();
        let mut state = self.lock_state()?;
        state.logs.push(info_log("DNS caches flushed"));
        Ok(())
    }

    pub async fn flush_fake_ip_cache_via_controller(&self) -> Result<(), HMetaError> {
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
            tunnel.ok_or_else(|| HMetaError::Core("meow tunnel is not loaded".to_owned()))?;
        tunnel
            .resolver()
            .flush_fake_ip()
            .map_err(|error| HMetaError::Core(format!("fake-IP cache flush failed: {error}")))?;
        let mut state = self.lock_state()?;
        state.logs.push(info_log("fake-IP cache flushed"));
        Ok(())
    }

    pub async fn healthcheck_proxy_provider_via_controller(
        &self,
        provider_name: &str,
    ) -> Result<(), HMetaError> {
        let (addr, secret) = {
            let state = self.lock_state()?;
            controller_credentials(&state)
        }
        .ok_or_else(|| HMetaError::Core("meow external-controller is not running".to_owned()))?;
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
            .map_err(|error| HMetaError::Core(format!("provider health check failed: {error}")))?;
        if !response.status().is_success() {
            return Err(HMetaError::Core(format!(
                "provider health check failed with HTTP {}",
                response.status()
            )));
        }
        let mut state = self.lock_state()?;
        state.logs.push(info_log(format!(
            "proxy provider {provider_name} health checked via meow API"
        )));
        Ok(())
    }

    pub async fn healthcheck_provider_proxy_via_controller(
        &self,
        provider_name: &str,
        proxy_name: &str,
        url: &str,
        timeout_ms: Option<u64>,
        expected_status: Option<&str>,
    ) -> Result<u16, HMetaError> {
        let (addr, secret) = {
            let state = self.lock_state()?;
            controller_credentials(&state)
        }
        .ok_or_else(|| HMetaError::Core("meow external-controller is not running".to_owned()))?;
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
                HMetaError::Core(format!("provider member health check failed: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(HMetaError::Core(format!(
                "provider member health check failed with HTTP {}",
                response.status()
            )));
        }
        let value = response
            .json::<serde_json::Value>()
            .await
            .map_err(|error| {
                HMetaError::Core(format!(
                    "provider member health response parse failed: {error}"
                ))
            })?;
        value
            .get("delay")
            .and_then(serde_json::Value::as_u64)
            .and_then(|delay| u16::try_from(delay).ok())
            .ok_or_else(|| {
                HMetaError::Core("provider member health response missing delay".to_owned())
            })
    }

    pub async fn refresh_provider(&self, provider_name: &str) -> Result<(), HMetaError> {
        self.refresh_provider_with_type(None, provider_name).await
    }

    pub async fn refresh_provider_of_type(
        &self,
        provider_type: &str,
        provider_name: &str,
    ) -> Result<(), HMetaError> {
        self.refresh_provider_with_type(Some(provider_type), provider_name)
            .await
    }

    async fn refresh_provider_with_type(
        &self,
        requested_provider_type: Option<&str>,
        provider_name: &str,
    ) -> Result<(), HMetaError> {
        let (controller_addr, provider, active_profile) = {
            let state = self.lock_state()?;
            (
                state
                    .api_controller
                    .as_ref()
                    .map(|controller| controller.addr),
                state
                    .providers
                    .iter()
                    .find(|provider| {
                        provider.name == provider_name
                            && requested_provider_type
                                .map(|provider_type| provider.provider_type == provider_type)
                                .unwrap_or(true)
                    })
                    .cloned(),
                state.profiles.active_profile().map(ToOwned::to_owned),
            )
        };
        let Some(provider) = provider else {
            let provider_label = requested_provider_type
                .map(|provider_type| format!("{provider_type}/{provider_name}"))
                .unwrap_or_else(|| provider_name.to_owned());
            let message = format!("provider refresh failed: provider not found: {provider_label}");
            let mut state = self.lock_state()?;
            state.logs.push(warning_log(message.clone()));
            return Err(HMetaError::Core(message));
        };
        let provider_type = provider.provider_type.clone();
        if provider_is_inline(&provider) {
            let message =
                format!("{provider_type} provider refresh skipped: {provider_name} is inline");
            let mut state = self.lock_state()?;
            mark_provider_refresh(
                &mut state,
                &provider_type,
                provider_name,
                unix_timestamp_string(),
                Some(message.clone()),
            );
            state.logs.push(warning_log(message.clone()));
            return Err(HMetaError::Core(message));
        }
        let Some(addr) = controller_addr else {
            let active =
                active_profile.ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
            return self.reload_config(&active).await;
        };
        let provider_collection = match provider_type.as_str() {
            "proxy" => "proxies",
            "rule" => "rules",
            other => {
                let message = format!("unknown provider type for {provider_name}: {other}");
                let mut state = self.lock_state()?;
                mark_provider_refresh(
                    &mut state,
                    other,
                    provider_name,
                    unix_timestamp_string(),
                    Some(message.clone()),
                );
                return Err(HMetaError::Core(format!(
                    "unknown provider type for {provider_name}: {other}"
                )));
            }
        };
        let url = controller_url(addr, &["providers", provider_collection, provider_name])?;
        let response = reqwest::Client::new().put(url).send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                {
                    let mut state = self.lock_state()?;
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
                }
                if provider_type == "rule" {
                    let active = active_profile
                        .ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
                    self.reload_config(&active).await?;
                }
                Ok(())
            }
            Ok(response) => {
                let message = format!(
                    "{provider_type} provider refresh failed via meow API: {provider_name} ({})",
                    response.status()
                );
                let mut state = self.lock_state()?;
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
                Err(HMetaError::Core(message))
            }
            Err(err) => {
                let message = format!(
                    "{provider_type} provider refresh failed via meow API: {provider_name} ({err})"
                );
                let mut state = self.lock_state()?;
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
                Err(HMetaError::Core(message))
            }
        }
    }

    pub async fn refresh_all_providers(&self) -> Result<(), HMetaError> {
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
            return Err(HMetaError::Core(format!(
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
    ) -> Result<(), HMetaError> {
        self.validate_meow_config(raw_yaml).await?;
        let previous_raw_yaml = {
            let state = self.lock_state()?;
            state.profiles.raw_yaml(profile_id)?
        };
        let active = {
            let mut state = self.lock_state()?;
            state
                .profiles
                .update_profile_content(profile_id, raw_yaml)?;
            state
                .logs
                .push(info_log(format!("profile edited: {profile_id}")));
            state.profiles.active_profile().map(ToOwned::to_owned)
        };
        if active.as_deref() == Some(profile_id) {
            if let Err(error) = self.reload_config(profile_id).await {
                {
                    let mut state = self.lock_state()?;
                    let _ = state
                        .profiles
                        .update_profile_content(profile_id, previous_raw_yaml);
                    state.logs.push(warning_log(format!(
                        "profile edit rolled back after reload failure: {profile_id}"
                    )));
                }
                let _ = self.reload_config(profile_id).await;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn update_profile_subscription(
        &self,
        profile_id: &str,
        name: &str,
        subscription_url: &str,
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        state
            .profiles
            .update_profile_subscription(profile_id, name, subscription_url)?;
        state.logs.push(info_log(format!(
            "profile subscription updated: {profile_id}"
        )));
        Ok(())
    }

    pub async fn validate_profile_content(&self, raw_yaml: &str) -> Result<(), HMetaError> {
        self.validate_meow_config(raw_yaml).await
    }

    async fn validate_meow_config(&self, raw_yaml: &str) -> Result<(), HMetaError> {
        let store_root = {
            let state = self.lock_state()?;
            state.profiles.root().to_path_buf()
        };
        validate_meow_config(raw_yaml, &store_root).await
    }

    pub fn profile_raw_yaml(&self, profile_id: &str) -> Result<String, HMetaError> {
        let state = self.lock_state()?;
        state.profiles.raw_yaml(profile_id)
    }

    pub async fn restore_profile_backup(&self, profile_id: &str) -> Result<(), HMetaError> {
        let active = {
            let mut state = self.lock_state()?;
            state.profiles.restore_profile_backup(profile_id)?;
            state.logs.push(info_log(format!(
                "profile restored from backup: {profile_id}"
            )));
            state.profiles.active_profile().map(ToOwned::to_owned)
        };
        if active.as_deref() == Some(profile_id) {
            self.reload_config(profile_id).await?;
        }
        Ok(())
    }

    pub async fn set_profile_dns_servers(
        &self,
        profile_id: &str,
        dns_servers: Vec<String>,
    ) -> Result<(), HMetaError> {
        let (fallbacks, policy) = {
            let state = self.lock_state()?;
            let raw_yaml = state.profiles.raw_yaml(profile_id)?;
            let options = hmeta_profile::vpn_options_from_yaml(&raw_yaml)?;
            (options.dns_fallbacks, options.dns_nameserver_policy)
        };
        self.set_profile_dns_config(profile_id, dns_servers, fallbacks, policy)
            .await
    }

    pub async fn set_profile_dns_config(
        &self,
        profile_id: &str,
        dns_servers: Vec<String>,
        dns_fallbacks: Vec<String>,
        dns_nameserver_policy: BTreeMap<String, Vec<String>>,
    ) -> Result<(), HMetaError> {
        let active = {
            let mut state = self.lock_state()?;
            state.profiles.set_profile_dns_config(
                profile_id,
                dns_servers,
                dns_fallbacks,
                dns_nameserver_policy,
            )?;
            state
                .logs
                .push(info_log(format!("DNS config updated for {profile_id}")));
            state.profiles.active_profile().map(ToOwned::to_owned)
        };
        if active.as_deref() == Some(profile_id) {
            self.reload_config(profile_id).await?;
        }
        Ok(())
    }

    pub async fn set_profile_vpn_config(
        &self,
        profile_id: &str,
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    ) -> Result<(), HMetaError> {
        let active = {
            let mut state = self.lock_state()?;
            state.profiles.set_profile_vpn_config(
                profile_id,
                system_proxy,
                dns_hijacking,
                allow_bypass,
                stack,
            )?;
            state
                .logs
                .push(info_log(format!("VPN config updated for {profile_id}")));
            state.profiles.active_profile().map(ToOwned::to_owned)
        };
        if active.as_deref() == Some(profile_id) {
            self.reload_config(profile_id).await?;
        }
        Ok(())
    }

    pub fn close_connection(&self, id: &str) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        let Some(tunnel) = &state.tunnel else {
            return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
        };
        let connection_id = id
            .parse()
            .map_err(|err| HMetaError::Core(format!("invalid connection id {id}: {err}")))?;
        tunnel.statistics().close_connection(connection_id);
        state
            .logs
            .push(info_log(format!("connection closed: {id}")));
        Ok(())
    }

    pub async fn close_connection_via_controller(&self, id: &str) -> Result<(), HMetaError> {
        let controller_addr = {
            let state = self.lock_state()?;
            state
                .api_controller
                .as_ref()
                .map(|controller| controller.addr)
        };
        let Some(addr) = controller_addr else {
            return self.close_connection(id);
        };
        let url = controller_url(addr, &["connections", id])?;
        let response = reqwest::Client::new().delete(url).send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let mut state = self.lock_state()?;
                state
                    .logs
                    .push(info_log(format!("connection closed via meow API: {id}")));
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

    pub fn close_all_connections(&self) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        let Some(tunnel) = &state.tunnel else {
            return Err(HMetaError::Core("meow tunnel is not loaded".to_owned()));
        };
        let count = tunnel.statistics().active_connection_count();
        tunnel.statistics().close_all_connections();
        state
            .logs
            .push(info_log(format!("all connections closed: {count}")));
        Ok(())
    }

    pub async fn close_all_connections_via_controller(&self) -> Result<(), HMetaError> {
        let (controller_addr, count) = {
            let state = self.lock_state()?;
            (
                state
                    .api_controller
                    .as_ref()
                    .map(|controller| controller.addr),
                state
                    .tunnel
                    .as_ref()
                    .map(|tunnel| tunnel.statistics().active_connection_count())
                    .unwrap_or(0),
            )
        };
        let Some(addr) = controller_addr else {
            return self.close_all_connections();
        };
        let url = controller_url(addr, &["connections"])?;
        let response = reqwest::Client::new().delete(url).send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let mut state = self.lock_state()?;
                state.logs.push(info_log(format!(
                    "all connections closed via meow API: {count}"
                )));
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

    pub fn snapshot(&self) -> Result<RuntimeSnapshot, HMetaError> {
        self.snapshot_internal(true)
    }

    fn snapshot_internal(
        &self,
        allow_platform_telemetry: bool,
    ) -> Result<RuntimeSnapshot, HMetaError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        state.logs.sync_session();
        if let Ok(mut runtime_logs) = RUNTIME_LOGS.lock() {
            runtime_logs.sync(state.logs.root());
        }
        let recording_enabled = state.logs.enabled();
        let local_logs = if recording_enabled {
            merged_logs(&state.logs)
        } else {
            Vec::new()
        };
        let native_vpn_running = self.vpn.is_running();
        let tun_stats = self.vpn.stats();
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let platform_telemetry = if allow_platform_telemetry && tun_stats.is_none() {
            self.read_platform_vpn_telemetry().filter(|telemetry| {
                telemetry.active_profile.as_deref() == active_profile.as_deref()
            })
        } else {
            None
        };

        let (connections, dns, request_history, logs) =
            if let Some(telemetry) = platform_telemetry.as_ref() {
                state.traffic = telemetry.traffic.clone();
                state.traffic_history = telemetry.traffic_history.iter().cloned().collect();
                state.last_traffic_sample = None;
                state.last_meow_traffic_sample = None;
                (
                    telemetry.connections.clone(),
                    telemetry.dns.clone(),
                    telemetry.request_history.clone(),
                    if recording_enabled {
                        merge_platform_logs(local_logs, &telemetry.logs)
                    } else {
                        Vec::new()
                    },
                )
            } else {
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
                (
                    connections,
                    dns_snapshot(&state.vpn_options, tun_stats.as_ref()),
                    state.request_history.iter().cloned().rev().collect(),
                    local_logs,
                )
            };

        let mut profiles = state.profiles.summaries();
        if let Some(profile) = profiles.iter_mut().find(|profile| profile.active) {
            profile.rule_count = state
                .runtime_rules
                .iter()
                .filter(|rule| rule.enabled)
                .count();
        }
        if let Some(telemetry) = platform_telemetry.as_ref() {
            if let Some(profile) = profiles
                .iter_mut()
                .find(|profile| Some(profile.id.as_str()) == telemetry.active_profile.as_deref())
            {
                profile.upload_bytes = telemetry.profile_upload_bytes;
                profile.download_bytes = telemetry.profile_download_bytes;
            }
        };
        refresh_provider_cache_metadata(&mut state.providers);
        if let Some(proxy_providers) = state
            .api_controller
            .as_ref()
            .map(|controller| Arc::clone(&controller.proxy_providers))
        {
            enrich_proxy_provider_members(&mut state.providers, &proxy_providers);
        }
        let controller_diagnostics = state
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
            });
        let vpn_running = native_vpn_running || state.platform_vpn_running;
        Ok(RuntimeSnapshot {
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
                .map(|controller| controller.addr.to_string()),
            controller_diagnostics,
            active_profile,
            mode: state.mode,
            traffic: state.traffic.clone(),
            traffic_history: state.traffic_history.iter().cloned().collect(),
            dns,
            vpn_options: state.vpn_options.clone(),
            proxy_groups: state.proxy_groups.clone(),
            profiles,
            rules: state.runtime_rules.clone(),
            providers: state.providers.clone(),
            geodata: state.profiles.geodata_files(),
            logs,
            connections,
            request_history,
            about: about_snapshot(),
        })
    }

    /// Persists the VPN extension process' live counters for the UI process.
    ///
    /// HarmonyOS runs `VpnExtensionAbility` in a separate process, so the UI
    /// cannot observe the native TUN session through this process' memory.
    pub fn persist_vpn_telemetry(&self) -> Result<(), HMetaError> {
        let snapshot = self.snapshot_internal(false)?;
        let (profile_upload_bytes, profile_download_bytes) = snapshot
            .profiles
            .iter()
            .find(|profile| profile.active)
            .map(|profile| (profile.upload_bytes, profile.download_bytes))
            .unwrap_or_default();
        let telemetry = PlatformVpnTelemetry {
            updated_at: now_unix_nanos(),
            active_profile: snapshot.active_profile,
            traffic: snapshot.traffic,
            traffic_history: snapshot.traffic_history,
            dns: snapshot.dns,
            connections: snapshot.connections,
            request_history: snapshot.request_history,
            logs: snapshot.logs,
            profile_upload_bytes,
            profile_download_bytes,
        };
        let Some(platform) = self.platform_ipc()? else {
            return Ok(());
        };
        platform
            .publish_telemetry(telemetry)
            .map_err(platform_ipc_error)
    }

    pub fn snapshot_json(&self) -> Result<String, HMetaError> {
        to_json(&self.snapshot()?)
    }

    fn restart_api_controller(
        &self,
        state: &mut CoreState,
        profile_id: &str,
        raw_config: RawConfig,
        proxy_providers: HashMap<String, Arc<ProxyProvider>>,
        rule_providers: HashMap<String, Arc<RuleProvider>>,
        listeners: Vec<NamedListener>,
        tunnel: &Tunnel,
    ) -> Result<(), HMetaError> {
        if !self.api_controller_enabled {
            return Ok(());
        }
        let addr = self
            .api_controller_addr_override
            .or_else(|| {
                raw_config
                    .external_controller
                    .as_deref()
                    .and_then(|addr| addr.parse::<SocketAddr>().ok())
            })
            .ok_or_else(|| HMetaError::Core("external-controller is not configured".to_owned()))?;
        let runtime_path = state.profiles.runtime_yaml_path(profile_id);
        let (log_tx, _) = tokio::sync::broadcast::channel(256);
        if let Ok(mut senders) = API_LOG_TXS.lock() {
            senders.push_back(log_tx.clone());
            while senders.len() > MAX_API_LOG_SENDERS {
                senders.pop_front();
            }
        }
        state.api_controller = None;
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
            if let Err(err) = run_api_controller(addr, app_state, task_revision).await {
                tracing::warn!("meow external-controller stopped: {err}");
            }
        });
        let memory_task = tokio::spawn(monitor_controller_memory(
            addr,
            raw_config.secret.clone(),
            Arc::clone(&memory_in_use_bytes),
            Arc::clone(&memory_limit_bytes),
        ));
        state.api_controller = Some(ApiControllerRuntime {
            addr,
            task,
            memory_task,
            raw_config: shared_raw_config,
            baseline_raw_config: raw_config,
            proxy_providers,
            config_revision,
            synced_revision: 0,
            memory_in_use_bytes,
            memory_limit_bytes,
        });
        state.logs.push(info_log(format!(
            "meow external-controller listening on {addr}"
        )));
        Ok(())
    }

    fn persist_platform_vpn_state_locked(&self, state: &mut CoreState) -> Result<(), HMetaError> {
        // SystemTime can have coarser resolution than nanoseconds on device.
        // Starting and running may otherwise receive the same timestamp, and
        // the receiver would permanently discard the terminal state because
        // platform synchronization accepts only strictly newer revisions.
        state.platform_vpn_state_updated_at =
            now_unix_nanos().max(state.platform_vpn_state_updated_at.saturating_add(1));
        let Some(platform) = self.platform_ipc()? else {
            self.notify_platform_start_locked(state);
            return Ok(());
        };
        self.notify_platform_start_locked(state);
        platform
            .publish_state(platform_vpn_state(state))
            .map_err(platform_ipc_error)
    }

    fn persist_platform_vpn_control_locked(
        &self,
        state: &mut CoreState,
        mode: RuntimeMode,
        global_proxy: Option<String>,
    ) -> Result<(), HMetaError> {
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
    ) -> Result<(), HMetaError> {
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
        if let Some(remote) = envelope
            .state
            .filter(|remote| remote.updated_at > state.platform_vpn_state_updated_at)
        {
            let remote_attempt_matches = !remote.start_attempt_id.is_empty()
                && remote.start_attempt_id == state.platform_start_attempt_id;
            if !is_ui || remote_attempt_matches {
                if !is_ui && !remote.start_attempt_id.is_empty() && !remote_attempt_matches {
                    state.platform_start_attempt_id = remote.start_attempt_id.clone();
                    state.platform_start_outcome = remote.start_outcome;
                    state.platform_extension_attached = remote.extension_attached;
                } else if remote_attempt_matches
                    && state.platform_start_outcome == PlatformStartOutcome::Pending
                    && matches!(
                        remote.start_outcome,
                        PlatformStartOutcome::Connected
                            | PlatformStartOutcome::Failed
                            | PlatformStartOutcome::Cancelled
                    )
                {
                    state.platform_start_outcome = remote.start_outcome;
                }
                if remote_attempt_matches && remote.extension_attached {
                    state.platform_extension_attached = true;
                }
                state.platform_vpn_starting = !remote.running && remote.starting;
                state.platform_vpn_running = remote.running;
                if remote.running && state.platform_start_outcome == PlatformStartOutcome::Pending {
                    state.platform_start_outcome = PlatformStartOutcome::Connected;
                } else if !remote.running
                    && state.platform_start_outcome == PlatformStartOutcome::Pending
                    && remote.start_outcome == PlatformStartOutcome::Failed
                {
                    state.platform_start_outcome = PlatformStartOutcome::Failed;
                }
                state.platform_network_protected = remote.network_protected;
                state.platform_network_protect_error = remote.network_protect_error;
                state.platform_vpn_state_updated_at = remote.updated_at;
                self.notify_platform_start_locked(state);
            }
        }
        if let Some(control) = envelope
            .control
            .filter(|control| control.updated_at > state.platform_vpn_control_updated_at)
        {
            self.sync_platform_vpn_control_locked(state, control);
        }
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
            extension_attached: state.platform_extension_attached,
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

    fn sync_platform_vpn_control_locked(&self, state: &mut CoreState, control: PlatformVpnControl) {
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

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CoreState>, HMetaError> {
        self.state
            .lock()
            .map_err(|_| HMetaError::Core("core state lock poisoned".to_owned()))
    }
}

#[cfg(test)]
mod tests;
