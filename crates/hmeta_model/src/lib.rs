use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const DEFAULT_CHINA_DNS_SERVERS: &[&str] = &["223.5.5.5", "119.29.29.29"];
pub const DEFAULT_GLOBAL_DNS_FALLBACKS: &[&str] = &["1.1.1.1", "8.8.8.8"];
pub const DEFAULT_CHINA_DNS_POLICY_MATCHER: &str = "geosite:cn";
pub const DEFAULT_GLOBAL_DNS_POLICY_MATCHER: &str = "geosite:geolocation-!cn";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeMode {
    Rule,
    Global,
    Direct,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }
}

impl TryFrom<&str> for RuntimeMode {
    type Error = HMetaError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "rule" => Ok(Self::Rule),
            "global" => Ok(Self::Global),
            "direct" => Ok(Self::Direct),
            other => Err(HMetaError::InvalidMode(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PerAppMode {
    Off,
    Proxy,
    Bypass,
}

impl PerAppMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Proxy => "proxy",
            Self::Bypass => "bypass",
        }
    }
}

impl TryFrom<&str> for PerAppMode {
    type Error = HMetaError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "off" | "disabled" | "none" => Ok(Self::Off),
            "proxy" | "allow" | "allowlist" | "include" => Ok(Self::Proxy),
            "bypass" | "block" | "blocklist" | "exclude" => Ok(Self::Bypass),
            other => Err(HMetaError::Core(format!(
                "invalid per-app VPN mode: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnOptions {
    #[serde(default = "default_vpn_addresses")]
    pub addresses: Vec<String>,
    pub mtu: u16,
    pub ipv6: bool,
    pub system_proxy: bool,
    pub dns_hijacking: bool,
    pub allow_bypass: bool,
    pub stack: String,
    pub routes: Vec<String>,
    #[serde(default = "default_vpn_dns_addresses")]
    pub dns_addresses: Vec<String>,
    pub dns_servers: Vec<String>,
    #[serde(default)]
    pub dns_fallbacks: Vec<String>,
    #[serde(default)]
    pub dns_nameserver_policy: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub per_app_mode: PerAppMode,
    pub trusted_applications: Vec<String>,
    pub blocked_applications: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApplication {
    pub bundle_name: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsSnapshot {
    pub model: String,
    pub hijacking: bool,
    pub listen: String,
    pub upstreams: Vec<String>,
    #[serde(default)]
    pub fallbacks: Vec<String>,
    #[serde(default)]
    pub nameserver_policy: BTreeMap<String, Vec<String>>,
    pub tun_addresses: Vec<String>,
    pub handled_packets: u64,
    #[serde(default)]
    pub cache_hits: u64,
    #[serde(default)]
    pub cache_misses: u64,
    #[serde(default)]
    pub recent_queries: Vec<DnsQuerySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsQuerySummary {
    pub name: String,
    pub record_type: String,
    pub count: u64,
}

impl Default for DnsSnapshot {
    fn default() -> Self {
        let options = VpnOptions::default();
        Self {
            model: "tun-hijack".to_owned(),
            hijacking: options.dns_hijacking,
            listen: "127.0.0.1:1053".to_owned(),
            upstreams: options.dns_servers,
            fallbacks: options.dns_fallbacks,
            nameserver_policy: options.dns_nameserver_policy,
            tun_addresses: options.dns_addresses,
            handled_packets: 0,
            cache_hits: 0,
            cache_misses: 0,
            recent_queries: Vec::new(),
        }
    }
}

impl Default for VpnOptions {
    fn default() -> Self {
        Self {
            addresses: default_vpn_addresses(),
            mtu: 1500,
            ipv6: false,
            system_proxy: false,
            dns_hijacking: true,
            allow_bypass: false,
            stack: "netstack-smoltcp".to_owned(),
            routes: vec!["0.0.0.0/0".to_owned()],
            dns_addresses: default_vpn_dns_addresses(),
            dns_servers: default_china_dns_servers(),
            dns_fallbacks: default_global_dns_fallbacks(),
            dns_nameserver_policy: default_dns_policy(),
            per_app_mode: PerAppMode::Off,
            trusted_applications: Vec::new(),
            blocked_applications: Vec::new(),
        }
    }
}

impl Default for PerAppMode {
    fn default() -> Self {
        Self::Off
    }
}

fn default_vpn_addresses() -> Vec<String> {
    vec!["172.19.0.1/30".to_owned()]
}

fn default_vpn_dns_addresses() -> Vec<String> {
    vec!["172.19.0.2".to_owned()]
}

fn default_china_dns_servers() -> Vec<String> {
    DEFAULT_CHINA_DNS_SERVERS
        .iter()
        .map(|server| (*server).to_owned())
        .collect()
}

fn default_global_dns_fallbacks() -> Vec<String> {
    DEFAULT_GLOBAL_DNS_FALLBACKS
        .iter()
        .map(|server| (*server).to_owned())
        .collect()
}

fn default_dns_policy() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        (
            DEFAULT_CHINA_DNS_POLICY_MATCHER.to_owned(),
            default_china_dns_servers(),
        ),
        (
            DEFAULT_GLOBAL_DNS_POLICY_MATCHER.to_owned(),
            default_global_dns_fallbacks(),
        ),
    ])
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficSnapshot {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub upload_speed: u64,
    pub download_speed: u64,
    #[serde(default)]
    pub tun_upload_bytes: u64,
    #[serde(default)]
    pub tun_download_bytes: u64,
    #[serde(default)]
    pub tun_upload_speed: u64,
    #[serde(default)]
    pub tun_download_speed: u64,
    #[serde(default)]
    pub meow_upload_bytes: u64,
    #[serde(default)]
    pub meow_download_bytes: u64,
    #[serde(default)]
    pub meow_upload_speed: u64,
    #[serde(default)]
    pub meow_download_speed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficHistoryPoint {
    pub download_speed: u64,
    pub upload_speed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyItem {
    pub name: String,
    pub proxy_type: String,
    pub delay_ms: Option<u32>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyGroup {
    pub name: String,
    pub group_type: String,
    pub selected: Option<String>,
    /// `None` for manual/non-selectable groups, `Some("")` for an automatic
    /// group in auto mode, and `Some(name)` when URLTest/Fallback is pinned.
    #[serde(default)]
    pub fixed: Option<String>,
    pub proxies: Vec<ProxyItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub raw_yaml_path: String,
    #[serde(default)]
    pub runtime_yaml_path: String,
    pub active: bool,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub last_refresh_at: Option<String>,
    #[serde(default)]
    pub last_refresh_error: Option<String>,
    pub subscription_url: Option<String>,
    pub rule_count: usize,
    #[serde(default)]
    pub selected_proxies: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub has_backup: bool,
    #[serde(default)]
    pub upload_bytes: u64,
    #[serde(default)]
    pub download_bytes: u64,
    #[serde(default)]
    pub subscription_user_info: Option<SubscriptionUserInfo>,
    #[serde(default)]
    pub subscription_metadata: Option<SubscriptionMetadata>,
    #[serde(default)]
    pub next_refresh_at: Option<String>,
    #[serde(default)]
    pub refresh_due: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionUserInfo {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub expire_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionMetadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub update_interval_hours: Option<u64>,
    #[serde(default)]
    pub web_page_url: Option<String>,
    #[serde(default)]
    pub support_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSummary {
    pub id: String,
    pub profile_id: String,
    pub line: String,
    pub enabled: bool,
    pub order: u32,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManualRuleMatchKind {
    Domain,
    DomainSuffix,
    IpCidr,
}

impl ManualRuleMatchKind {
    pub fn rule_type(self, ipv6: bool) -> &'static str {
        match self {
            Self::Domain => "DOMAIN",
            Self::DomainSuffix => "DOMAIN-SUFFIX",
            Self::IpCidr if ipv6 => "IP-CIDR6",
            Self::IpCidr => "IP-CIDR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRuleSpec {
    pub match_kind: ManualRuleMatchKind,
    pub value: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManualRuleMutationKind {
    Added,
    Updated,
    Reenabled,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRuleMutation {
    pub rule_id: String,
    pub line: String,
    pub kind: ManualRuleMutationKind,
    pub replaced_line: Option<String>,
    pub removed_duplicates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub name: String,
    pub provider_type: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub vehicle_type: Option<String>,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub exclude_filter: Option<String>,
    #[serde(default)]
    pub behavior: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub health_check_enabled: bool,
    #[serde(default)]
    pub health_check_url: Option<String>,
    #[serde(default)]
    pub health_check_interval_seconds: Option<u64>,
    #[serde(default)]
    pub expected_status: Option<String>,
    #[serde(default)]
    pub members: Vec<ProviderProxySummary>,
    #[serde(default)]
    pub cache_exists: bool,
    #[serde(default)]
    pub cache_bytes: Option<u64>,
    #[serde(default)]
    pub cache_updated_at: Option<String>,
    #[serde(default)]
    pub stale_cache_available: bool,
    #[serde(default)]
    pub last_refresh_at: Option<String>,
    #[serde(default)]
    pub last_refresh_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProxySummary {
    pub name: String,
    pub proxy_type: String,
    pub alive: bool,
    #[serde(default)]
    pub delay_ms: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerDiagnostics {
    #[serde(default)]
    pub memory_in_use_bytes: u64,
    #[serde(default)]
    pub memory_limit_bytes: u64,
    #[serde(default)]
    pub config_sync_count: u64,
    #[serde(default)]
    pub last_config_sync_at: Option<String>,
    #[serde(default)]
    pub last_config_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeodataFileSummary {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub exists: bool,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    pub id: String,
    pub host: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: u16,
    pub network: String,
    pub rule: String,
    #[serde(default)]
    pub rule_payload: String,
    pub proxy: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub started_at: String,
    pub upload_bytes: u64,
    pub download_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub id: String,
    pub host: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: u16,
    pub network: String,
    pub rule: String,
    pub proxy: String,
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub active: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutSnapshot {
    pub app_version: String,
    pub core_version: String,
    pub meow_rs_version: String,
    pub arkit_rev: String,
    pub rust_version: String,
    pub privacy_summary: Vec<String>,
}

impl Default for AboutSnapshot {
    fn default() -> Self {
        Self {
            app_version: "0.1.0".to_owned(),
            core_version: "0.1.0".to_owned(),
            meow_rs_version: "unknown".to_owned(),
            arkit_rev: "unknown".to_owned(),
            rust_version: "unknown".to_owned(),
            privacy_summary: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VpnLifecycle {
    Stopped,
    EngineLoaded,
    Starting,
    Connected,
    ProtectFailed,
    Failed,
}

impl Default for VpnLifecycle {
    fn default() -> Self {
        Self::Stopped
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    #[serde(default)]
    pub vpn_lifecycle: VpnLifecycle,
    #[serde(default)]
    pub engine_loaded: bool,
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
    pub active_profile: Option<String>,
    pub mode: RuntimeMode,
    pub traffic: TrafficSnapshot,
    #[serde(default)]
    pub traffic_history: Vec<TrafficHistoryPoint>,
    #[serde(default)]
    pub dns: DnsSnapshot,
    #[serde(default)]
    pub vpn_options: VpnOptions,
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
            vpn_lifecycle: VpnLifecycle::Stopped,
            engine_loaded: false,
            running: false,
            vpn_running: false,
            network_protected: false,
            network_protect_error: None,
            controller_running: false,
            controller_addr: None,
            controller_diagnostics: ControllerDiagnostics::default(),
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
            proxy_groups: Vec::new(),
            profiles: Vec::new(),
            rules: Vec::new(),
            providers: Vec::new(),
            geodata: Vec::new(),
            logs: vec![LogEntry {
                level: "info".to_owned(),
                message: "hmeta core initialized".to_owned(),
                timestamp: "boot".to_owned(),
            }],
            connections: Vec::new(),
            request_history: Vec::new(),
            about: AboutSnapshot::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HMetaError {
    #[error("invalid mode: {0}")]
    InvalidMode(String),
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("vpn is not running")]
    VpnNotRunning,
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("rule not found: {0}")]
    RuleNotFound(String),
    #[error("platform callback not registered: {0}")]
    MissingCallback(String),
    #[error("core error: {0}")]
    Core(String),
    #[error("io error: {0}")]
    Io(String),
}

pub fn to_json<T: Serialize>(value: &T) -> Result<String, HMetaError> {
    serde_json::to_string(value).map_err(|err| HMetaError::InvalidJson(err.to_string()))
}

pub fn from_json<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, HMetaError> {
    serde_json::from_str(value).map_err(|err| HMetaError::InvalidJson(err.to_string()))
}
