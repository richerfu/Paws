//! Engine ports owned by Paws. No backend or platform UI types cross this boundary.
use async_trait::async_trait;
use paws_model::{VpnEngine, VpnOptions};
use serde::{Deserialize, Serialize};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

mod connections;
mod journal;
mod observation;
pub use connections::EngineConnectionBuffer;
pub use journal::EngineLogBuffer;
pub use observation::{
    EngineConnectionEvent, EngineConnectionPage, EngineConnectionSummary, EngineDnsConfig,
    EngineDnsStatistics, EngineGroupSummary, EngineLogPage, EngineLogRecord, EngineMemory,
    EngineMemoryScope, EngineObservationDomain, EngineProviderKind, EngineProviderState,
    EngineRuntimeRevisions, EngineRuntimeRule,
};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{engine:?} does not support {capability:?}")]
    Unsupported {
        engine: VpnEngine,
        capability: EngineCapability,
    },
    #[error("{engine:?} cannot observe {domain:?}: {reason}")]
    ObservationUnavailable {
        engine: VpnEngine,
        domain: EngineObservationDomain,
        reason: String,
    },
    #[error("{engine:?} is unavailable: {reason}")]
    Unavailable { engine: VpnEngine, reason: String },
    #[error("invalid engine configuration: {0}")]
    InvalidConfig(String),
    #[error("{engine:?} {operation} failed: {message}")]
    Backend {
        engine: VpnEngine,
        operation: &'static str,
        message: String,
    },
    #[error("engine is not running")]
    NotRunning,
    #[error("engine session already running")]
    AlreadyRunning,
    #[error("{primary}; cleanup also failed: {cleanup}")]
    Cleanup {
        primary: Box<EngineError>,
        cleanup: Box<EngineError>,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineCapability {
    Tun,
    Tcp,
    Udp,
    Dns,
    Traffic,
    RuntimeLog,
    Reload,
    TrimMemory,
    ProxySelection,
    ProxyUnfix,
    Connections,
    Providers,
    DnsCacheFlush,
    FakeIpCacheFlush,
    ProxyDelay,
    ProxyEcho,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    pub engine: VpnEngine,
    pub operations: Vec<EngineCapability>,
}
impl EngineCapabilities {
    pub fn supports(&self, operation: EngineCapability) -> bool {
        self.operations.contains(&operation)
    }
    pub fn require(&self, operation: EngineCapability) -> Result<(), EngineError> {
        if self.supports(operation) {
            Ok(())
        } else {
            Err(EngineError::Unsupported {
                engine: self.engine,
                capability: operation,
            })
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineTraffic {
    pub upload_speed: u64,
    pub download_speed: u64,
    pub upload_total: u64,
    pub download_total: u64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineHttpProxyEndpoint {
    pub port: u16,
    pub token: String,
}
impl std::fmt::Debug for EngineHttpProxyEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineHttpProxyEndpoint")
            .field("port", &self.port)
            .field("token", &"[redacted]")
            .finish()
    }
}
impl EngineHttpProxyEndpoint {
    pub fn proxy_url(&self) -> String {
        format!("http://paws-app:{}@127.0.0.1:{}", self.token, self.port)
    }
}
#[derive(Debug, Clone, Default)]
pub struct EngineServices {
    pub controller_endpoint: Option<EngineControllerEndpoint>,
    pub public_controller_addr: Option<SocketAddr>,
    pub app_http_proxy: Option<EngineHttpProxyEndpoint>,
    pub benchmark_proxy: Option<EngineHttpProxyEndpoint>,
    pub mixed_ready: bool,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct EngineServicePlan {
    pub enabled: bool,
    pub network_protected: bool,
    pub network: EngineNetworkServices,
    pub benchmark_port: u16,
}

/// User-facing listeners are independent of the credentialed private engine
/// management endpoint. Disabled services must not reserve their public port.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineNetworkServices {
    pub ports: paws_model::NetworkPortConfig,
    pub access: paws_model::ControllerAccessConfig,
}
impl EngineNetworkServices {
    /// Ports occupied by private management and the three protected benchmark
    /// listeners must never be repurposed as user-facing services.
    pub fn validate(&self) -> Result<(), EngineError> {
        self.ports
            .validate()
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
        for (enabled, port, service) in [
            (
                self.ports.mixed_enabled,
                self.ports.mixed_port,
                "mixed proxy",
            ),
            (
                self.ports.controller_enabled,
                self.ports.controller_port,
                "public controller",
            ),
        ] {
            if enabled && [18490, 18491, 18492, 19090].contains(&port) {
                return Err(EngineError::InvalidConfig(format!(
                    "{service} port {port} is reserved for private engine services"
                )));
            }
        }
        Ok(())
    }
}
impl std::fmt::Debug for EngineNetworkServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineNetworkServices")
            .field("ports", &self.ports)
            .field("allow_lan", &self.access.allow_lan)
            .field("secret", &self.access.secret.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineControllerEndpoint {
    pub address: SocketAddr,
    pub secret: Option<String>,
}
/// A single ownership read: callers must retain this credentialed endpoint
/// with its client when checking whether an asynchronous result still belongs
/// to the applied runtime. Credentials rotate on every prepared Go reload.
#[derive(Clone)]
pub struct EngineManagementBinding {
    pub client: Arc<dyn EngineManagement>,
    pub endpoint: EngineControllerEndpoint,
}

#[derive(Debug, Clone)]
pub struct EngineProxySelection {
    pub selected: Option<String>,
    pub fixed: Option<String>,
}
impl std::fmt::Debug for EngineControllerEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineControllerEndpoint")
            .field("address", &self.address)
            .field("secret", &self.secret.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to boxed futures"
)]
#[async_trait]
pub trait EngineManagement: Send + Sync {
    fn capabilities(&self) -> EngineCapabilities;
    async fn runtime_revisions(&self) -> Result<EngineRuntimeRevisions, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Groups))
    }
    async fn connection_summary(&self) -> Result<EngineConnectionSummary, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::ConnectionEvents))
    }
    async fn proxy_group(&self, _name: &str) -> Result<paws_model::ProxyGroup, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Groups))
    }
    async fn group_summaries(&self) -> Result<Vec<EngineGroupSummary>, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Groups))
    }
    async fn provider_state(
        &self,
        _kind: EngineProviderKind,
        _name: &str,
    ) -> Result<EngineProviderState, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Provider))
    }
    async fn runtime_rules(&self) -> Result<Vec<EngineRuntimeRule>, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Rules))
    }
    async fn dns_statistics(&self) -> Result<EngineDnsStatistics, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Dns))
    }
    async fn dns_config(&self) -> Result<EngineDnsConfig, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Dns))
    }
    async fn memory(&self) -> Result<EngineMemory, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Memory))
    }
    async fn runtime_logs(&self, _after: u64, _limit: u16) -> Result<EngineLogPage, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::Logs))
    }
    async fn connection_events(
        &self,
        _after: u64,
        _limit: u16,
    ) -> Result<EngineConnectionPage, EngineError> {
        Err(self.observation_unavailable(EngineObservationDomain::ConnectionEvents))
    }
    fn observation_unavailable(&self, domain: EngineObservationDomain) -> EngineError {
        EngineError::ObservationUnavailable {
            engine: self.capabilities().engine,
            domain,
            reason: "the backend has no observer for this domain".into(),
        }
    }
    async fn select_proxy(&self, _group: &str, _proxy: Option<&str>) -> Result<(), EngineError> {
        Err(EngineError::Unsupported {
            engine: self.capabilities().engine,
            capability: EngineCapability::ProxySelection,
        })
    }
    async fn proxy_selection(&self, _group: &str) -> Result<EngineProxySelection, EngineError> {
        Err(EngineError::Unsupported {
            engine: self.capabilities().engine,
            capability: EngineCapability::ProxySelection,
        })
    }
    async fn proxy_groups(&self) -> Result<Vec<paws_model::ProxyGroup>, EngineError> {
        Err(EngineError::Unsupported {
            engine: self.capabilities().engine,
            capability: EngineCapability::ProxySelection,
        })
    }
    async fn proxy_echo(
        &self,
        _proxy: &str,
        _url: &str,
        _payload: &str,
        _timeout: Duration,
    ) -> Result<String, EngineError> {
        Err(EngineError::Unsupported {
            engine: self.capabilities().engine,
            capability: EngineCapability::ProxyEcho,
        })
    }
    async fn connections(&self) -> Result<Vec<paws_model::ConnectionSummary>, EngineError>;
    async fn close_connections(&self, id: Option<&str>) -> Result<(), EngineError>;
    async fn flush_dns(&self, fake_ip: bool) -> Result<(), EngineError>;
    async fn proxy_delay(
        &self,
        proxy: &str,
        url: &str,
        timeout: Duration,
    ) -> Result<u16, EngineError>;
    async fn refresh_provider(&self, kind: &str, name: &str) -> Result<(), EngineError>;
    async fn provider_members(
        &self,
        _name: &str,
    ) -> Result<Vec<paws_model::ProviderProxySummary>, EngineError> {
        Err(EngineError::Unsupported {
            engine: self.capabilities().engine,
            capability: EngineCapability::Providers,
        })
    }
    async fn provider_health(
        &self,
        name: &str,
        proxy: Option<&str>,
        url: Option<&str>,
        timeout: Duration,
        expected_status: Option<&str>,
    ) -> Result<Option<u16>, EngineError>;
}

#[derive(Debug, Clone)]
pub enum EngineConfig {
    Meow,
    SingBox {
        config_json: String,
        work_dir: PathBuf,
    },
    Mihomo {
        init_params: String,
        setup_params: String,
        controller_endpoint: EngineControllerEndpoint,
    },
}
impl EngineConfig {
    pub fn engine(&self) -> VpnEngine {
        match self {
            Self::Meow => VpnEngine::Meow,
            Self::SingBox { .. } => VpnEngine::SingBox,
            Self::Mihomo { .. } => VpnEngine::Mihomo,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineStart {
    /// Borrowed platform descriptor. Backends must duplicate it before owning it.
    pub tun_fd: i32,
    pub underlying_network: i32,
    pub options: VpnOptions,
    pub config: EngineConfig,
}

#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to boxed futures"
)]
#[async_trait]
pub trait EngineSession: Send + Sync {
    /// Optional sink for the host's existing tracing layer. A session owns its
    /// journal; installing a second process-global subscriber is unnecessary.
    fn runtime_log_sink(&self) -> Option<Arc<EngineLogBuffer>> {
        None
    }
    fn engine(&self) -> VpnEngine;
    fn capabilities(&self) -> EngineCapabilities;
    fn management_binding(&self) -> Option<EngineManagementBinding> {
        None
    }
    fn management(&self) -> Option<Arc<dyn EngineManagement>> {
        None
    }
    fn controller_endpoint(&self) -> Option<EngineControllerEndpoint> {
        None
    }
    fn services(&self) -> EngineServices {
        EngineServices::default()
    }
    fn stop_services(&self) -> Result<(), EngineError> {
        Ok(())
    }
    fn stop_public_listener(&self) -> Result<(), EngineError> {
        Ok(())
    }
    async fn start(&self, request: EngineStart) -> Result<u64, EngineError>;
    async fn stop(&self) -> Result<(), EngineError>;
    async fn reload(
        &self,
        _config: EngineConfig,
        _underlying_network: i32,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported {
            engine: self.engine(),
            capability: EngineCapability::Reload,
        })
    }
    fn is_running(&self) -> bool;
    fn traffic(&self) -> Result<EngineTraffic, EngineError>;
    fn version(&self) -> Result<String, EngineError>;
    fn runtime_log(&self) -> Result<String, EngineError> {
        Err(EngineError::Unsupported {
            engine: self.engine(),
            capability: EngineCapability::RuntimeLog,
        })
    }
    fn trim_memory(&self) -> Result<(), EngineError> {
        Err(EngineError::Unsupported {
            engine: self.engine(),
            capability: EngineCapability::TrimMemory,
        })
    }
}

/// Optional host OS capability. Missing process identity remains unknown; an
/// adapter must never manufacture a UID or package name from a failed query.
pub trait ProcessResolver: Send + Sync {
    fn resolve_uid(&self, protocol: i32, source: &str, target: &str) -> Option<i32>;
    fn resolve_package(&self, uid: i32) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Network {
    #[default]
    Tcp,
    Udp,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnType {
    #[default]
    Inner,
}
#[derive(Debug, Clone, Default)]
pub struct FlowMetadata {
    pub network: Network,
    pub conn_type: ConnType,
    pub src_ip: Option<IpAddr>,
    pub src_port: u16,
    pub dst_ip: Option<IpAddr>,
    pub dst_port: u16,
    pub host: String,
    pub sniff_host: String,
    pub in_name: String,
    pub in_port: u16,
}
impl FlowMetadata {
    pub fn source_address(&self) -> String {
        self.src_ip
            .map(|ip| SocketAddr::new(ip, self.src_port).to_string())
            .unwrap_or_default()
    }
    pub fn remote_address(&self) -> String {
        self.dst_ip
            .map(|ip| SocketAddr::new(ip, self.dst_port).to_string())
            .unwrap_or_else(|| format!("{}:{}", self.host, self.dst_port))
    }
}

pub trait ProxyStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> ProxyStream for T {}

#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to boxed futures"
)]
#[async_trait]
pub trait PacketConnection: Send + Sync {
    async fn read_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), EngineError>;
    async fn write_packet(&self, buffer: &[u8], address: &SocketAddr)
        -> Result<usize, EngineError>;
    fn local_addr(&self) -> Result<SocketAddr, EngineError>;
    fn close(&self) -> Result<(), EngineError>;
}

#[derive(Debug, Clone)]
pub struct SnifferConfig {
    pub enable: bool,
    pub override_destination: bool,
    pub parse_pure_ip: bool,
    pub tls_ports: Vec<u16>,
    pub http_ports: Vec<u16>,
    pub force_domain: Vec<String>,
    pub skip_domain: Vec<String>,
    pub timeout: Duration,
}
impl Default for SnifferConfig {
    fn default() -> Self {
        Self {
            enable: false,
            override_destination: false,
            parse_pure_ip: true,
            tls_ports: vec![443, 8443],
            http_ports: vec![80, 8080, 8880],
            force_domain: Vec::new(),
            skip_domain: Vec::new(),
            timeout: Duration::from_millis(100),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum SniffProtocol {
    Tls,
    Http,
}

/// Paws' packet stack speaks this port; concrete engine metadata stays in adapters.
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to boxed futures"
)]
#[async_trait]
pub trait EngineTunnel: Send + Sync {
    async fn handle_tcp(&self, stream: Box<dyn ProxyStream>, metadata: FlowMetadata);
    async fn open_udp(
        &self,
        metadata: &mut FlowMetadata,
    ) -> Result<Arc<dyn PacketConnection>, EngineError>;
    async fn dns_query(&self, query: &[u8]) -> Result<Vec<u8>, EngineError>;
    fn sniffer_config(&self) -> SnifferConfig;
    fn sniff(&self, protocol: SniffProtocol, payload: &[u8]) -> Option<String>;
    fn should_sniff(&self, metadata: &FlowMetadata) -> bool;
    fn apply_sniff(&self, host: &str, metadata: &mut FlowMetadata);
}
