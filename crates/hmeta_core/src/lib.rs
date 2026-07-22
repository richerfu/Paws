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
use meow_common::{ConnType, Metadata, Network, TunnelMode};
use meow_config::{
    proxy_provider::ProxyProvider, raw::RawConfig, rule_provider::RuleProvider, Config,
    NamedListener,
};
use meow_tunnel::Tunnel;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

static CORE: Lazy<Arc<CoreHandle>> = Lazy::new(|| Arc::new(CoreHandle::new()));
static RUNTIME_LOGS: Lazy<Arc<Mutex<VecDeque<LogEntry>>>> =
    Lazy::new(|| Arc::new(Mutex::new(VecDeque::with_capacity(MAX_RUNTIME_LOGS))));
static API_LOG_TXS: Lazy<
    Arc<Mutex<VecDeque<tokio::sync::broadcast::Sender<meow_api::log_stream::LogMessage>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(VecDeque::new())));
static INSTALL_RUNTIME_LOG_LAYER: Once = Once::new();
const MAX_RUNTIME_LOGS: usize = 256;
const MAX_API_LOG_SENDERS: usize = 8;
const MAX_REQUEST_HISTORY: usize = 128;
const MAX_TRAFFIC_HISTORY: usize = 32;
const PLATFORM_VPN_STATE_FILE: &str = "platform-vpn-state.json";
const PLATFORM_VPN_TELEMETRY_FILE: &str = "platform-vpn-telemetry.json";
const APP_VERSION: &str = "1.0.0";
const MEOW_RS_VERSION: &str = "0.18.0";
const ARKIT_REV: &str = "e091886482f915779bc927d4aab5045922508851";
const RUST_VERSION: &str = "1.89";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformVpnState {
    starting: bool,
    running: bool,
    network_protected: bool,
    network_protect_error: Option<String>,
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
    platform_network_protected: bool,
    platform_network_protect_error: Option<String>,
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
    logs: Vec<LogEntry>,
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
        Self {
            engine_loaded: false,
            platform_vpn_starting: false,
            platform_vpn_running: false,
            platform_network_protected: false,
            platform_network_protect_error: None,
            mode: RuntimeMode::Rule,
            profiles,
            tunnel: None,
            sniffer_config: SnifferConfig::default(),
            proxy_groups: Vec::new(),
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
            logs: vec![LogEntry {
                level: "info".to_owned(),
                message: format!("hmeta core booted with {}", meow_version_marker()),
                timestamp: "boot".to_owned(),
            }],
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
    config_reload_lock: tokio::sync::Mutex<()>,
    vpn: TunSession,
    api_controller_enabled: bool,
    api_controller_addr_override: Option<SocketAddr>,
}

#[derive(Debug, Clone)]
pub struct ManualRuleApplyResult {
    pub mutation: ManualRuleMutation,
    pub live_updated: bool,
    pub rule_mode_active: bool,
}

impl CoreHandle {
    fn new() -> Self {
        install_runtime_log_layer();
        Self {
            state: Mutex::new(CoreState::default()),
            config_reload_lock: tokio::sync::Mutex::new(()),
            vpn: TunSession::default(),
            api_controller_enabled: true,
            api_controller_addr_override: None,
        }
    }

    #[cfg(test)]
    fn new_with_profile_root(root: impl Into<std::path::PathBuf>) -> Self {
        install_runtime_log_layer();
        let profiles = ProfileStore::open(root).expect("test profile store");
        Self {
            state: Mutex::new(CoreState {
                engine_loaded: false,
                platform_vpn_starting: false,
                platform_vpn_running: false,
                platform_network_protected: false,
                platform_network_protect_error: None,
                mode: RuntimeMode::Rule,
                profiles,
                tunnel: None,
                sniffer_config: SnifferConfig::default(),
                proxy_groups: Vec::new(),
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
                logs: Vec::new(),
                request_history: VecDeque::with_capacity(MAX_REQUEST_HISTORY),
                vpn_options: VpnOptions::default(),
                api_controller: None,
                controller_config_sync_count: 0,
                last_controller_config_sync_at: None,
                last_controller_config_sync_error: None,
            }),
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
        validate_meow_config(&raw_yaml).await?;
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
        validate_meow_config(&raw_yaml).await?;
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
        validate_meow_config(&raw_yaml).await?;
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
        sync_platform_vpn_state(&mut state);
        state.vpn_options = options;
        state.engine_loaded = true;
        state.platform_vpn_starting = false;
        state.platform_vpn_running = true;
        state
            .logs
            .push(info_log(format!("vpn started with tun fd {fd}")));
        persist_platform_vpn_state(&state)?;
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

    pub fn active_vpn_options_json(&self) -> Result<String, HMetaError> {
        let state = self.lock_state()?;
        to_json(&state.profiles.active_vpn_options()?)
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
        state.platform_network_protected = false;
        state.platform_network_protect_error = None;
        state.traffic.upload_speed = 0;
        state.traffic.download_speed = 0;
        state.last_traffic_sample = None;
        state.logs.push(info_log("vpn stopped"));
        persist_platform_vpn_state(&state)?;
        Ok(())
    }

    pub fn set_platform_vpn_starting(&self, starting: bool) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        sync_platform_vpn_state(&mut state);
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
        persist_platform_vpn_state(&state)
    }

    pub fn expire_platform_vpn_start(&self) -> Result<bool, HMetaError> {
        let mut state = self.lock_state()?;
        sync_platform_vpn_state(&mut state);
        if !state.platform_vpn_starting || state.platform_vpn_running {
            return Ok(false);
        }
        state.platform_vpn_starting = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error =
            Some("VPN extension did not report readiness before the startup timeout".to_owned());
        state
            .logs
            .push(warning_log("platform vpn startup timed out"));
        persist_platform_vpn_state(&state)?;
        Ok(true)
    }

    pub fn set_platform_vpn_failed(&self, error: String) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        sync_platform_vpn_state(&mut state);
        state.platform_vpn_starting = false;
        state.platform_vpn_running = false;
        state.platform_network_protected = false;
        state.platform_network_protect_error = Some(error.clone());
        state
            .logs
            .push(warning_log(format!("platform vpn start failed: {error}")));
        persist_platform_vpn_state(&state)
    }

    pub fn set_platform_vpn_running(&self, running: bool) -> Result<(), HMetaError> {
        let stats = if running { None } else { self.vpn.stats() };
        let mut state = self.lock_state()?;
        sync_platform_vpn_state(&mut state);
        state.platform_vpn_starting = false;
        state.platform_vpn_running = running;
        if !running {
            settle_traffic_before_platform_stop(&mut state, stats.as_ref())?;
            state.platform_network_protected = false;
            state.platform_network_protect_error = None;
        }
        state.logs.push(info_log(if running {
            "platform vpn running"
        } else {
            "platform vpn stopped"
        }));
        persist_platform_vpn_state(&state)
    }

    pub fn set_platform_network_protected(
        &self,
        protected: bool,
        error: Option<String>,
    ) -> Result<(), HMetaError> {
        let mut state = self.lock_state()?;
        sync_platform_vpn_state(&mut state);
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
        persist_platform_vpn_state(&state)
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
        let mut proxy_groups = proxy_groups_from_tunnel(&tunnel);
        let runtime_ready = Instant::now();
        let mut state = self.lock_state()?;
        if preserve_existing_order && state.profiles.active_profile() == Some(profile_id) {
            preserve_proxy_group_member_order(&state.proxy_groups, &mut proxy_groups);
        }
        apply_provider_refresh_states(&mut providers, &state.provider_refresh);
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
        validate_meow_config(raw_yaml).await?;
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
        validate_meow_config(raw_yaml).await
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
        sync_platform_vpn_state(&mut state);
        let native_vpn_running = self.vpn.is_running();
        let tun_stats = self.vpn.stats();
        let active_profile = state.profiles.active_profile().map(ToOwned::to_owned);
        let platform_telemetry = if allow_platform_telemetry && tun_stats.is_none() {
            read_platform_vpn_telemetry(&state).filter(|telemetry| {
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
                    merge_platform_logs(merged_logs(&state.logs), &telemetry.logs),
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
                    merged_logs(&state.logs),
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
        let path = {
            let state = self.lock_state()?;
            platform_vpn_telemetry_path(&state)
        };
        persist_platform_vpn_telemetry_at(&path, &telemetry)
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

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CoreState>, HMetaError> {
        self.state
            .lock()
            .map_err(|_| HMetaError::Core("core state lock poisoned".to_owned()))
    }
}

async fn track_controller_mutation(
    AxumState(revision): AxumState<Arc<AtomicU64>>,
    request: Request,
    next: Next,
) -> AxumResponse {
    let mutation = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let response = next.run(request).await;
    if mutation && response.status().is_success() {
        revision.fetch_add(1, Ordering::AcqRel);
    }
    response
}

async fn run_api_controller(
    addr: SocketAddr,
    state: Arc<meow_api::routes::AppState>,
    revision: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = meow_api::routes::create_router(state).layer(axum::middleware::from_fn_with_state(
        revision,
        track_controller_mutation,
    ));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("REST API listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn monitor_controller_memory(
    addr: SocketAddr,
    secret: Option<String>,
    memory_in_use_bytes: Arc<AtomicU64>,
    memory_limit_bytes: Arc<AtomicU64>,
) {
    loop {
        let mut url = format!("ws://{addr}/memory");
        if let Some(secret) = secret.as_deref().filter(|secret| !secret.is_empty()) {
            if let Ok(mut parsed) = reqwest::Url::parse(&url) {
                parsed.query_pairs_mut().append_pair("token", secret);
                url = parsed.to_string();
            }
        }
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut socket, _)) => {
                while let Some(frame) = socket.next().await {
                    let payload = match frame {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            Some(text.as_bytes().to_vec())
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                            Some(bytes.to_vec())
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                        _ => None,
                    };
                    let Some(payload) = payload else { continue };
                    if let Ok(frame) = serde_json::from_slice::<ControllerMemoryFrame>(&payload) {
                        memory_in_use_bytes.store(frame.inuse, Ordering::Relaxed);
                        memory_limit_bytes.store(frame.oslimit, Ordering::Relaxed);
                    }
                }
            }
            Err(error) => {
                tracing::debug!(%error, "waiting for meow controller memory stream");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

pub fn shared_core() -> Arc<CoreHandle> {
    CoreHandle::shared()
}

async fn validate_meow_config(raw_yaml: &str) -> Result<(), HMetaError> {
    let validation_yaml = hmeta_profile::sanitize_profile_for_meow_validation(raw_yaml)?;
    let _ = load_meow_config(&validation_yaml).await?;
    Ok(())
}

async fn load_meow_config(raw_yaml: &str) -> Result<Config, HMetaError> {
    meow_config::load_config_from_str(raw_yaml)
        .await
        .map_err(|err| HMetaError::Core(format!("meow config load failed: {err}")))
}

fn tunnel_from_config(config: Config, mode: RuntimeMode) -> Tunnel {
    let tunnel = Tunnel::new(config.dns.resolver.clone());
    tunnel.update_rules(config.rules);
    tunnel.update_proxies(config.proxies);
    tunnel.set_mode(mode_to_tunnel(mode));
    tunnel.spawn_background_tasks();
    tunnel
}

fn raw_configs_equal(left: &RawConfig, right: &RawConfig) -> Result<bool, HMetaError> {
    let left = serde_yaml::to_value(left)
        .map_err(|error| HMetaError::Core(format!("cannot inspect controller config: {error}")))?;
    let right = serde_yaml::to_value(right)
        .map_err(|error| HMetaError::Core(format!("cannot inspect controller config: {error}")))?;
    Ok(left == right)
}

fn merge_external_raw_config(
    profile_yaml: &str,
    baseline: &RawConfig,
    current: &RawConfig,
) -> Result<String, HMetaError> {
    let mut profile = serde_yaml::from_str::<serde_yaml::Value>(profile_yaml)
        .map_err(|error| HMetaError::Core(format!("profile YAML parse failed: {error}")))?;
    let profile = profile
        .as_mapping_mut()
        .ok_or_else(|| HMetaError::Core("profile YAML root must be a mapping".to_owned()))?;
    let baseline = serde_yaml::to_value(baseline).map_err(|error| {
        HMetaError::Core(format!("cannot serialize controller config: {error}"))
    })?;
    let current = serde_yaml::to_value(current).map_err(|error| {
        HMetaError::Core(format!("cannot serialize controller config: {error}"))
    })?;
    let baseline = baseline
        .as_mapping()
        .ok_or_else(|| HMetaError::Core("controller baseline is not a mapping".to_owned()))?;
    let current = current
        .as_mapping()
        .ok_or_else(|| HMetaError::Core("controller config is not a mapping".to_owned()))?;

    let mut keys = baseline
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    keys.dedup();
    for key in keys {
        let Some(name) = key.as_str() else { continue };
        if controller_runtime_only_key(name) || baseline.get(&key) == current.get(&key) {
            continue;
        }
        match current.get(&key) {
            Some(value) if !value.is_null() => {
                profile.insert(key, value.clone());
            }
            _ => {
                profile.remove(&key);
            }
        }
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(profile.clone()))
        .map_err(|error| HMetaError::Core(format!("profile YAML serialization failed: {error}")))
}

fn controller_runtime_only_key(key: &str) -> bool {
    matches!(
        key,
        "port"
            | "socks-port"
            | "mixed-port"
            | "allow-lan"
            | "bind-address"
            | "mode"
            | "log-level"
            | "external-controller"
            | "external-ui"
            | "external-ui-name"
            | "external-ui-url"
            | "secret"
            | "tproxy-port"
            | "tproxy-sni"
            | "routing-mark"
            | "listeners"
            | "authentication"
            | "skip-auth-prefixes"
            | "max-connections"
    )
}

fn sync_live_controller_route(state: &mut CoreState) -> Result<(), HMetaError> {
    let Some(tunnel) = state.tunnel.clone() else {
        return Ok(());
    };
    state.mode = mode_from_tunnel(tunnel.mode());
    refresh_proxy_groups_preserving_order(state, &tunnel);
    let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) else {
        return Ok(());
    };
    let persisted = state.profiles.selected_proxies(&profile_id)?;
    let selections = state
        .proxy_groups
        .iter()
        .filter_map(|group| {
            let selected = match group.fixed.as_deref() {
                Some(fixed) => fixed.to_owned(),
                None => group.selected.clone()?,
            };
            Some((group.name.clone(), selected))
        })
        .collect::<Vec<_>>();
    for (group, selected) in selections {
        if persisted.get(&group) != Some(&selected) {
            state
                .profiles
                .set_selected_proxy(&profile_id, group, selected)?;
        }
    }
    Ok(())
}

fn restore_proxy_selections(
    tunnel: &Tunnel,
    selected_proxies: &std::collections::BTreeMap<String, String>,
) {
    if selected_proxies.is_empty() {
        return;
    }
    let route = tunnel.route_snapshot();
    let proxies = &route.proxies;
    for (group_name, proxy_name) in selected_proxies {
        let Some(group) = proxies.get(group_name.as_str()) else {
            continue;
        };
        let Some(selection) = group.selection() else {
            continue;
        };
        if proxy_name.is_empty() && selection.can_unfix() {
            selection.force_set(None);
        } else if group
            .members()
            .is_some_and(|members| members.iter().any(|member| member == proxy_name))
        {
            selection.force_set(Some(proxy_name));
        }
    }
}

fn runtime_rule_summaries(
    profile_id: &str,
    loaded_lines: &[String],
    editable_rules: &[hmeta_model::RuleSummary],
) -> Vec<hmeta_model::RuleSummary> {
    let mut consumed_editable = vec![false; editable_rules.len()];
    let mut summaries = Vec::with_capacity(loaded_lines.len() + editable_rules.len());
    for (index, line) in loaded_lines.iter().enumerate() {
        if let Some((editable_index, editable)) =
            editable_rules
                .iter()
                .enumerate()
                .find(|(editable_index, editable)| {
                    !consumed_editable[*editable_index]
                        && editable.enabled
                        && editable.line.trim() == line.trim()
                })
        {
            consumed_editable[editable_index] = true;
            let mut editable = editable.clone();
            editable.order = index as u32;
            summaries.push(editable);
        } else {
            summaries.push(hmeta_model::RuleSummary {
                id: format!("runtime-rule-{index}"),
                profile_id: profile_id.to_owned(),
                line: line.clone(),
                enabled: true,
                order: index as u32,
                source: "profile-yaml".to_owned(),
            });
        }
    }
    for (editable_index, editable) in editable_rules.iter().enumerate() {
        if !consumed_editable[editable_index] {
            let mut editable = editable.clone();
            editable.order = summaries.len() as u32;
            summaries.push(editable);
        }
    }
    summaries
}

fn apply_traffic_sample(state: &mut CoreState, stats: &TunStats) -> Result<(), HMetaError> {
    // Reading from a TUN descriptor receives packets written by applications
    // (device -> network), while writing to it delivers packets back to those
    // applications (network -> device). `TunStats` uses rx/tx from the native
    // descriptor's point of view, so their user-facing meanings are inverted.
    let upload_total = stats.rx_bytes;
    let download_total = stats.tx_bytes;
    let now = Instant::now();
    let (upload_delta, download_delta) =
        if let Some((last_at, last_upload, last_download)) = state.last_traffic_sample {
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let upload_delta = upload_total.saturating_sub(last_upload);
            let download_delta = download_total.saturating_sub(last_download);
            state.traffic.tun_upload_speed = ((upload_delta as f64) / elapsed) as u64;
            state.traffic.tun_download_speed = ((download_delta as f64) / elapsed) as u64;
            (upload_delta, download_delta)
        } else {
            (upload_total, download_total)
        };

    if let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) {
        state
            .profiles
            .add_profile_traffic(&profile_id, upload_delta, download_delta)?;
    }
    state.traffic.tun_upload_bytes = upload_total;
    state.traffic.tun_download_bytes = download_total;
    state.traffic.upload_bytes = state.traffic.tun_upload_bytes;
    state.traffic.download_bytes = state.traffic.tun_download_bytes;
    state.traffic.upload_speed = state.traffic.tun_upload_speed;
    state.traffic.download_speed = state.traffic.tun_download_speed;
    state.last_traffic_sample = Some((now, upload_total, download_total));
    Ok(())
}

fn apply_meow_traffic_sample(
    state: &mut CoreState,
    tunnel: &Tunnel,
    use_as_primary: bool,
) -> Result<(), HMetaError> {
    let (upload_total, download_total) = tunnel.statistics().snapshot();
    let upload_total = non_negative_i64_to_u64(upload_total);
    let download_total = non_negative_i64_to_u64(download_total);
    let now = Instant::now();
    let (upload_delta, download_delta) =
        if let Some((last_at, last_upload, last_download)) = state.last_meow_traffic_sample {
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let upload_delta = upload_total.saturating_sub(last_upload);
            let download_delta = download_total.saturating_sub(last_download);
            state.traffic.meow_upload_speed = ((upload_delta as f64) / elapsed) as u64;
            state.traffic.meow_download_speed = ((download_delta as f64) / elapsed) as u64;
            (upload_delta, download_delta)
        } else {
            (upload_total, download_total)
        };

    if use_as_primary {
        if let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) {
            state
                .profiles
                .add_profile_traffic(&profile_id, upload_delta, download_delta)?;
        }
        state.traffic.upload_bytes = upload_total;
        state.traffic.download_bytes = download_total;
        state.traffic.upload_speed = state.traffic.meow_upload_speed;
        state.traffic.download_speed = state.traffic.meow_download_speed;
    }
    state.traffic.meow_upload_bytes = upload_total;
    state.traffic.meow_download_bytes = download_total;
    state.last_meow_traffic_sample = Some((now, upload_total, download_total));
    Ok(())
}

fn baseline_meow_traffic_sample(state: &mut CoreState) {
    let Some(tunnel) = state.tunnel.clone() else {
        return;
    };
    let (upload_total, download_total) = tunnel.statistics().snapshot();
    let upload_total = non_negative_i64_to_u64(upload_total);
    let download_total = non_negative_i64_to_u64(download_total);
    state.traffic.meow_upload_bytes = upload_total;
    state.traffic.meow_download_bytes = download_total;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    state.last_meow_traffic_sample = Some((Instant::now(), upload_total, download_total));
}

fn settle_traffic_before_platform_stop(
    state: &mut CoreState,
    tun_stats: Option<&TunStats>,
) -> Result<(), HMetaError> {
    if let Some(stats) = tun_stats {
        apply_traffic_sample(state, stats)?;
        baseline_meow_traffic_sample(state);
    } else if let Some(tunnel) = state.tunnel.clone() {
        apply_meow_traffic_sample(state, &tunnel, true)?;
    }
    state.traffic.upload_speed = 0;
    state.traffic.download_speed = 0;
    state.traffic.tun_upload_speed = 0;
    state.traffic.tun_download_speed = 0;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    Ok(())
}

fn settle_traffic_before_profile_switch(
    state: &mut CoreState,
    tun_stats: Option<&TunStats>,
) -> Result<(), HMetaError> {
    if let Some(stats) = tun_stats {
        apply_traffic_sample(state, stats)?;
        baseline_meow_traffic_sample(state);
    } else if let Some(tunnel) = state.tunnel.clone() {
        apply_meow_traffic_sample(state, &tunnel, true)?;
    }
    state.traffic.upload_speed = 0;
    state.traffic.download_speed = 0;
    state.traffic.tun_upload_speed = 0;
    state.traffic.tun_download_speed = 0;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    Ok(())
}

fn record_traffic_history(state: &mut CoreState) {
    state.traffic_history.push_back(TrafficHistoryPoint {
        download_speed: state.traffic.download_speed,
        upload_speed: state.traffic.upload_speed,
    });
    while state.traffic_history.len() > MAX_TRAFFIC_HISTORY {
        state.traffic_history.pop_front();
    }
}

fn proxy_test_metadata(url: &str, in_name: &str) -> Result<Metadata, HMetaError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|err| HMetaError::Core(format!("invalid proxy test URL: {err}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| HMetaError::Core("proxy test URL has no host".to_owned()))?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    Ok(Metadata {
        network: Network::Tcp,
        // The echo probe writes an opaque payload after `dial_tcp`; it is not
        // an HTTP request even when its target is expressed as an http(s) URL.
        // `Inner` makes HTTP outbound adapters establish a CONNECT tunnel
        // instead of treating the payload as an HTTP-forward-proxy request.
        conn_type: ConnType::Inner,
        dst_port: port,
        host: host.into(),
        in_name: in_name.into(),
        in_port: 0,
        ..Metadata::default()
    })
}

fn dns_snapshot(options: &VpnOptions, tun_stats: Option<&TunStats>) -> DnsSnapshot {
    DnsSnapshot {
        model: if options.dns_hijacking {
            "tun-hijack".to_owned()
        } else {
            "meow-listener".to_owned()
        },
        hijacking: options.dns_hijacking,
        listen: "127.0.0.1:1053".to_owned(),
        upstreams: options.dns_servers.clone(),
        fallbacks: options.dns_fallbacks.clone(),
        nameserver_policy: options.dns_nameserver_policy.clone(),
        tun_addresses: options.dns_addresses.clone(),
        handled_packets: tun_stats.map(|stats| stats.dns_packets).unwrap_or(0),
        cache_hits: tun_stats.map(|stats| stats.dns_cache_hits).unwrap_or(0),
        cache_misses: tun_stats.map(|stats| stats.dns_cache_misses).unwrap_or(0),
        recent_queries: tun_stats
            .map(|stats| stats.recent_dns_queries.clone())
            .unwrap_or_default(),
    }
}

fn platform_vpn_state_path(state: &CoreState) -> PathBuf {
    state.profiles.root().join(PLATFORM_VPN_STATE_FILE)
}

fn platform_vpn_telemetry_path(state: &CoreState) -> PathBuf {
    state.profiles.root().join(PLATFORM_VPN_TELEMETRY_FILE)
}

fn platform_vpn_state(state: &CoreState) -> PlatformVpnState {
    PlatformVpnState {
        starting: state.platform_vpn_starting,
        running: state.platform_vpn_running,
        network_protected: state.platform_network_protected,
        network_protect_error: state.platform_network_protect_error.clone(),
        updated_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0),
    }
}

fn persist_platform_vpn_state(state: &CoreState) -> Result<(), HMetaError> {
    let path = platform_vpn_state_path(state);
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let content = serde_json::to_vec(&platform_vpn_state(state)).map_err(|error| {
        HMetaError::Core(format!("serialize platform VPN state failed: {error}"))
    })?;
    fs::write(&temp_path, content)
        .map_err(|error| HMetaError::Core(format!("write platform VPN state failed: {error}")))?;
    fs::rename(&temp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        HMetaError::Core(format!("replace platform VPN state failed: {error}"))
    })
}

fn persist_platform_vpn_telemetry_at(
    path: &Path,
    telemetry: &PlatformVpnTelemetry,
) -> Result<(), HMetaError> {
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let content = serde_json::to_vec(telemetry).map_err(|error| {
        HMetaError::Core(format!("serialize platform VPN telemetry failed: {error}"))
    })?;
    fs::write(&temp_path, content).map_err(|error| {
        HMetaError::Core(format!("write platform VPN telemetry failed: {error}"))
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        HMetaError::Core(format!("replace platform VPN telemetry failed: {error}"))
    })
}

fn read_platform_vpn_telemetry(state: &CoreState) -> Option<PlatformVpnTelemetry> {
    let content = fs::read(platform_vpn_telemetry_path(state)).ok()?;
    serde_json::from_slice(&content).ok()
}

fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn sync_platform_vpn_state(state: &mut CoreState) {
    let path = platform_vpn_state_path(state);
    let Ok(content) = fs::read(&path) else {
        return;
    };
    let Ok(platform) = serde_json::from_slice::<PlatformVpnState>(&content) else {
        return;
    };
    state.platform_vpn_starting = platform.starting;
    state.platform_vpn_running = platform.running;
    state.platform_network_protected = platform.network_protected;
    state.platform_network_protect_error = platform.network_protect_error;
}

fn vpn_lifecycle(
    engine_loaded: bool,
    platform_vpn_starting: bool,
    platform_vpn_running: bool,
    native_vpn_running: bool,
    network_protected: bool,
    network_protect_error: Option<&str>,
) -> VpnLifecycle {
    if native_vpn_running || platform_vpn_running {
        if network_protect_error.is_some() && !network_protected {
            VpnLifecycle::ProtectFailed
        } else {
            VpnLifecycle::Connected
        }
    } else if platform_vpn_starting {
        VpnLifecycle::Starting
    } else if network_protect_error.is_some() {
        VpnLifecycle::Failed
    } else if engine_loaded {
        VpnLifecycle::EngineLoaded
    } else {
        VpnLifecycle::Stopped
    }
}

fn mark_provider_refresh(
    state: &mut CoreState,
    provider_type: &str,
    provider_name: &str,
    refreshed_at: String,
    error: Option<String>,
) {
    let key = provider_refresh_key(provider_type, provider_name);
    let refresh = ProviderRefreshState {
        refreshed_at,
        error,
    };
    state.provider_refresh.insert(key, refresh);
    apply_provider_refresh_states(&mut state.providers, &state.provider_refresh);
}

fn apply_provider_refresh_states(
    providers: &mut [ProviderSummary],
    refresh: &HashMap<String, ProviderRefreshState>,
) {
    for provider in providers.iter_mut() {
        if let Some(record) = refresh.get(&provider_refresh_key(
            &provider.provider_type,
            &provider.name,
        )) {
            provider.last_refresh_at = Some(record.refreshed_at.clone());
            provider.last_refresh_error = record.error.clone();
        }
    }
    refresh_provider_cache_metadata(providers);
}

fn provider_refresh_key(provider_type: &str, provider_name: &str) -> String {
    format!("{provider_type}:{provider_name}")
}

fn provider_is_inline(provider: &ProviderSummary) -> bool {
    provider
        .vehicle_type
        .as_deref()
        .is_some_and(|vehicle_type| vehicle_type.eq_ignore_ascii_case("inline"))
}

fn provider_stale_cache_available(
    state: &CoreState,
    provider_type: &str,
    provider_name: &str,
) -> bool {
    state
        .providers
        .iter()
        .find(|provider| provider.provider_type == provider_type && provider.name == provider_name)
        .is_some_and(|provider| provider.stale_cache_available)
}

fn provider_refresh_failure_log_message(message: &str, stale_cache_available: bool) -> String {
    if stale_cache_available {
        format!("{message}; stale provider cache retained")
    } else {
        message.to_owned()
    }
}

fn refresh_provider_cache_metadata(providers: &mut [ProviderSummary]) {
    for provider in providers {
        let metadata = provider
            .path
            .as_deref()
            .and_then(|path| Path::new(path).metadata().ok())
            .filter(std::fs::Metadata::is_file);
        provider.cache_exists = metadata.is_some();
        provider.cache_bytes = metadata.as_ref().map(std::fs::Metadata::len);
        provider.cache_updated_at = metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(system_time_secs);
        provider.stale_cache_available =
            provider.last_refresh_error.is_some() && provider.cache_exists;
    }
}

fn enrich_proxy_provider_members(
    providers: &mut [ProviderSummary],
    registry: &dashmap::DashMap<String, Arc<ProxyProvider>>,
) {
    for provider in providers
        .iter_mut()
        .filter(|provider| provider.provider_type == "proxy")
    {
        let Some(runtime) = registry.get(&provider.name) else {
            provider.members.clear();
            continue;
        };
        if let Some(health_check) = runtime.health_check.as_ref() {
            provider.health_check_enabled = true;
            provider.health_check_url = Some(health_check.url.clone());
            provider.health_check_interval_seconds = Some(health_check.interval);
            provider.expected_status = Some(health_check.expected_status.clone());
        }
        let members: Vec<_> = runtime
            .proxies()
            .into_iter()
            .map(|proxy| ProviderProxySummary {
                name: proxy.name().to_owned(),
                proxy_type: proxy.adapter_type().to_string(),
                alive: proxy.alive(),
                delay_ms: (!proxy.delay_history().is_empty())
                    .then(|| u32::from(proxy.last_delay())),
            })
            .collect();
        provider.members = members;
    }
}

fn about_snapshot() -> AboutSnapshot {
    AboutSnapshot {
        app_version: APP_VERSION.to_owned(),
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        meow_rs_version: MEOW_RS_VERSION.to_owned(),
        arkit_rev: ARKIT_REV.to_owned(),
        rust_version: RUST_VERSION.to_owned(),
        privacy_summary: vec![
            "订阅内容下载后保存到应用私有目录，解析和运行时改写均在本机完成。".to_owned(),
            "本地 YAML、备份、节点选择和规则缓存不会上传到第三方服务。".to_owned(),
            "连接、日志、DNS 查询计数和流量统计仅用于本机页面展示与排错。".to_owned(),
        ],
    }
}

fn active_connections_from_tunnel(tunnel: &Tunnel) -> Vec<ConnectionSummary> {
    let mut connections: Vec<_> = tunnel
        .statistics()
        .active_connections()
        .into_iter()
        .map(|connection| {
            let chains: Vec<String> = connection
                .chains
                .into_iter()
                .map(|chain| chain.to_string())
                .collect();
            let proxy = if chains.is_empty() {
                "DIRECT".to_owned()
            } else {
                chains.join(" > ")
            };
            let rule_payload = connection.rule_payload.to_string();
            let rule = if rule_payload.is_empty() {
                connection.rule.to_string()
            } else {
                format!("{}({})", connection.rule, rule_payload)
            };
            ConnectionSummary {
                id: connection.id.to_string(),
                host: connection.metadata.remote_address().to_string(),
                domain: connection.metadata.rule_host().to_owned(),
                destination_ip: connection
                    .metadata
                    .dst_ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_default(),
                destination_port: connection.metadata.dst_port,
                network: connection.metadata.network.to_string(),
                rule,
                rule_payload,
                proxy,
                chains,
                started_at: connection.start.to_string(),
                upload_bytes: non_negative_i64_to_u64(connection.counters.upload_bytes()),
                download_bytes: non_negative_i64_to_u64(connection.counters.download_bytes()),
            }
        })
        .collect();
    connections.sort_by(|a, b| a.host.cmp(&b.host).then_with(|| a.id.cmp(&b.id)));
    connections
}

fn record_request_history(state: &mut CoreState, connections: &[ConnectionSummary]) {
    let now = system_time_secs(SystemTime::now()).unwrap_or_else(|| "now".to_owned());
    for request in &mut state.request_history {
        request.active = false;
    }

    for connection in connections {
        if let Some(request) = state
            .request_history
            .iter_mut()
            .find(|request| request.id == connection.id)
        {
            request.host = connection.host.clone();
            request.domain = connection.domain.clone();
            request.destination_ip = connection.destination_ip.clone();
            request.destination_port = connection.destination_port;
            request.network = connection.network.clone();
            request.rule = connection.rule.clone();
            request.proxy = connection.proxy.clone();
            request.upload_bytes = connection.upload_bytes;
            request.download_bytes = connection.download_bytes;
            request.active = true;
            request.updated_at = now.clone();
            continue;
        }

        state.request_history.push_back(RequestSummary {
            id: connection.id.clone(),
            host: connection.host.clone(),
            domain: connection.domain.clone(),
            destination_ip: connection.destination_ip.clone(),
            destination_port: connection.destination_port,
            network: connection.network.clone(),
            rule: connection.rule.clone(),
            proxy: connection.proxy.clone(),
            upload_bytes: connection.upload_bytes,
            download_bytes: connection.download_bytes,
            active: true,
            updated_at: now.clone(),
        });
        while state.request_history.len() > MAX_REQUEST_HISTORY {
            state.request_history.pop_front();
        }
    }
}

fn non_negative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

fn controller_url(addr: SocketAddr, segments: &[&str]) -> Result<reqwest::Url, HMetaError> {
    let mut url = reqwest::Url::parse(&format!("http://{addr}/"))
        .map_err(|err| HMetaError::Core(format!("invalid controller URL: {err}")))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| HMetaError::Core("controller URL cannot be a base".to_owned()))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn controller_credentials(state: &CoreState) -> Option<(SocketAddr, Option<String>)> {
    let controller = state.api_controller.as_ref()?;
    let secret = controller
        .raw_config
        .read()
        .secret
        .clone()
        .filter(|secret| !secret.is_empty());
    Some((controller.addr, secret))
}

fn proxy_groups_from_tunnel(tunnel: &Tunnel) -> Vec<ProxyGroup> {
    let route = tunnel.route_snapshot();
    let proxies = &route.proxies;
    let mut groups: Vec<_> = proxies
        .values()
        .filter_map(|proxy| {
            let members = proxy.members()?;
            let selected = proxy.current();
            Some(ProxyGroup {
                name: proxy.name().to_owned(),
                group_type: proxy.adapter_type().to_string(),
                selected: selected.clone(),
                fixed: proxy.selection().and_then(|selection| selection.fixed()),
                proxies: members
                    .into_iter()
                    .map(|name| {
                        proxy_item(
                            proxies.get(name.as_str()),
                            &name,
                            selected.as_deref() == Some(name.as_str()),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}

fn refresh_proxy_groups_preserving_order(state: &mut CoreState, tunnel: &Tunnel) {
    let mut refreshed = proxy_groups_from_tunnel(tunnel);
    preserve_proxy_group_member_order(&state.proxy_groups, &mut refreshed);
    state.proxy_groups = refreshed;
}

fn preserve_proxy_group_member_order(previous: &[ProxyGroup], refreshed: &mut [ProxyGroup]) {
    for group in refreshed {
        let Some(previous_group) = previous
            .iter()
            .find(|candidate| candidate.name == group.name)
        else {
            continue;
        };
        let positions = previous_group
            .proxies
            .iter()
            .enumerate()
            .map(|(index, proxy)| (proxy.name.as_str(), index))
            .collect::<std::collections::HashMap<_, _>>();
        // Rust's stable sort also preserves the tunnel order of newly added
        // provider nodes after every member already present in the snapshot.
        group.proxies.sort_by_key(|proxy| {
            positions
                .get(proxy.name.as_str())
                .copied()
                .map_or((1, usize::MAX), |index| (0, index))
        });
    }
}

fn proxy_item(
    proxy: Option<&Arc<dyn meow_common::Proxy>>,
    name: &str,
    selected: bool,
) -> ProxyItem {
    ProxyItem {
        name: name.to_owned(),
        proxy_type: proxy
            .map(|proxy| proxy.adapter_type().to_string())
            .unwrap_or_else(|| "Unknown".to_owned()),
        delay_ms: proxy.and_then(|proxy| {
            let delay = proxy.last_delay();
            (delay > 0).then_some(u32::from(delay))
        }),
        selected,
    }
}

fn profile_name_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Subscription".to_owned())
}

fn subscription_userinfo_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<hmeta_model::SubscriptionUserInfo> {
    headers
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .and_then(hmeta_profile::parse_subscription_userinfo)
}

fn subscription_metadata_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<hmeta_model::SubscriptionMetadata> {
    let content_disposition_title = header_str(headers, "content-disposition")
        .and_then(hmeta_profile::parse_content_disposition_filename);
    let title = header_str(headers, "profile-title").or(content_disposition_title.as_deref());
    hmeta_profile::parse_subscription_metadata(
        title,
        header_str(headers, "profile-update-interval"),
        header_str(headers, "profile-web-page-url"),
        header_str(headers, "support-url"),
    )
}

fn subscription_profile_name_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    header_str(headers, "profile-title")
        .and_then(hmeta_profile::decode_subscription_header_text)
        .or_else(|| {
            header_str(headers, "content-disposition")
                .and_then(hmeta_profile::parse_content_disposition_filename)
        })
}

fn header_str<'a>(headers: &'a reqwest::header::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn info_log(message: impl Into<String>) -> LogEntry {
    LogEntry {
        level: "info".to_owned(),
        message: message.into(),
        timestamp: "now".to_owned(),
    }
}

fn warning_log(message: impl Into<String>) -> LogEntry {
    LogEntry {
        level: "warning".to_owned(),
        message: message.into(),
        timestamp: "now".to_owned(),
    }
}

fn install_runtime_log_layer() {
    INSTALL_RUNTIME_LOG_LAYER.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(HMetaLogLayer {
            logs: RUNTIME_LOGS.clone(),
        });
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

struct HMetaLogLayer {
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl<S> Layer<S> for HMetaLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = match *event.metadata().level() {
            Level::TRACE | Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warning",
            Level::ERROR => "error",
        };
        let mut visitor = LogMessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.finish(event.metadata().target());
        if is_vpn_log_target(event.metadata().target()) {
            push_runtime_log(
                &self.logs,
                LogEntry {
                    level: level.to_owned(),
                    message,
                    timestamp: unix_timestamp_string(),
                },
            );
        }
        if let Ok(senders) = API_LOG_TXS.lock() {
            for tx in senders.iter() {
                let _ = tx.send(meow_api::log_stream::LogMessage {
                    level: meow_log_level(*event.metadata().level()),
                    payload: visitor_payload(event),
                    time: time::OffsetDateTime::now_utc(),
                });
            }
        }
    }
}

fn is_vpn_log_target(target: &str) -> bool {
    target.starts_with("hmeta_core")
        || target.starts_with("hmeta_vpn")
        || target.starts_with("meow_")
        || target.starts_with("meow-")
}

fn meow_log_level(level: Level) -> meow_api::log_stream::LogLevel {
    match level {
        Level::TRACE | Level::DEBUG => meow_api::log_stream::LogLevel::Debug,
        Level::INFO => meow_api::log_stream::LogLevel::Info,
        Level::WARN => meow_api::log_stream::LogLevel::Warning,
        Level::ERROR => meow_api::log_stream::LogLevel::Error,
    }
}

fn visitor_payload(event: &tracing::Event<'_>) -> String {
    let mut visitor = LogMessageVisitor::default();
    event.record(&mut visitor);
    visitor.finish(event.metadata().target())
}

#[derive(Default)]
struct LogMessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl LogMessageVisitor {
    fn finish(self, fallback: &str) -> String {
        let mut message = self.message.unwrap_or_else(|| fallback.to_owned());
        if !self.fields.is_empty() {
            message.push_str(" · ");
            message.push_str(&self.fields.join(", "));
        }
        message
    }
}

impl tracing::field::Visit for LogMessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_owned());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

fn push_runtime_log(logs: &Mutex<VecDeque<LogEntry>>, entry: LogEntry) {
    if let Ok(mut logs) = logs.lock() {
        if logs.len() >= MAX_RUNTIME_LOGS {
            logs.pop_front();
        }
        logs.push_back(entry);
    }
}

fn merged_logs(state_logs: &[LogEntry]) -> Vec<LogEntry> {
    let state_start = state_logs.len().saturating_sub(MAX_RUNTIME_LOGS);
    let mut logs: Vec<_> = state_logs[state_start..].to_vec();
    let remaining = MAX_RUNTIME_LOGS.saturating_sub(logs.len());
    if let Ok(runtime_logs) = RUNTIME_LOGS.lock() {
        let runtime_start = runtime_logs.len().saturating_sub(remaining);
        logs.extend(runtime_logs.iter().skip(runtime_start).cloned());
    }
    logs
}

fn merge_platform_logs(mut local: Vec<LogEntry>, platform: &[LogEntry]) -> Vec<LogEntry> {
    for entry in platform {
        if !local.iter().any(|existing| {
            existing.level == entry.level
                && existing.message == entry.message
                && existing.timestamp == entry.timestamp
        }) {
            local.push(entry.clone());
        }
    }
    if local.len() > MAX_RUNTIME_LOGS {
        local.drain(..local.len() - MAX_RUNTIME_LOGS);
    }
    local
}

fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn system_time_secs(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().to_string())
}

fn meow_version_marker() -> String {
    let _ = std::any::type_name::<meow_tunnel::Tunnel>();
    format!("meow-rs@{MEOW_RS_VERSION}")
}

fn mode_to_tunnel(value: RuntimeMode) -> TunnelMode {
    match value {
        RuntimeMode::Rule => TunnelMode::Rule,
        RuntimeMode::Global => TunnelMode::Global,
        RuntimeMode::Direct => TunnelMode::Direct,
    }
}

fn mode_from_tunnel(value: TunnelMode) -> RuntimeMode {
    match value {
        TunnelMode::Rule => RuntimeMode::Rule,
        TunnelMode::Global => RuntimeMode::Global,
        TunnelMode::Direct => RuntimeMode::Direct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use futures::StreamExt;

    static TEST_LOG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn track_test_connection(tunnel: &Tunnel, host: &str) -> String {
        tunnel
            .statistics()
            .track_connection(
                Metadata {
                    network: Network::Tcp,
                    conn_type: ConnType::Inner,
                    host: host.into(),
                    dst_port: 443,
                    ..Metadata::default()
                },
                "DOMAIN".into(),
                host.into(),
                std::iter::once(Arc::<str>::from("DIRECT")).collect(),
            )
            .to_string()
    }

    #[test]
    fn core_snapshot_is_json() {
        let core = CoreHandle::new();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.proxy_groups.is_empty());
        let json = to_json(&snapshot).unwrap();
        assert!(json.contains("proxyGroups"));
        assert!(json.contains("vpnLifecycle"));
        assert!(json.contains("networkProtected"));
        assert!(json.contains("trafficHistory"));
        assert!(json.contains("handledPackets"));
        assert!(json.contains("meowRsVersion"));
        assert!(json.contains("privacySummary"));
        assert!(json.contains("geodata"));
        assert_eq!(snapshot.dns.listen, "127.0.0.1:1053");
        assert!(snapshot.dns.hijacking);
        assert_eq!(snapshot.geodata.len(), 3);
        assert!(snapshot
            .geodata
            .iter()
            .any(|file| file.path.ends_with("geosite.dat")));
        assert_eq!(snapshot.about.app_version, APP_VERSION);
        assert_eq!(snapshot.about.meow_rs_version, MEOW_RS_VERSION);
        assert_eq!(snapshot.about.arkit_rev, ARKIT_REV);
        assert!(!snapshot.about.privacy_summary.is_empty());
    }

    #[test]
    fn proxy_selection_refresh_preserves_the_existing_member_order() {
        let item = |name: &str, selected: bool| ProxyItem {
            name: name.to_owned(),
            proxy_type: "VLESS".to_owned(),
            delay_ms: None,
            selected,
        };
        let previous = vec![ProxyGroup {
            name: "GLOBAL".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("Tokyo 04".to_owned()),
            fixed: None,
            proxies: vec![
                item("Tokyo 04", true),
                item("DIRECT", false),
                item("Tokyo 01", false),
                item("Tokyo 02", false),
            ],
        }];
        let mut refreshed = vec![ProxyGroup {
            name: "GLOBAL".to_owned(),
            group_type: "Selector".to_owned(),
            selected: Some("Tokyo 01".to_owned()),
            fixed: None,
            proxies: vec![
                item("Tokyo 01", true),
                item("DIRECT", false),
                item("Tokyo 02", false),
                item("Tokyo 04", false),
            ],
        }];

        preserve_proxy_group_member_order(&previous, &mut refreshed);

        assert_eq!(
            refreshed[0]
                .proxies
                .iter()
                .map(|proxy| proxy.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Tokyo 04", "DIRECT", "Tokyo 01", "Tokyo 02"]
        );
        assert_eq!(refreshed[0].selected.as_deref(), Some("Tokyo 01"));
        assert!(refreshed[0].proxies[2].selected);
    }

    #[test]
    fn traffic_history_is_bounded_and_exposed_in_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-traffic-history-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        {
            let mut state = core.lock_state().unwrap();
            for speed in 0..40 {
                state.traffic.download_speed = speed;
                state.traffic.upload_speed = speed * 2;
                record_traffic_history(&mut state);
            }
            state.traffic.download_speed = 99;
            state.traffic.upload_speed = 199;
        }

        let snapshot = core.snapshot().unwrap();

        assert_eq!(snapshot.traffic_history.len(), MAX_TRAFFIC_HISTORY);
        assert_eq!(snapshot.traffic_history[0].download_speed, 9);
        assert_eq!(snapshot.traffic_history[30].download_speed, 39);
        let latest = snapshot.traffic_history.last().unwrap();
        assert_eq!(latest.download_speed, 0);
        assert_eq!(latest.upload_speed, 0);
    }

    #[test]
    fn mode_changes_are_reflected() {
        let core = CoreHandle::new();
        core.set_mode(RuntimeMode::Direct).unwrap();
        assert_eq!(core.snapshot().unwrap().mode, RuntimeMode::Direct);
    }

    #[test]
    fn platform_vpn_status_is_reflected_in_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-platform-vpn-status-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(&root);
        let snapshot = core.snapshot().unwrap();
        assert!(!snapshot.engine_loaded);
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Stopped);
        assert!(!snapshot.running);
        assert!(!snapshot.vpn_running);
        assert!(!snapshot.network_protected);
        core.set_platform_vpn_starting(true).unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(!snapshot.engine_loaded);
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Starting);
        assert!(!snapshot.running);
        assert!(!snapshot.vpn_running);
        assert!(!snapshot.network_protected);
        core.set_platform_vpn_running(true).unwrap();
        core.set_platform_network_protected(true, None).unwrap();
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Connected);
        assert!(snapshot.vpn_running);
        assert!(snapshot.network_protected);
        assert!(snapshot.network_protect_error.is_none());
        core.set_platform_vpn_running(false).unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(!snapshot.engine_loaded);
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Stopped);
        assert!(!snapshot.running);
        assert!(!snapshot.vpn_running);
        assert!(!snapshot.network_protected);
        assert!(snapshot.network_protect_error.is_none());
        core.set_platform_network_protected(false, Some("denied".to_owned()))
            .unwrap();
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Failed);
        assert!(!snapshot.network_protected);
        assert_eq!(snapshot.network_protect_error.as_deref(), Some("denied"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn platform_vpn_state_is_shared_between_ui_and_extension_process_handles() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-platform-vpn-ipc-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ui = CoreHandle::new_with_profile_root(&root);
        let extension = CoreHandle::new_with_profile_root(&root);

        ui.set_platform_vpn_starting(true).unwrap();
        let extension_snapshot = extension.snapshot().unwrap();
        assert_eq!(extension_snapshot.vpn_lifecycle, VpnLifecycle::Starting);
        assert!(!extension_snapshot.vpn_running);

        extension
            .set_platform_network_protected(true, None)
            .unwrap();
        extension.set_platform_vpn_running(true).unwrap();
        let ui_snapshot = ui.snapshot().unwrap();
        assert_eq!(ui_snapshot.vpn_lifecycle, VpnLifecycle::Connected);
        assert!(ui_snapshot.vpn_running);
        assert!(ui_snapshot.network_protected);
        assert!(!ui.expire_platform_vpn_start().unwrap());

        extension.stop_vpn().unwrap();
        let ui_snapshot = ui.snapshot().unwrap();
        assert_eq!(ui_snapshot.vpn_lifecycle, VpnLifecycle::Stopped);
        assert!(!ui_snapshot.vpn_running);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn platform_vpn_telemetry_is_visible_to_the_ui_process() {
        let root =
            std::env::temp_dir().join(format!("hmeta-platform-vpn-telemetry-{}", now_unix_nanos()));
        let extension = CoreHandle::new_with_profile_root(&root);
        let profile_id = extension
            .import_profile_from_content(
                "Telemetry",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let telemetry = PlatformVpnTelemetry {
            updated_at: now_unix_nanos(),
            active_profile: Some(profile_id.clone()),
            traffic: TrafficSnapshot {
                upload_bytes: 321,
                download_bytes: 654,
                upload_speed: 32,
                download_speed: 65,
                ..TrafficSnapshot::default()
            },
            traffic_history: vec![TrafficHistoryPoint {
                upload_speed: 32,
                download_speed: 65,
            }],
            dns: DnsSnapshot {
                handled_packets: 7,
                ..DnsSnapshot::default()
            },
            profile_upload_bytes: 321,
            profile_download_bytes: 654,
            ..PlatformVpnTelemetry::default()
        };
        persist_platform_vpn_telemetry_at(&root.join(PLATFORM_VPN_TELEMETRY_FILE), &telemetry)
            .unwrap();

        let ui = CoreHandle::new_with_profile_root(&root);
        let snapshot = ui.snapshot().unwrap();
        assert_eq!(snapshot.traffic.upload_bytes, 321);
        assert_eq!(snapshot.traffic.download_bytes, 654);
        assert_eq!(snapshot.traffic.upload_speed, 32);
        assert_eq!(snapshot.traffic.download_speed, 65);
        assert_eq!(snapshot.dns.handled_packets, 7);
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .unwrap();
        assert_eq!(profile.upload_bytes, 321);
        assert_eq!(profile.download_bytes, 654);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn platform_vpn_start_timeout_becomes_visible_failure() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-platform-vpn-timeout-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(&root);
        core.set_platform_vpn_starting(true).unwrap();
        assert!(core.expire_platform_vpn_start().unwrap());
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Failed);
        assert!(!snapshot.vpn_running);
        assert!(snapshot
            .network_protect_error
            .as_deref()
            .is_some_and(|error| error.contains("startup timeout")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn vpn_lifecycle_derives_service_state_from_engine_vpn_and_protect_status() {
        assert_eq!(
            vpn_lifecycle(false, false, false, false, false, None),
            VpnLifecycle::Stopped
        );
        assert_eq!(
            vpn_lifecycle(true, false, false, false, false, None),
            VpnLifecycle::EngineLoaded
        );
        assert_eq!(
            vpn_lifecycle(true, true, false, false, false, None),
            VpnLifecycle::Starting
        );
        assert_eq!(
            vpn_lifecycle(true, false, true, false, true, None),
            VpnLifecycle::Connected
        );
        assert_eq!(
            vpn_lifecycle(true, false, true, false, false, Some("denied")),
            VpnLifecycle::ProtectFailed
        );
        assert_eq!(
            vpn_lifecycle(true, false, false, false, false, Some("denied")),
            VpnLifecycle::Failed
        );
    }

    #[tokio::test]
    async fn meow_crate_feature_matrix_loads_reference_client_protocols() {
        let yaml = r#"
mixed-port: 7890
external-controller: 127.0.0.1:0
proxies:
  - name: SS
    type: ss
    server: 127.0.0.1
    port: 8388
    cipher: aes-128-gcm
    password: test-password
  - name: Trojan
    type: trojan
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: VLESS
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: 00000000-0000-0000-0000-000000000001
  - name: AnyTLS
    type: anytls
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: VMess
    type: vmess
    server: 127.0.0.1
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    cipher: auto
  - name: Snell
    type: snell
    server: 127.0.0.1
    port: 8388
    psk: test-password
    version: 4
  - name: Hysteria2
    type: hysteria2
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: HTTP
    type: http
    server: 127.0.0.1
    port: 8080
  - name: SOCKS5
    type: socks5
    server: 127.0.0.1
    port: 1080
proxy-groups:
  - name: Proxy
    type: select
    proxies: [SS, Trojan, VLESS, AnyTLS, VMess, Snell, Hysteria2, HTTP, SOCKS5, DIRECT]
rules:
  - MATCH,Proxy
"#;
        let config = load_meow_config(yaml).await.unwrap();
        for proxy in [
            "SS",
            "Trojan",
            "VLESS",
            "AnyTLS",
            "VMess",
            "Snell",
            "Hysteria2",
            "HTTP",
            "SOCKS5",
        ] {
            assert!(
                config.proxies.contains_key(proxy),
                "meow config omitted enabled proxy type {proxy}"
            );
        }
    }

    #[test]
    fn snapshot_includes_runtime_tracing_logs() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let core = CoreHandle::new();
        let message = format!(
            "hmeta runtime log test {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        tracing::warn!(target: "hmeta_core_test", "{}", message);

        let snapshot = core.snapshot().unwrap();
        assert!(snapshot
            .logs
            .iter()
            .any(|log| log.level == "warning" && log.message.contains(&message)));
    }

    #[test]
    fn runtime_log_page_excludes_arkit_framework_targets() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let core = CoreHandle::new();
        let message = format!("arkit framework log {}", now_unix_nanos());

        tracing::warn!(target: "arkit::renderer", "{}", message);

        assert!(!core
            .snapshot()
            .unwrap()
            .logs
            .iter()
            .any(|log| log.message.contains(&message)));
        assert!(is_vpn_log_target("hmeta_vpn::tun"));
        assert!(is_vpn_log_target("meow_tunnel::dispatcher"));
        assert!(!is_vpn_log_target("arkit::renderer"));
    }

    #[test]
    fn clear_logs_removes_state_and_runtime_logs() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let core = CoreHandle::new();
        {
            let mut state = core.lock_state().unwrap();
            state.logs.push(warning_log("state log to clear"));
        }
        tracing::warn!(target: "hmeta_core_test", "runtime log to clear");
        assert!(!core.snapshot().unwrap().logs.is_empty());

        core.clear_logs().unwrap();

        assert!(core.snapshot().unwrap().logs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_loads_engine_without_marking_vpn_connected() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-reload-state-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();

        core.reload_config(&profile_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.engine_loaded);
        assert!(snapshot.running);
        assert!(!snapshot.vpn_running);
        assert!(!snapshot.rules.is_empty());
        assert!(snapshot
            .rules
            .iter()
            .any(|rule| rule.source == "profile-yaml" && rule.enabled));
        assert_eq!(snapshot.profiles[0].rule_count, snapshot.rules.len());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn manual_activity_rules_persist_and_hot_update_the_existing_tunnel() {
        let root =
            std::env::temp_dir().join(format!("hmeta-core-manual-rule-test-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(root.clone());
        let profile_id = core
            .import_profile_from_content(
                "Manual rule",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();
        let original_inner = {
            let state = core.lock_state().unwrap();
            Arc::clone(state.tunnel.as_ref().unwrap().inner())
        };

        let added = core
            .apply_manual_rule(
                &profile_id,
                &ManualRuleSpec {
                    match_kind: hmeta_model::ManualRuleMatchKind::Domain,
                    value: "API.Example.COM.".to_owned(),
                    target: "Proxy".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            added.mutation.kind,
            hmeta_model::ManualRuleMutationKind::Added
        );
        assert_eq!(added.mutation.line, "DOMAIN,api.example.com,Proxy");
        assert!(added.live_updated);
        assert!(added.rule_mode_active);

        let updated = core
            .apply_manual_rule(
                &profile_id,
                &ManualRuleSpec {
                    match_kind: hmeta_model::ManualRuleMatchKind::Domain,
                    value: "api.example.com".to_owned(),
                    target: "DIRECT".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.mutation.rule_id, added.mutation.rule_id);
        assert_eq!(
            updated.mutation.kind,
            hmeta_model::ManualRuleMutationKind::Updated
        );

        let snapshot = core.snapshot().unwrap();
        let matching = snapshot
            .rules
            .iter()
            .filter(|rule| rule.line.starts_with("DOMAIN,api.example.com,"))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].line, "DOMAIN,api.example.com,DIRECT");
        let current_inner = {
            let state = core.lock_state().unwrap();
            Arc::clone(state.tunnel.as_ref().unwrap().inner())
        };
        assert!(Arc::ptr_eq(&original_inner, &current_inner));

        let reopened = ProfileStore::open(&root).unwrap();
        assert!(reopened
            .rules_for_profile(&profile_id)
            .iter()
            .any(|rule| rule.line == "DOMAIN,api.example.com,DIRECT"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vpn_prepare_reuses_an_already_loaded_tunnel() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-vpn-prepare-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = Arc::new(CoreHandle::new_with_profile_root(root));
        core.import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();

        let (first_prepare, second_prepare) =
            tokio::join!(core.prepare_active_vpn(), core.prepare_active_vpn(),);
        assert_ne!(first_prepare.unwrap(), second_prepare.unwrap());
        let reloads_after_cold_prepare = core
            .snapshot()
            .unwrap()
            .logs
            .iter()
            .filter(|log| log.message.starts_with("config reloaded from profile"))
            .count();

        assert!(!core.prepare_active_vpn().await.unwrap());
        let reloads_after_warm_prepare = core
            .snapshot()
            .unwrap()
            .logs
            .iter()
            .filter(|log| log.message.starts_with("config reloaded from profile"))
            .count();
        assert_eq!(reloads_after_warm_prepare, reloads_after_cold_prepare);
        assert_eq!(reloads_after_cold_prepare, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_ignores_subscription_geodata_auto_update_fields() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-geodata-clean-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Geodata Auto Update",
                "test",
                r#"
mixed-port: 7890
geodata:
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
                None,
            )
            .await
            .unwrap();

        core.reload_config(&profile_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.engine_loaded);
        assert!(snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .is_some_and(|profile| profile.runtime_yaml_path.ends_with(".yaml")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_ignores_app_managed_listener_and_dns_validation_fields() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-managed-validation-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Managed Fields",
                "test",
                r#"
port: 7890
mixed-port: 7890
external-controller: 0.0.0.0:9090
listeners:
  - name: duplicated
    type: mixed
    port: 7890
dns:
  enable: true
  listen: 0.0.0.0:53
  default-nameserver:
    - bad bootstrap
  nameserver:
    - 223.5.5.5
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fallback-filter:
    geoip: true
  use-system-hosts: true
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
                None,
            )
            .await
            .unwrap();

        core.reload_config(&profile_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.engine_loaded);
        assert_eq!(snapshot.dns.listen, "127.0.0.1:1053");
        assert_eq!(snapshot.vpn_options.dns_servers, vec!["223.5.5.5"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vpn_lifecycle_reloads_tunnel_starts_and_stops() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-vpn-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let mut fds = [0_i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let options_json = to_json(&VpnOptions::default()).unwrap();
        core.start_vpn(fds[0], &options_json).await.unwrap();
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }

        let running = core.snapshot().unwrap();
        assert!(running.engine_loaded);
        assert!(running.running);
        assert!(running.vpn_running);
        assert_eq!(running.vpn_options.mtu, VpnOptions::default().mtu);

        core.stop_vpn().unwrap();
        let stopped = core.snapshot().unwrap();
        assert!(stopped.engine_loaded);
        assert!(stopped.running);
        assert!(!stopped.vpn_running);
        assert_eq!(stopped.traffic.upload_speed, 0);
        assert_eq!(stopped.traffic.download_speed, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dns_config_updates_reload_active_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-dns-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "DNS",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        core.set_profile_dns_config(
            &profile_id,
            vec!["https://dns.alidns.com/dns-query".to_owned()],
            vec!["https://dns.google/dns-query".to_owned()],
            BTreeMap::from([(
                "geosite:cn".to_owned(),
                vec!["https://dns.alidns.com/dns-query".to_owned()],
            )]),
        )
        .await
        .unwrap();

        let snapshot = core.snapshot().unwrap();
        assert_eq!(
            snapshot.vpn_options.dns_servers,
            vec!["https://dns.alidns.com/dns-query"]
        );
        assert_eq!(snapshot.dns.fallbacks, vec!["https://dns.google/dns-query"]);
        assert_eq!(
            snapshot
                .dns
                .nameserver_policy
                .get("geosite:cn")
                .cloned()
                .unwrap_or_default(),
            vec!["https://dns.alidns.com/dns-query"]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vpn_config_updates_reload_active_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-vpn-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "VPN",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        core.set_profile_vpn_config(&profile_id, true, false, true, "lwip".to_owned())
            .await
            .unwrap();

        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.vpn_options.system_proxy);
        assert!(!snapshot.vpn_options.dns_hijacking);
        assert!(snapshot.vpn_options.allow_bypass);
        assert_eq!(snapshot.vpn_options.stack, "lwip");
        assert!(!snapshot.dns.hijacking);
    }

    #[test]
    fn dns_snapshot_exposes_tun_cache_diagnostics() {
        let stats = TunStats {
            dns_packets: 7,
            dns_cache_hits: 3,
            dns_cache_misses: 4,
            ..TunStats::default()
        };
        let snapshot = dns_snapshot(&VpnOptions::default(), Some(&stats));

        assert_eq!(snapshot.handled_packets, 7);
        assert_eq!(snapshot.cache_hits, 3);
        assert_eq!(snapshot.cache_misses, 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_proxy_delay_reaches_local_tcp_listener() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-direct-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_handle = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let delay = core
            .test_proxy_delay("DIRECT", Some(&format!("http://{addr}")), Some(1000))
            .await
            .unwrap();
        assert!(delay < 1000);
        accept_handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_proxy_echo_roundtrips_local_tcp_payload() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-direct-echo-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut payload = vec![0_u8; "hmeta-echo-payload".len()];
            stream.read_exact(&mut payload).await.unwrap();
            stream.write_all(&payload).await.unwrap();
        });

        let echoed = core
            .test_proxy_echo(
                "DIRECT",
                &format!("http://{addr}"),
                "hmeta-echo-payload",
                Some(1000),
            )
            .await
            .unwrap();
        assert_eq!(echoed, "hmeta-echo-payload");
        accept_handle.await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot
            .logs
            .iter()
            .any(|log| { log.message.contains("DIRECT echo roundtrip: 18 bytes") }));
    }

    #[test]
    fn proxy_echo_metadata_uses_an_opaque_tcp_tunnel() {
        let metadata = proxy_test_metadata("http://127.0.0.1:8080", "hmeta-echo").unwrap();
        assert_eq!(metadata.conn_type, ConnType::Inner);
        assert_eq!(metadata.host.as_str(), "127.0.0.1");
        assert_eq!(metadata.dst_port, 8080);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_uses_meow_tunnel_statistics_for_connections_and_traffic() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-meow-stats-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let tunnel = {
            let state = core.lock_state().unwrap();
            state.tunnel.clone().expect("loaded tunnel")
        };
        tunnel.statistics().add_upload(128);
        tunnel.statistics().add_download(256);
        let connection_id = track_test_connection(&tunnel, "example.com");

        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.traffic.meow_upload_bytes, 128);
        assert_eq!(snapshot.traffic.meow_download_bytes, 256);
        assert_eq!(snapshot.traffic.upload_bytes, 128);
        assert_eq!(snapshot.traffic.download_bytes, 256);
        assert_eq!(snapshot.connections.len(), 1);
        let connection = &snapshot.connections[0];
        assert_eq!(connection.id, connection_id);
        assert_eq!(connection.host, "example.com:443");
        assert_eq!(connection.network, "tcp");
        assert_eq!(connection.rule, "DOMAIN(example.com)");
        assert_eq!(connection.rule_payload, "example.com");
        assert_eq!(connection.proxy, "DIRECT");
        assert_eq!(connection.chains, vec!["DIRECT"]);
        assert!(!connection.started_at.is_empty());
        assert_eq!(connection.started_at.len(), 20);
        assert_eq!(connection.started_at.as_bytes().get(10), Some(&b'T'));
        assert!(connection.started_at.ends_with('Z'));
        assert_eq!(snapshot.request_history.len(), 1);
        let request = &snapshot.request_history[0];
        assert_eq!(request.id, connection_id);
        assert_eq!(request.host, "example.com:443");
        assert_eq!(request.network, "tcp");
        assert_eq!(request.rule, "DOMAIN(example.com)");
        assert_eq!(request.proxy, "DIRECT");
        assert!(request.active);

        core.close_connection(&connection_id).unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.connections.is_empty());
        assert_eq!(snapshot.request_history.len(), 1);
        assert_eq!(snapshot.request_history[0].id, connection_id);
        assert!(!snapshot.request_history[0].active);

        core.clear_request_history().unwrap();
        assert!(core.snapshot().unwrap().request_history.is_empty());

        let first = track_test_connection(&tunnel, "one.example");
        let second = track_test_connection(&tunnel, "two.example");
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.connections.len(), 2);
        assert!(snapshot.request_history.iter().any(|item| item.id == first));
        assert!(snapshot
            .request_history
            .iter()
            .any(|item| item.id == second));

        core.close_all_connections().unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.connections.is_empty());
        assert_eq!(snapshot.request_history.len(), 2);
        assert!(snapshot.request_history.iter().all(|item| !item.active));
        assert!(snapshot
            .logs
            .iter()
            .any(|log| log.message == "all connections closed: 2"));
    }

    #[test]
    fn tun_descriptor_rx_is_upload_and_tx_is_download() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-tun-direction-test-{}",
            now_unix_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(&root);
        {
            let mut state = core.lock_state().unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    rx_bytes: 340,
                    tx_bytes: 120,
                    ..TunStats::default()
                },
            )
            .unwrap();
            assert_eq!(state.traffic.upload_bytes, 340);
            assert_eq!(state.traffic.download_bytes, 120);
            assert_eq!(state.traffic.tun_upload_bytes, 340);
            assert_eq!(state.traffic.tun_download_bytes, 120);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_traffic_is_not_double_counted_after_vpn_stop_baseline() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-traffic-stop-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();
        let tunnel = {
            let state = core.lock_state().unwrap();
            state.tunnel.clone().expect("loaded tunnel")
        };
        tunnel.statistics().add_upload(128);
        tunnel.statistics().add_download(256);

        {
            let mut state = core.lock_state().unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 128,
                    rx_bytes: 256,
                    ..TunStats::default()
                },
            )
            .unwrap();
            baseline_meow_traffic_sample(&mut state);
        }

        let snapshot = core.snapshot().unwrap();
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile summary");
        assert_eq!(profile.upload_bytes, 256);
        assert_eq!(profile.download_bytes, 128);
        // With no live native TUN handle this snapshot intentionally falls
        // back to meow's already-semantic upload/download counters.
        assert_eq!(snapshot.traffic.upload_bytes, 128);
        assert_eq!(snapshot.traffic.download_bytes, 256);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_switch_settles_tun_traffic_to_previous_profile() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-profile-switch-tun-traffic-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let first_id = core
            .import_profile_from_content(
                "First",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let second_id = core
            .import_profile_from_content(
                "Second",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();

        {
            let mut state = core.lock_state().unwrap();
            state.profiles.set_active(&first_id).unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 100,
                    rx_bytes: 200,
                    ..TunStats::default()
                },
            )
            .unwrap();
            settle_traffic_before_profile_switch(
                &mut state,
                Some(&TunStats {
                    tx_bytes: 150,
                    rx_bytes: 260,
                    ..TunStats::default()
                }),
            )
            .unwrap();
            state.profiles.set_active(&second_id).unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 180,
                    rx_bytes: 300,
                    ..TunStats::default()
                },
            )
            .unwrap();
        }

        let snapshot = core.snapshot().unwrap();
        let first = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == first_id)
            .expect("first profile");
        let second = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == second_id)
            .expect("second profile");
        assert_eq!(first.upload_bytes, 260);
        assert_eq!(first.download_bytes, 150);
        assert_eq!(second.upload_bytes, 40);
        assert_eq!(second.download_bytes, 30);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_switch_settles_meow_traffic_when_native_stats_are_unavailable() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-profile-switch-meow-traffic-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let first_id = core
            .import_profile_from_content(
                "First",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let second_id = core
            .import_profile_from_content(
                "Second",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&first_id).await.unwrap();
        let tunnel = {
            let state = core.lock_state().unwrap();
            state.tunnel.clone().expect("loaded tunnel")
        };
        tunnel.statistics().add_upload(320);
        tunnel.statistics().add_download(640);

        {
            let mut state = core.lock_state().unwrap();
            settle_traffic_before_profile_switch(&mut state, None).unwrap();
            state.profiles.set_active(&second_id).unwrap();
        }

        let snapshot = core.snapshot().unwrap();
        let first = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == first_id)
            .expect("first profile");
        let second = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == second_id)
            .expect("second profile");
        assert_eq!(first.upload_bytes, 320);
        assert_eq!(first.download_bytes, 640);
        assert_eq!(second.upload_bytes, 0);
        assert_eq!(second.download_bytes, 0);
        assert_eq!(snapshot.traffic.upload_speed, 0);
        assert_eq!(snapshot.traffic.download_speed, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deleting_active_profile_settles_traffic_baseline_before_next_profile() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-profile-delete-traffic-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let first_id = core
            .import_profile_from_content(
                "First",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let second_id = core
            .import_profile_from_content(
                "Second",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();

        {
            let mut state = core.lock_state().unwrap();
            state.profiles.set_active(&first_id).unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 100,
                    rx_bytes: 200,
                    ..TunStats::default()
                },
            )
            .unwrap();
            settle_traffic_before_profile_switch(
                &mut state,
                Some(&TunStats {
                    tx_bytes: 150,
                    rx_bytes: 260,
                    ..TunStats::default()
                }),
            )
            .unwrap();
            state.profiles.delete_profile(&first_id).unwrap();
            state.profiles.set_active(&second_id).unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 180,
                    rx_bytes: 300,
                    ..TunStats::default()
                },
            )
            .unwrap();
        }

        let snapshot = core.snapshot().unwrap();
        assert!(snapshot
            .profiles
            .iter()
            .all(|profile| profile.id != first_id));
        let second = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == second_id)
            .expect("second profile");
        assert_eq!(second.upload_bytes, 40);
        assert_eq!(second.download_bytes, 30);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn platform_stop_settles_meow_traffic_when_native_stats_are_unavailable() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-platform-stop-traffic-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Direct",
                "test",
                &hmeta_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();
        core.set_platform_vpn_running(true).unwrap();
        let tunnel = {
            let state = core.lock_state().unwrap();
            state.tunnel.clone().expect("loaded tunnel")
        };
        tunnel.statistics().add_upload(320);
        tunnel.statistics().add_download(640);

        core.set_platform_vpn_running(false).unwrap();
        let snapshot = core.snapshot().unwrap();
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile summary");

        assert_eq!(profile.upload_bytes, 320);
        assert_eq!(profile.download_bytes, 640);
        assert_eq!(snapshot.traffic.upload_bytes, 320);
        assert_eq!(snapshot.traffic.download_bytes, 640);
        assert_eq!(snapshot.traffic.upload_speed, 0);
        assert_eq!(snapshot.traffic.download_speed, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_starts_meow_external_controller() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-controller-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let core = CoreHandle::new_with_profile_root_and_controller(root, addr);
        let yaml = format!(
            r#"mixed-port: 7890
external-controller: {addr}
proxies:
  - name: HTTP-MOCK
    type: http
    server: 127.0.0.1
    port: 18080
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
      - HTTP-MOCK
  - name: Auto
    type: url-test
    proxies:
      - DIRECT
    url: https://www.gstatic.com/generate_204
    interval: 3600
rules:
  - MATCH,Proxy
"#
        );
        let profile_id = core
            .import_profile_from_content("Direct", "test", &yaml, None)
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.controller_running);
        let addr_string = addr.to_string();
        assert_eq!(
            snapshot.controller_addr.as_deref(),
            Some(addr_string.as_str())
        );

        let version = wait_for_json(&format!("http://{addr}/version")).await;
        assert_eq!(
            version.get("meta").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        let proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
        assert!(proxies
            .get("proxies")
            .and_then(|value| value.get("DIRECT"))
            .is_some());
        core.select_proxy_via_controller("Proxy", "HTTP-MOCK")
            .await
            .unwrap();
        let snapshot = core.snapshot().unwrap();
        assert_eq!(
            snapshot.profiles[0]
                .selected_proxies
                .get("Proxy")
                .map(String::as_str),
            Some("HTTP-MOCK")
        );
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .unwrap();
        assert_eq!(proxy_group.selected.as_deref(), Some("HTTP-MOCK"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "HTTP-MOCK" && proxy.selected));
        core.select_proxy_via_controller("Auto", "DIRECT")
            .await
            .unwrap();
        let auto_group = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == "Auto")
            .expect("URLTest group");
        assert_eq!(auto_group.fixed.as_deref(), Some("DIRECT"));
        core.unfix_proxy_via_controller("Auto").await.unwrap();
        let auto_group = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == "Auto")
            .expect("URLTest group");
        assert_eq!(auto_group.fixed.as_deref(), Some(""));
        let rules = wait_for_json(&format!("http://{addr}/rules")).await;
        assert!(rules
            .get("rules")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rules| !rules.is_empty()));
        let health_url = spawn_healthcheck_http_server().await;
        let delay = core
            .test_proxy_delay_via_controller("DIRECT", Some(&health_url), Some(1000))
            .await
            .unwrap();
        assert!(delay > 0);
        let proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
        assert!(proxies
            .get("proxies")
            .and_then(|value| value.get("DIRECT"))
            .and_then(|value| value.get("history"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|history| !history.is_empty()));

        let group_health_url = spawn_healthcheck_http_server().await;
        let group_delays = core
            .test_proxy_group_via_controller("Auto", Some(&group_health_url), Some(1000))
            .await
            .unwrap();
        assert!(group_delays.get("DIRECT").is_some_and(|delay| *delay > 0));
        core.flush_dns_cache_via_controller().await.unwrap();
        core.flush_fake_ip_cache_via_controller().await.unwrap();

        let mut memory_in_use = 0;
        for _ in 0..30 {
            memory_in_use = core
                .snapshot()
                .unwrap()
                .controller_diagnostics
                .memory_in_use_bytes;
            if memory_in_use > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(memory_in_use > 0, "controller memory stream stayed empty");

        let tunnel = {
            let state = core.lock_state().unwrap();
            state.tunnel.clone().expect("loaded tunnel")
        };
        tunnel.statistics().add_upload(64);
        tunnel.statistics().add_download(96);
        let connection_id = track_test_connection(&tunnel, "api.example.test");
        let traffic = wait_for_traffic_frame(&format!("ws://{addr}/traffic"), 64, 96).await;
        assert_eq!(
            traffic.get("up").and_then(serde_json::Value::as_i64),
            Some(64)
        );
        assert_eq!(
            traffic.get("down").and_then(serde_json::Value::as_i64),
            Some(96)
        );
        let connections = wait_for_json(&format!("http://{addr}/connections")).await;
        assert_eq!(
            connections
                .get("connections")
                .and_then(serde_json::Value::as_array)
                .and_then(|connections| connections.first())
                .and_then(|connection| connection.get("id"))
                .and_then(serde_json::Value::as_str),
            Some(connection_id.as_str())
        );
        core.close_connection_via_controller(&connection_id)
            .await
            .unwrap();
        assert!(tunnel.statistics().active_connections().is_empty());
        let first = track_test_connection(&tunnel, "first-api.example.test");
        let second = track_test_connection(&tunnel, "second-api.example.test");
        assert!(tunnel
            .statistics()
            .active_connections()
            .iter()
            .any(|connection| connection.id.to_string() == first));
        assert!(tunnel
            .statistics()
            .active_connections()
            .iter()
            .any(|connection| connection.id.to_string() == second));
        core.close_all_connections_via_controller().await.unwrap();
        assert!(tunnel.statistics().active_connections().is_empty());
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot
            .logs
            .iter()
            .any(|log| log.message == format!("connection closed via meow API: {connection_id}")));
        assert!(snapshot
            .logs
            .iter()
            .any(|log| log.message == "all connections closed via meow API: 2"));

        let warning = format!(
            "hmeta controller ws log test {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let (mut ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/logs?level=warning"))
                .await
                .unwrap();
        tracing::warn!(target: "hmeta_core_controller_test", "{}", warning);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
        let mut matched = false;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Some(frame) = tokio::time::timeout(remaining, ws.next())
                .await
                .expect("logs websocket frame")
            else {
                break;
            };
            let frame = frame
                .expect("logs websocket receive")
                .into_text()
                .expect("text frame");
            let log: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if log
                .get("payload")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|payload| payload.contains(&warning))
            {
                assert_eq!(
                    log.get("type").and_then(serde_json::Value::as_str),
                    Some("warning")
                );
                matched = true;
                break;
            }
        }
        assert!(matched, "logs websocket did not receive warning payload");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_controller_config_reload_converges_profile_and_native_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-controller-sync-test-{}",
            now_unix_nanos()
        ));
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let core = CoreHandle::new_with_profile_root_and_controller(root, addr);
        let original = format!(
            r#"mixed-port: 7890
hmeta:
  vpn:
    mtu: 1410
proxies:
  - name: HTTP-OLD
    type: http
    server: 127.0.0.1
    port: 18080
proxy-groups:
  - name: OldProxy
    type: select
    proxies: [DIRECT, HTTP-OLD]
rules:
  - MATCH,OldProxy
"#
        );
        let profile_id = core
            .import_profile_from_content("Controller sync", "test", &original, None)
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();
        let _ = wait_for_json(&format!("http://{addr}/version")).await;
        let mut fds = [0_i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let options_json = to_json(&VpnOptions::default()).unwrap();
        core.start_vpn(fds[0], &options_json).await.unwrap();
        assert_eq!(core.vpn.fd(), Some(fds[0]));

        let replacement = r#"mode: direct
proxies:
  - name: HTTP-NEW
    type: http
    server: 127.0.0.1
    port: 18081
proxy-groups:
  - name: NewProxy
    type: select
    proxies: [DIRECT, HTTP-NEW]
rules:
  - MATCH,NewProxy
"#;
        let payload = base64::engine::general_purpose::STANDARD.encode(replacement);
        let response = reqwest::Client::new()
            .put(format!("http://{addr}/configs"))
            .json(&serde_json::json!({ "payload": payload }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

        assert!(core.sync_external_controller_config().await.unwrap());
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.vpn_running);
        assert_eq!(core.vpn.fd(), Some(fds[0]));
        assert_eq!(snapshot.mode, RuntimeMode::Direct);
        assert!(snapshot
            .proxy_groups
            .iter()
            .any(|group| group.name == "NewProxy"));
        assert!(!snapshot
            .proxy_groups
            .iter()
            .any(|group| group.name == "OldProxy"));
        assert!(snapshot
            .rules
            .iter()
            .any(|rule| rule.line == "MATCH,NewProxy"));
        assert_eq!(snapshot.controller_diagnostics.config_sync_count, 1);
        assert!(snapshot
            .controller_diagnostics
            .last_config_sync_at
            .is_some());
        assert!(snapshot
            .controller_diagnostics
            .last_config_sync_error
            .is_none());
        let controller_proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
        assert!(controller_proxies
            .get("proxies")
            .and_then(|proxies| proxies.get("NewProxy"))
            .is_some());

        let persisted = core.profile_raw_yaml(&profile_id).unwrap();
        assert!(persisted.contains("HTTP-NEW"));
        assert!(!persisted.contains("HTTP-OLD"));
        assert!(persisted.contains("hmeta:"));
        assert!(persisted.contains("mtu: 1410"));
        assert!(!persisted.contains("external-controller:"));
        core.stop_vpn().unwrap();
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn controller_exposes_loaded_provider_registries() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-provider-controller-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let import_provider_path = root.join("import-provider.yaml");
        std::fs::write(&import_provider_path, provider_proxy_yaml()).unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let core = CoreHandle::new_with_profile_root_and_controller(root.clone(), addr);
        let profile_id = core
            .import_profile_from_content(
                "Provider",
                "test",
                &provider_profile_yaml(&import_provider_path),
                None,
            )
            .await
            .unwrap();
        let runtime_provider_dir = root.join("providers/proxy").join(&profile_id);
        std::fs::create_dir_all(&runtime_provider_dir).unwrap();
        std::fs::write(
            runtime_provider_dir.join("LocalProxyProvider.yaml"),
            provider_proxy_yaml(),
        )
        .unwrap();

        core.reload_config(&profile_id).await.unwrap();

        let proxy_providers = wait_for_json(&format!("http://{addr}/providers/proxies")).await;
        assert_eq!(
            proxy_providers
                .get("providers")
                .and_then(|providers| providers.get("LocalProxyProvider"))
                .and_then(|provider| provider.get("proxies"))
                .and_then(serde_json::Value::as_array)
                .and_then(|proxies| proxies.first())
                .and_then(|proxy| proxy.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("PROVIDER-HTTP")
        );

        let rule_providers = wait_for_json(&format!("http://{addr}/providers/rules")).await;
        assert_eq!(
            rule_providers
                .get("providers")
                .and_then(|providers| providers.get("LocalRuleProvider"))
                .and_then(|provider| provider.get("ruleCount"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        core.refresh_provider("LocalProxyProvider").await.unwrap();
        {
            let state = core.lock_state().unwrap();
            assert!(state.logs.iter().any(|log| {
                log.level == "info"
                    && log
                        .message
                        .contains("proxy provider refreshed via meow API: LocalProxyProvider")
            }));
        }
        let snapshot = core.snapshot().unwrap();
        let proxy_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "LocalProxyProvider")
            .expect("LocalProxyProvider summary");
        assert_eq!(
            proxy_provider.path.as_deref(),
            Some(
                runtime_provider_dir
                    .join("LocalProxyProvider.yaml")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(proxy_provider.cache_exists);
        assert!(proxy_provider.cache_bytes.is_some_and(|bytes| bytes > 0));
        assert!(proxy_provider.cache_updated_at.is_some());
        assert!(proxy_provider.last_refresh_at.is_some());
        assert!(proxy_provider.last_refresh_error.is_none());
        assert_eq!(proxy_provider.members.len(), 1);
        assert_eq!(proxy_provider.members[0].name, "PROVIDER-HTTP");

        core.healthcheck_proxy_provider_via_controller("LocalProxyProvider")
            .await
            .unwrap();
        let health_url = spawn_healthcheck_http_server().await;
        let error = core
            .healthcheck_provider_proxy_via_controller(
                "LocalProxyProvider",
                "PROVIDER-HTTP",
                &health_url,
                Some(1000),
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 503"));
        let snapshot = core.snapshot().unwrap();
        let proxy_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "LocalProxyProvider")
            .expect("LocalProxyProvider summary after health check");
        assert!(!proxy_provider.members[0].alive);
        assert_eq!(proxy_provider.members[0].delay_ms, Some(0));

        core.refresh_all_providers().await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let proxy_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "LocalProxyProvider")
            .expect("LocalProxyProvider summary after refresh all");
        let rule_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "LocalRuleProvider")
            .expect("LocalRuleProvider summary after refresh all");
        assert!(proxy_provider.last_refresh_at.is_some());
        assert!(proxy_provider.last_refresh_error.is_none());
        assert!(rule_provider.last_refresh_at.is_none());
        assert!(rule_provider.last_refresh_error.is_none());
        let state = core.lock_state().unwrap();
        assert!(state.logs.iter().any(|log| {
            log.level == "info"
                && log
                    .message
                    .contains("provider refresh all finished: 1 succeeded, 0 failed")
        }));
        drop(state);

        {
            let mut state = core.lock_state().unwrap();
            state.providers.push(ProviderSummary {
                name: "BrokenProvider".to_owned(),
                provider_type: "broken".to_owned(),
                path: None,
                url: None,
                vehicle_type: None,
                interval_seconds: None,
                filter: None,
                exclude_filter: None,
                behavior: None,
                format: None,
                health_check_enabled: false,
                health_check_url: None,
                health_check_interval_seconds: None,
                expected_status: None,
                members: Vec::new(),
                cache_exists: false,
                cache_bytes: None,
                cache_updated_at: None,
                stale_cache_available: false,
                last_refresh_at: None,
                last_refresh_error: None,
            });
        }
        let err = core.refresh_provider("BrokenProvider").await.unwrap_err();
        assert!(err.to_string().contains("unknown provider type"));
        let snapshot = core.snapshot().unwrap();
        let broken_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "BrokenProvider")
            .expect("BrokenProvider summary");
        assert!(broken_provider.last_refresh_at.is_some());
        assert!(broken_provider
            .last_refresh_error
            .as_deref()
            .unwrap_or_default()
            .contains("unknown provider type"));

        let err = core.refresh_provider("MissingProvider").await.unwrap_err();
        assert!(err.to_string().contains("provider not found"));
        let state = core.lock_state().unwrap();
        assert!(state.logs.iter().any(|log| {
            log.level == "warning"
                && log
                    .message
                    .contains("provider refresh failed: provider not found: MissingProvider")
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_refresh_disambiguates_same_name_by_type() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-provider-duplicate-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let import_provider_path = root.join("import-provider.yaml");
        std::fs::write(&import_provider_path, provider_proxy_yaml()).unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let core = CoreHandle::new_with_profile_root_and_controller(root.clone(), addr);
        let profile_id = core
            .import_profile_from_content(
                "Duplicate Providers",
                "test",
                &duplicate_provider_profile_yaml(&import_provider_path),
                None,
            )
            .await
            .unwrap();
        let runtime_provider_dir = root.join("providers/proxy").join(&profile_id);
        std::fs::create_dir_all(&runtime_provider_dir).unwrap();
        std::fs::write(
            runtime_provider_dir.join("Shared.yaml"),
            provider_proxy_yaml(),
        )
        .unwrap();
        core.reload_config(&profile_id).await.unwrap();
        let _ = wait_for_json(&format!("http://{addr}/providers/rules")).await;

        let err = core
            .refresh_provider_of_type("rule", "Shared")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is inline"));
        let snapshot = core.snapshot().unwrap();
        let rule_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "Shared" && provider.provider_type == "rule")
            .expect("rule provider");
        let proxy_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "Shared" && provider.provider_type == "proxy")
            .expect("proxy provider");
        assert!(rule_provider.last_refresh_at.is_some());
        assert!(rule_provider
            .last_refresh_error
            .as_deref()
            .is_some_and(|error| error.contains("is inline")));
        assert!(proxy_provider.last_refresh_at.is_none());
        let _ = wait_for_json(&format!("http://{addr}/providers/rules")).await;

        core.refresh_all_providers().await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let rule_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "Shared" && provider.provider_type == "rule")
            .expect("rule provider after refresh all");
        let proxy_provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "Shared" && provider.provider_type == "proxy")
            .expect("proxy provider after refresh all");
        assert!(rule_provider
            .last_refresh_error
            .as_deref()
            .is_some_and(|error| error.contains("is inline")));
        assert!(proxy_provider.last_refresh_at.is_some());
        assert!(proxy_provider.last_refresh_error.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inline_rule_provider_runtime_cache_fields_do_not_break_reload() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-inline-rule-provider-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile_id = core
            .import_profile_from_content(
                "Inline Rule Provider",
                "test",
                r#"
mixed-port: 7890
mode: rule
rule-providers:
  InlineRules:
    type: inline
    behavior: classical
    interval: 3600
    path: ../../inline.yaml
    payload:
      - DOMAIN-SUFFIX,inline.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - RULE-SET,InlineRules,DIRECT
  - MATCH,DIRECT
"#,
                None,
            )
            .await
            .unwrap();

        core.reload_config(&profile_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.engine_loaded);
        let provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "InlineRules" && provider.provider_type == "rule")
            .expect("inline rule provider");
        assert!(provider.path.is_none());
        assert!(provider.interval_seconds.is_none());
        assert_eq!(provider.behavior.as_deref(), Some("classical"));
    }

    #[test]
    fn provider_refresh_failure_marks_stale_cache_available() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-provider-stale-cache-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let cache_path = root.join("providers/proxy/default/StaleProvider.yaml");
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, provider_proxy_yaml()).unwrap();

        let core = CoreHandle::new_with_profile_root(root.join("store"));
        let mut state = core.lock_state().unwrap();
        state.providers.push(ProviderSummary {
            name: "StaleProvider".to_owned(),
            provider_type: "proxy".to_owned(),
            path: Some(cache_path.to_string_lossy().into_owned()),
            url: Some("http://127.0.0.1:9/provider.yaml".to_owned()),
            vehicle_type: Some("http".to_owned()),
            interval_seconds: None,
            filter: None,
            exclude_filter: None,
            behavior: None,
            format: None,
            health_check_enabled: false,
            health_check_url: None,
            health_check_interval_seconds: None,
            expected_status: None,
            members: Vec::new(),
            cache_exists: false,
            cache_bytes: None,
            cache_updated_at: None,
            stale_cache_available: false,
            last_refresh_at: None,
            last_refresh_error: None,
        });

        mark_provider_refresh(
            &mut state,
            "proxy",
            "StaleProvider",
            "12345".to_owned(),
            Some("refresh failed".to_owned()),
        );

        let provider = state.providers.first().expect("provider summary");
        assert!(provider.cache_exists);
        assert!(provider.cache_bytes.is_some_and(|bytes| bytes > 0));
        assert!(provider.cache_updated_at.is_some());
        assert!(provider.stale_cache_available);
        assert_eq!(provider.last_refresh_at.as_deref(), Some("12345"));
        assert_eq!(
            provider.last_refresh_error.as_deref(),
            Some("refresh failed")
        );
        assert_eq!(
            provider_refresh_failure_log_message("refresh failed", provider.stale_cache_available),
            "refresh failed; stale provider cache retained"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_all_providers_reports_empty_provider_set() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-provider-empty-refresh-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        core.refresh_all_providers().await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.logs.iter().any(|log| {
            log.level == "info"
                && log.message == "provider refresh skipped: no refreshable providers"
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_all_providers_skips_inline_only_provider_set() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-provider-inline-refresh-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        {
            let mut state = core.lock_state().unwrap();
            state.providers.push(ProviderSummary {
                name: "InlineRules".to_owned(),
                provider_type: "rule".to_owned(),
                path: None,
                url: None,
                vehicle_type: Some("inline".to_owned()),
                interval_seconds: None,
                filter: None,
                exclude_filter: None,
                behavior: Some("classical".to_owned()),
                format: None,
                health_check_enabled: false,
                health_check_url: None,
                health_check_interval_seconds: None,
                expected_status: None,
                members: Vec::new(),
                cache_exists: false,
                cache_bytes: None,
                cache_updated_at: None,
                stale_cache_available: false,
                last_refresh_at: None,
                last_refresh_error: None,
            });
        }

        core.refresh_all_providers().await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.name == "InlineRules")
            .expect("inline provider");
        assert!(provider.last_refresh_at.is_none());
        assert!(provider.last_refresh_error.is_none());
        assert!(snapshot.logs.iter().any(|log| {
            log.level == "info"
                && log.message == "provider refresh skipped: no refreshable providers"
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn selected_proxy_is_restored_after_reload() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-selected-proxy-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let id = core
            .import_profile_from_content(
                "http",
                "local-file",
                &local_protocol_profile("http"),
                None,
            )
            .await
            .unwrap();

        core.reload_config(&id).await.unwrap();
        let order_before_selection = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == "Proxy")
            .unwrap()
            .proxies
            .into_iter()
            .map(|proxy| proxy.name)
            .collect::<Vec<_>>();
        core.select_proxy("Proxy", "DIRECT").await.unwrap();
        core.reload_config(&id).await.unwrap();

        let snapshot = core.snapshot().unwrap();
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert_eq!(proxy_group.selected.as_deref(), Some("DIRECT"));
        assert_eq!(
            proxy_group
                .proxies
                .iter()
                .map(|proxy| proxy.name.as_str())
                .collect::<Vec<_>>(),
            order_before_selection
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            snapshot.profiles[0]
                .selected_proxies
                .get("Proxy")
                .map(String::as_str),
            Some("DIRECT")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automatic_group_pins_and_auto_mode_persist_across_reload() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-automatic-group-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let yaml = r#"
proxy-groups:
  - name: Auto
    type: url-test
    proxies: [DIRECT]
    url: https://www.gstatic.com/generate_204
    interval: 3600
  - name: Backup
    type: fallback
    proxies: [DIRECT]
    url: https://www.gstatic.com/generate_204
    interval: 3600
rules:
  - MATCH,Auto
"#;
        let profile_id = core
            .import_profile_from_content("Automatic", "test", yaml, None)
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        for group_name in ["Auto", "Backup"] {
            core.select_proxy(group_name, "DIRECT").await.unwrap();
            let group = core
                .snapshot()
                .unwrap()
                .proxy_groups
                .into_iter()
                .find(|group| group.name == group_name)
                .expect("automatic group");
            assert_eq!(group.fixed.as_deref(), Some("DIRECT"));

            core.unfix_proxy(group_name).unwrap();
            let snapshot = core.snapshot().unwrap();
            let group = snapshot
                .proxy_groups
                .iter()
                .find(|group| group.name == group_name)
                .expect("automatic group");
            assert_eq!(group.fixed.as_deref(), Some(""));
            assert_eq!(
                snapshot.profiles[0]
                    .selected_proxies
                    .get(group_name)
                    .map(String::as_str),
                Some("")
            );
        }

        core.reload_config(&profile_id).await.unwrap();
        for group_name in ["Auto", "Backup"] {
            let group = core
                .snapshot()
                .unwrap()
                .proxy_groups
                .into_iter()
                .find(|group| group.name == group_name)
                .expect("restored automatic group");
            assert_eq!(group.fixed.as_deref(), Some(""));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_retains_sniffer_config_for_harmony_tun() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-sniffer-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let yaml = r#"
sniffer:
  enable: true
  timeout: 250
  parse-pure-ip: true
  override-destination: true
  sniff:
    TLS:
      ports: [443, 8443]
    HTTP:
      ports: [80, 8080]
proxy-groups:
  - name: Proxy
    type: select
    proxies: [DIRECT]
rules:
  - MATCH,Proxy
"#;
        let profile_id = core
            .import_profile_from_content("Sniffer", "test", yaml, None)
            .await
            .unwrap();
        core.reload_config(&profile_id).await.unwrap();

        let config = core.lock_state().unwrap().sniffer_config.clone();
        assert!(config.enable);
        assert_eq!(config.timeout, std::time::Duration::from_millis(250));
        assert!(config.override_destination);
        assert_eq!(config.tls_ports, vec![443, 8443]);
        assert_eq!(config.http_ports, vec![80, 8080]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_edit_and_backup_restore_reload_active_tunnel() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-profile-edit-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let id = core
            .import_profile_from_content(
                "http",
                "local-file",
                &local_protocol_profile("http"),
                None,
            )
            .await
            .unwrap();

        core.reload_config(&id).await.unwrap();
        let original_yaml = core.profile_raw_yaml(&id).unwrap();
        let invalid = core.update_profile_content(&id, "proxy-groups: [").await;
        assert!(invalid.is_err());
        assert_eq!(core.profile_raw_yaml(&id).unwrap(), original_yaml);
        assert!(core
            .validate_profile_content(&local_protocol_profile("direct"))
            .await
            .is_ok());
        assert!(core
            .validate_profile_content("proxy-groups: [")
            .await
            .is_err());

        core.update_profile_content(&id, &local_protocol_profile("direct"))
            .await
            .unwrap();
        let snapshot = core.snapshot().unwrap();
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "DIRECT"));

        core.restore_profile_backup(&id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "HTTP-MOCK"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deleting_active_profile_reloads_next_or_clears_engine() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-profile-delete-active-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let direct_id = core
            .import_profile_from_content(
                "direct",
                "local-file",
                &local_protocol_profile("direct"),
                None,
            )
            .await
            .unwrap();
        let http_id = core
            .import_profile_from_content(
                "http",
                "local-file",
                &local_protocol_profile("http"),
                None,
            )
            .await
            .unwrap();

        core.reload_config(&direct_id).await.unwrap();
        assert_eq!(
            core.snapshot().unwrap().active_profile.as_deref(),
            Some(direct_id.as_str())
        );

        core.delete_profile(&direct_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.active_profile.as_deref(), Some(http_id.as_str()));
        assert!(snapshot.engine_loaded);
        assert!(snapshot
            .proxy_groups
            .iter()
            .any(|group| group.proxies.iter().any(|proxy| proxy.name == "HTTP-MOCK")));

        core.delete_profile(&http_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        assert!(snapshot.active_profile.is_none());
        assert!(!snapshot.engine_loaded);
        assert!(snapshot.proxy_groups.is_empty());
        assert!(snapshot.providers.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_all_profiles_continues_after_single_failure() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-refresh-all-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let (good_url, bad_url) =
            spawn_profile_refresh_http_server(local_protocol_profile("direct")).await;
        let good_id = core
            .import_profile_from_content(
                "good",
                &good_url,
                &local_protocol_profile("http"),
                Some(good_url.clone()),
            )
            .await
            .unwrap();
        let bad_id = core
            .import_profile_from_content(
                "bad",
                &bad_url,
                &local_protocol_profile("http"),
                Some(bad_url.clone()),
            )
            .await
            .unwrap();

        core.reload_config(&good_id).await.unwrap();
        core.refresh_all_profiles().await.unwrap();

        let good_yaml = core.profile_raw_yaml(&good_id).unwrap();
        let bad_yaml = core.profile_raw_yaml(&bad_id).unwrap();
        assert!(!good_yaml.contains("HTTP-MOCK"));
        assert!(good_yaml.contains("MATCH,DIRECT"));
        assert!(bad_yaml.contains("HTTP-MOCK"));

        let snapshot = core.snapshot().unwrap();
        let good_profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == good_id)
            .expect("good profile summary");
        assert!(good_profile.last_refresh_at.is_some());
        assert!(good_profile.last_refresh_error.is_none());
        let bad_profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == bad_id)
            .expect("bad profile summary");
        assert!(bad_profile.last_refresh_at.is_some());
        assert!(bad_profile
            .last_refresh_error
            .as_deref()
            .unwrap_or_default()
            .contains("profile refresh failed"));
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "DIRECT"));
        assert!(snapshot.logs.iter().any(|log| {
            log.level == "warning" && log.message.contains("profile refresh failed: bad")
        }));
        assert!(snapshot.logs.iter().any(|log| {
            log.level == "info"
                && log
                    .message
                    .contains("profile refresh all finished: 1 succeeded, 1 failed")
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscription_userinfo_header_updates_profile_summary() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-sub-userinfo-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let url = spawn_subscription_userinfo_http_server(local_protocol_profile("direct")).await;

        let profile_id = core.import_profile_from_url(&url, None).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile summary");
        assert_eq!(profile.name, "Remote Sub");
        let info = profile
            .subscription_user_info
            .as_ref()
            .expect("subscription userinfo");
        assert_eq!(info.upload_bytes, 100);
        assert_eq!(info.download_bytes, 200);
        assert_eq!(info.total_bytes, Some(1000));
        assert_eq!(info.expire_at.as_deref(), Some("1893456000"));
        let metadata = profile
            .subscription_metadata
            .as_ref()
            .expect("subscription metadata");
        assert_eq!(metadata.title.as_deref(), Some("Remote Sub"));
        assert_eq!(metadata.update_interval_hours, Some(12));
        assert_eq!(
            metadata.web_page_url.as_deref(),
            Some("https://example.test/portal")
        );
        assert_eq!(
            metadata.support_url.as_deref(),
            Some("https://example.test/support")
        );

        core.refresh_profile(&profile_id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile summary after refresh");
        let info = profile
            .subscription_user_info
            .as_ref()
            .expect("subscription userinfo after refresh");
        assert_eq!(info.upload_bytes, 300);
        assert_eq!(info.download_bytes, 400);
        assert_eq!(info.total_bytes, Some(2000));
        assert_eq!(info.expire_at.as_deref(), Some("1896048000"));
        let metadata = profile
            .subscription_metadata
            .as_ref()
            .expect("subscription metadata after refresh");
        assert_eq!(metadata.title.as_deref(), Some("Remote Sub Updated"));
        assert_eq!(metadata.update_interval_hours, Some(24));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscription_metadata_comment_fills_missing_header_fields() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-sub-comment-metadata-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let body = format!(
            "{}\n{}",
            "# profile-title=Body%20Title; profile-update-interval=6; profile-web-page-url=https://example.test/body; support-url=https://example.test/help",
            local_protocol_profile("direct")
        );
        let url = spawn_subscription_metadata_comment_http_server(body).await;

        let profile_id = core.import_profile_from_url(&url, None).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile summary");
        assert_eq!(profile.name, "Header Title");
        let metadata = profile
            .subscription_metadata
            .as_ref()
            .expect("subscription metadata");
        assert_eq!(metadata.title.as_deref(), Some("Header Title"));
        assert_eq!(metadata.update_interval_hours, Some(6));
        assert_eq!(
            metadata.web_page_url.as_deref(),
            Some("https://example.test/body")
        );
        assert_eq!(
            metadata.support_url.as_deref(),
            Some("https://example.test/help")
        );
    }

    #[test]
    fn content_disposition_title_is_used_as_subscription_metadata_title() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "content-disposition",
            reqwest::header::HeaderValue::from_static(
                "attachment; filename*=UTF-8''%E8%BF%9C%E7%A8%8B.yaml",
            ),
        );
        headers.insert(
            "profile-update-interval",
            reqwest::header::HeaderValue::from_static("24"),
        );

        let metadata = subscription_metadata_from_headers(&headers).expect("metadata");
        assert_eq!(metadata.title.as_deref(), Some("远程.yaml"));
        assert_eq!(metadata.update_interval_hours, Some(24));
        assert_eq!(
            subscription_profile_name_from_headers(&headers).as_deref(),
            Some("远程.yaml")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feature_gated_proxy_types_are_loaded() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-protocol-feature-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let profile = r#"
mixed-port: 7890
mode: rule
log-level: info
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxies:
  - name: TROJAN-MOCK
    type: trojan
    server: 127.0.0.1
    port: 443
    password: test-trojan-password
    sni: localhost
    skip-cert-verify: true
    udp: false
  - name: VLESS-MOCK
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    tls: false
    udp: false
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - TROJAN-MOCK
      - VLESS-MOCK
      - DIRECT
rules:
  - MATCH,Proxy
"#;
        let id = core
            .import_profile_from_content("feature-gated", "test", profile, None)
            .await
            .unwrap();

        core.reload_config(&id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "TROJAN-MOCK"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-MOCK"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn share_link_subscription_imports_before_meow_validation() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-share-subscription-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let links = "\
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&security=none#VLESS-MOCK
trojan://test-trojan-password@127.0.0.1:443?sni=localhost&allowInsecure=1#TROJAN-MOCK
";
        let encoded = base64::engine::general_purpose::STANDARD.encode(links);
        let id = core
            .import_profile_from_content(
                "share-subscription",
                "https://example.test/sub",
                &encoded,
                Some("https://example.test/sub".to_owned()),
            )
            .await
            .unwrap();

        core.reload_config(&id).await.unwrap();
        let snapshot = core.snapshot().unwrap();
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-MOCK"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "TROJAN-MOCK"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn share_link_transport_options_reload_with_meow_config() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-share-transport-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let links = "\
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=ws&security=tls&sni=localhost&host=localhost&path=%2Fws&client-fingerprint=chrome&alpn=h2%2Chttp%2F1.1&ed=2048&eh=Sec-WebSocket-Protocol&tfo=1#VLESS-WS
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?network=ws&security=tls&serverName=localhost&wsHost=localhost&wsPath=%2Falias-ws&fingerprint=chrome&allow-insecure=allow#VLESS-WS-ALIAS
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=h2&security=tls&sni=localhost&host=localhost,alt.localhost&path=%2Fh2#VLESS-H2
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=httpupgrade&security=tls&sni=localhost&host=localhost&path=%2Fupgrade#VLESS-HTTPUPGRADE
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&security=tls&sni=localhost&flow=xtls-rprx-vision&allowInsecure=1#VLESS-VISION
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&tls=true&sni=localhost#VLESS-TLS-QUERY
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&encryption=none#VLESS-ENCRYPTION-NONE
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&udp=false#VLESS-UDP-OFF
trojan://test-trojan-password@127.0.0.1:443?type=grpc&serviceName=svc&sni=localhost&allowInsecure=1&fast-open=true#TROJAN-GRPC
trojan://test-trojan-password@127.0.0.1:443?type=grpc&grpc-service-name=alias-svc&grpc-mode=gun&serverName=localhost&allow-insecure=allow#TROJAN-GRPC-ALIAS
http://user:pass@127.0.0.1:8080?headers=User-Agent%3DHMeta%3BProxy-Authorization%3DBearer%20token#HTTP-SHARE
socks5://sock:sockpass@127.0.0.1:1080?tls=true&skip-cert-verify=true&udp=true&fastOpen=true#SOCKS5-SHARE
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8388?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dlocalhost&TFO=true#SS-OBFS
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8389?plugin=v2ray-plugin%3Bmode%3Dwebsocket%3Bhost%3Dlocalhost%3Bpath%3D%2Fss-ws%3Btls#SS-V2RAY
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8390?plugin=Simple-Obfs%3Bobfs%3Dhttp%3Bobfs-host%3Dlocalhost#SS-OBFS-CASE
";
        let id = core
            .import_profile_from_content("share-transport", "clipboard", links, None)
            .await
            .unwrap();

        core.reload_config(&id).await.unwrap();
        let proxy_group = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == "Proxy")
            .expect("Proxy group");
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-WS"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-WS-ALIAS"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-H2"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-HTTPUPGRADE"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-VISION"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-TLS-QUERY"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-ENCRYPTION-NONE"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "VLESS-UDP-OFF"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "TROJAN-GRPC"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "TROJAN-GRPC-ALIAS"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "HTTP-SHARE"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "SOCKS5-SHARE"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "SS-OBFS"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "SS-V2RAY"));
        assert!(proxy_group
            .proxies
            .iter()
            .any(|proxy| proxy.name == "SS-OBFS-CASE"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generated_local_protocol_profiles_import_and_populate_proxy_groups() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-generated-profiles-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let cases = [
            ("direct", "DIRECT"),
            ("http", "HTTP-MOCK"),
            ("http-auth", "HTTP-AUTH-MOCK"),
            ("http-bad-auth", "HTTP-BAD-AUTH-MOCK"),
            ("http-down", "HTTP-DOWN-MOCK"),
            ("socks5", "SOCKS5-MOCK"),
            ("socks5-auth", "SOCKS5-AUTH-MOCK"),
            ("socks5-bad-auth", "SOCKS5-BAD-AUTH-MOCK"),
            ("ss", "SS-MOCK"),
            ("ss-bad-password", "SS-BAD-PASSWORD-MOCK"),
            ("trojan", "TROJAN-MOCK"),
            ("trojan-bad-password", "TROJAN-BAD-PASSWORD-MOCK"),
            ("vless", "VLESS-MOCK"),
            ("vless-bad-uuid", "VLESS-BAD-UUID-MOCK"),
        ];

        for (mode, expected_proxy) in cases {
            let profile = local_protocol_profile(mode);
            let id = core
                .import_profile_from_content(mode, "local-file", &profile, None)
                .await
                .unwrap_or_else(|err| panic!("{mode} profile should import: {err}"));

            core.reload_config(&id)
                .await
                .unwrap_or_else(|err| panic!("{mode} profile should reload: {err}"));
            let snapshot = core.snapshot().unwrap();
            assert_eq!(snapshot.active_profile.as_deref(), Some(id.as_str()));
            assert!(snapshot.profiles.iter().any(|profile| profile.id == id));
            let proxy_group = snapshot
                .proxy_groups
                .iter()
                .find(|group| group.name == "Proxy")
                .unwrap_or_else(|| panic!("{mode} profile should expose Proxy group"));
            assert!(
                proxy_group
                    .proxies
                    .iter()
                    .any(|proxy| proxy.name == expected_proxy),
                "{mode} profile should expose {expected_proxy}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shadowsocks_proxy_echo_roundtrip_and_bad_password_fails() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-ss-echo-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let echo_addr = spawn_tcp_echo_server().await;
        let ss_addr = spawn_shadowsocks_proxy().await;
        let good_profile =
            local_protocol_profile_with_ports("ss", "127.0.0.1", echo_addr.port(), ss_addr.port());
        let good_id = core
            .import_profile_from_content("ss", "local-file", &good_profile, None)
            .await
            .unwrap();
        core.reload_config(&good_id).await.unwrap();

        let delay = core
            .test_proxy_delay("SS-MOCK", Some(&format!("http://{echo_addr}")), Some(1000))
            .await
            .unwrap();
        assert!(delay < 1000);
        let echoed = core
            .test_proxy_echo(
                "SS-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-ss-echo",
                Some(1000),
            )
            .await
            .unwrap();
        assert_eq!(echoed, "hmeta-ss-echo");

        let bad_profile = local_protocol_profile_with_ports(
            "ss-bad-password",
            "127.0.0.1",
            echo_addr.port(),
            ss_addr.port(),
        );
        let bad_id = core
            .import_profile_from_content("ss-bad-password", "local-file", &bad_profile, None)
            .await
            .unwrap();
        core.reload_config(&bad_id).await.unwrap();

        let err = core
            .test_proxy_echo(
                "SS-BAD-PASSWORD-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-ss-echo",
                Some(300),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("echo test"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_and_socks5_auth_echo_roundtrip_and_bad_credentials_fail() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-core-http-socks-echo-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let core = CoreHandle::new_with_profile_root(root);
        let echo_addr = spawn_tcp_echo_server().await;

        let http_addr =
            spawn_http_connect_proxy(Some("Proxy-Authorization: Basic YWxpY2U6czNjcjN0")).await;
        let http_profile = local_protocol_profile_with_ports(
            "http-auth",
            "127.0.0.1",
            echo_addr.port(),
            http_addr.port(),
        );
        let http_id = core
            .import_profile_from_content("http-auth", "local-file", &http_profile, None)
            .await
            .unwrap();
        core.reload_config(&http_id).await.unwrap();
        assert!(
            core.test_proxy_delay(
                "HTTP-AUTH-MOCK",
                Some(&format!("http://{echo_addr}")),
                Some(1000)
            )
            .await
            .unwrap()
                < 1000
        );
        assert_eq!(
            core.test_proxy_echo(
                "HTTP-AUTH-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-http-echo",
                Some(1000),
            )
            .await
            .unwrap(),
            "hmeta-http-echo"
        );

        let bad_http_profile = local_protocol_profile_with_ports(
            "http-bad-auth",
            "127.0.0.1",
            echo_addr.port(),
            http_addr.port(),
        );
        let bad_http_id = core
            .import_profile_from_content("http-bad-auth", "local-file", &bad_http_profile, None)
            .await
            .unwrap();
        core.reload_config(&bad_http_id).await.unwrap();
        let err = core
            .test_proxy_echo(
                "HTTP-BAD-AUTH-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-http-echo",
                Some(300),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("echo test"));

        let socks_addr = spawn_socks5_proxy(Some((b"bob", b"hunter2"))).await;
        let socks_profile = local_protocol_profile_with_ports(
            "socks5-auth",
            "127.0.0.1",
            echo_addr.port(),
            socks_addr.port(),
        );
        let socks_id = core
            .import_profile_from_content("socks5-auth", "local-file", &socks_profile, None)
            .await
            .unwrap();
        core.reload_config(&socks_id).await.unwrap();
        assert!(
            core.test_proxy_delay(
                "SOCKS5-AUTH-MOCK",
                Some(&format!("http://{echo_addr}")),
                Some(1000)
            )
            .await
            .unwrap()
                < 1000
        );
        assert_eq!(
            core.test_proxy_echo(
                "SOCKS5-AUTH-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-socks-echo",
                Some(1000),
            )
            .await
            .unwrap(),
            "hmeta-socks-echo"
        );

        let bad_socks_profile = local_protocol_profile_with_ports(
            "socks5-bad-auth",
            "127.0.0.1",
            echo_addr.port(),
            socks_addr.port(),
        );
        let bad_socks_id = core
            .import_profile_from_content("socks5-bad-auth", "local-file", &bad_socks_profile, None)
            .await
            .unwrap();
        core.reload_config(&bad_socks_id).await.unwrap();
        let err = core
            .test_proxy_echo(
                "SOCKS5-BAD-AUTH-MOCK",
                &format!("http://{echo_addr}"),
                "hmeta-socks-echo",
                Some(300),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("echo test"));
    }

    fn local_protocol_profile(mode: &str) -> String {
        local_protocol_profile_with_ports(mode, "127.0.0.1", 58197, 58198)
    }

    fn local_protocol_profile_with_ports(
        mode: &str,
        host: &str,
        echo_port: u16,
        proxy_port: u16,
    ) -> String {
        let template_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../local-protocol-tests/profiles")
            .join(format!("{mode}.yaml.in"));
        std::fs::read_to_string(template_path)
            .unwrap()
            .replace("{{HOST}}", host)
            .replace("{{ECHO_PORT}}", &echo_port.to_string())
            .replace("{{PROXY_PORT}}", &proxy_port.to_string())
    }

    async fn spawn_tcp_echo_server() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 16 * 1024];
                    loop {
                        let Ok(n) = stream.read(&mut buffer).await else {
                            break;
                        };
                        if n == 0 {
                            break;
                        }
                        if stream.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        addr
    }

    async fn spawn_shadowsocks_proxy() -> SocketAddr {
        use shadowsocks::config::{ServerConfig, ServerType};
        use shadowsocks::context::Context;
        use shadowsocks::crypto::CipherKind;
        use shadowsocks::relay::socks5::Address;
        use shadowsocks::ProxyListener;

        let config = ServerConfig::new(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            "test-shadowsocks-password",
            CipherKind::AES_128_GCM,
        )
        .unwrap();
        let listener = ProxyListener::bind(Context::new_shared(ServerType::Server), &config)
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _peer)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let target = match inbound.handshake().await {
                        Ok(Address::SocketAddress(addr)) => addr,
                        Ok(Address::DomainNameAddress(host, port)) => {
                            if host == "localhost" || host == "127.0.0.1" {
                                SocketAddr::from(([127, 0, 0, 1], port))
                            } else {
                                return;
                            }
                        }
                        Err(_) => return,
                    };
                    let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        addr
    }

    async fn spawn_http_connect_proxy(required_auth_header: Option<&'static str>) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let Ok(request) = read_http_proxy_request(&mut inbound).await else {
                        return;
                    };
                    let first_line = request.lines().next().unwrap_or_default();
                    let mut parts = first_line.split_whitespace();
                    if !parts.next().is_some_and(|method| method == "CONNECT") {
                        let _ = inbound
                            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                            .await;
                        return;
                    }
                    let Some(authority) = parts.next() else {
                        return;
                    };
                    if let Some(required) = required_auth_header {
                        if !request.contains(required)
                            || !request.contains("X-HMeta-Test: local-protocol")
                        {
                            let _ = inbound
                                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                                .await;
                            return;
                        }
                    }
                    let Some(target) = parse_local_authority(authority) else {
                        return;
                    };
                    let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = inbound
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        addr
    }

    async fn read_http_proxy_request(
        stream: &mut tokio::net::TcpStream,
    ) -> Result<String, std::io::Error> {
        let mut bytes = Vec::with_capacity(1024);
        let mut one = [0_u8; 1];
        while bytes.len() < 16 * 1024 {
            let n = stream.read(&mut one).await?;
            if n == 0 {
                break;
            }
            bytes.push(one[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn spawn_socks5_proxy(
        required_auth: Option<(&'static [u8], &'static [u8])>,
    ) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut greeting = [0_u8; 2];
                    if inbound.read_exact(&mut greeting).await.is_err() || greeting[0] != 0x05 {
                        return;
                    }
                    let mut methods = vec![0_u8; greeting[1] as usize];
                    if inbound.read_exact(&mut methods).await.is_err() {
                        return;
                    }
                    let method = if required_auth.is_some() {
                        0x02
                    } else if methods.contains(&0x00) {
                        0x00
                    } else {
                        0xff
                    };
                    if inbound.write_all(&[0x05, method]).await.is_err() || method == 0xff {
                        return;
                    }
                    if let Some((expected_user, expected_pass)) = required_auth {
                        if !read_socks5_auth(&mut inbound, expected_user, expected_pass).await {
                            return;
                        }
                    }
                    let Some(target) = read_socks5_connect_target(&mut inbound).await else {
                        return;
                    };
                    let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = inbound
                        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        addr
    }

    async fn read_socks5_auth(
        inbound: &mut tokio::net::TcpStream,
        expected_user: &[u8],
        expected_pass: &[u8],
    ) -> bool {
        let mut auth_hdr = [0_u8; 2];
        if inbound.read_exact(&mut auth_hdr).await.is_err() || auth_hdr[0] != 0x01 {
            return false;
        }
        let mut user = vec![0_u8; auth_hdr[1] as usize];
        if inbound.read_exact(&mut user).await.is_err() {
            return false;
        }
        let mut pass_len = [0_u8; 1];
        if inbound.read_exact(&mut pass_len).await.is_err() {
            return false;
        }
        let mut pass = vec![0_u8; pass_len[0] as usize];
        if inbound.read_exact(&mut pass).await.is_err() {
            return false;
        }
        let ok = user == expected_user && pass == expected_pass;
        let _ = inbound
            .write_all(&[0x01, if ok { 0x00 } else { 0x01 }])
            .await;
        ok
    }

    async fn read_socks5_connect_target(inbound: &mut tokio::net::TcpStream) -> Option<SocketAddr> {
        let mut header = [0_u8; 4];
        if inbound.read_exact(&mut header).await.is_err() || header[0] != 0x05 || header[1] != 0x01
        {
            return None;
        }
        match header[3] {
            0x01 => {
                let mut octets = [0_u8; 4];
                inbound.read_exact(&mut octets).await.ok()?;
                let port = read_u16(inbound).await?;
                Some(SocketAddr::from((octets, port)))
            }
            0x03 => {
                let mut len = [0_u8; 1];
                inbound.read_exact(&mut len).await.ok()?;
                let mut host = vec![0_u8; len[0] as usize];
                inbound.read_exact(&mut host).await.ok()?;
                let port = read_u16(inbound).await?;
                let host = String::from_utf8_lossy(&host);
                parse_local_authority(&format!("{host}:{port}"))
            }
            _ => None,
        }
    }

    async fn read_u16(inbound: &mut tokio::net::TcpStream) -> Option<u16> {
        let mut port = [0_u8; 2];
        inbound.read_exact(&mut port).await.ok()?;
        Some(u16::from_be_bytes(port))
    }

    fn parse_local_authority(authority: &str) -> Option<SocketAddr> {
        let (host, port) = authority.rsplit_once(':')?;
        let port = port.parse::<u16>().ok()?;
        match host.trim_matches(['[', ']']) {
            "localhost" | "127.0.0.1" => Some(SocketAddr::from(([127, 0, 0, 1], port))),
            "::1" => Some(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port))),
            _ => None,
        }
    }

    fn provider_proxy_yaml() -> &'static str {
        r#"
proxies:
  - name: PROVIDER-HTTP
    type: http
    server: 127.0.0.1
    port: 9
"#
    }

    fn provider_profile_yaml(import_provider_path: &std::path::Path) -> String {
        format!(
            r#"
mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxy-providers:
  LocalProxyProvider:
    type: file
    path: "{}"
rule-providers:
  LocalRuleProvider:
    type: inline
    behavior: classical
    payload:
      - DOMAIN-SUFFIX,provider.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - LocalProxyProvider
    proxies:
      - DIRECT
rules:
  - RULE-SET,LocalRuleProvider,DIRECT
  - MATCH,DIRECT
"#,
            import_provider_path.to_string_lossy()
        )
    }

    fn duplicate_provider_profile_yaml(import_provider_path: &std::path::Path) -> String {
        format!(
            r#"
mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxy-providers:
  Shared:
    type: file
    path: "{}"
rule-providers:
  Shared:
    type: inline
    behavior: classical
    payload:
      - DOMAIN-SUFFIX,provider.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - Shared
    proxies:
      - DIRECT
rules:
  - RULE-SET,Shared,DIRECT
  - MATCH,DIRECT
"#,
            import_provider_path.to_string_lossy()
        )
    }

    async fn wait_for_json(url: &str) -> serde_json::Value {
        let mut last_error = String::new();
        for _ in 0..40 {
            match reqwest::get(url).await {
                Ok(response) if response.status().is_success() => {
                    return response.json().await.expect("JSON response");
                }
                Ok(response) => {
                    last_error = format!("HTTP {}", response.status());
                }
                Err(err) => {
                    last_error = err.to_string();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("{url} did not become ready: {last_error}");
    }

    async fn wait_for_traffic_frame(
        url: &str,
        expected_upload: i64,
        expected_download: i64,
    ) -> serde_json::Value {
        let (mut socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("traffic websocket connects");
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Some(frame) = tokio::time::timeout(remaining, socket.next())
                .await
                .expect("traffic websocket frame before timeout")
            else {
                break;
            };
            let Ok(frame) = frame else {
                continue;
            };
            let Ok(text) = frame.into_text() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if value.get("up").and_then(serde_json::Value::as_i64) == Some(expected_upload)
                && value.get("down").and_then(serde_json::Value::as_i64) == Some(expected_download)
            {
                return value;
            }
        }
        panic!(
            "{url} did not publish expected traffic frame: up={expected_upload}, down={expected_download}"
        );
    }

    async fn spawn_healthcheck_http_server() -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });
        format!("http://{addr}/generate_204")
    }

    async fn spawn_profile_refresh_http_server(good_body: String) -> (String, String) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..2 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0_u8; 1024];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                if request.starts_with("GET /good ") {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/yaml\r\nContent-Length: {}\r\n\r\n{}",
                        good_body.len(),
                        good_body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                }
            }
        });
        (format!("http://{addr}/good"), format!("http://{addr}/bad"))
    }

    async fn spawn_subscription_userinfo_http_server(body: String) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let headers = [
                (
                    "upload=100; download=200; total=1000; expire=1893456000",
                    "Remote%20Sub",
                    "12",
                ),
                (
                    "upload=300; download=400; total=2000; expire=1896048000",
                    "Remote%20Sub%20Updated",
                    "24",
                ),
            ];
            for (userinfo, title, interval) in headers {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let response = format!(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/yaml\r\n",
                        "Subscription-Userinfo: {}\r\n",
                        "Profile-Title: {}\r\n",
                        "Profile-Update-Interval: {}\r\n",
                        "Profile-Web-Page-Url: https://example.test/portal\r\n",
                        "Support-Url: https://example.test/support\r\n",
                        "Content-Length: {}\r\n\r\n{}"
                    ),
                    userinfo,
                    title,
                    interval,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        format!("http://{addr}/sub.yaml")
    }

    async fn spawn_subscription_metadata_comment_http_server(body: String) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/yaml\r\n",
                    "Profile-Title: Header%20Title\r\n",
                    "Content-Length: {}\r\n\r\n{}"
                ),
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        format!("http://{addr}/sub.yaml")
    }
}
