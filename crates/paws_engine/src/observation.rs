//! Individually queried engine observations. These are never a UI snapshot.
use paws_model::{ConnectionSummary, DnsQuerySummary, ProviderProxySummary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineObservationDomain {
    Groups,
    Provider,
    Rules,
    Dns,
    Memory,
    Logs,
    ConnectionEvents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineGroupSummary {
    pub name: String,
    pub group_type: String,
    pub selected: Option<String>,
    pub fixed: Option<String>,
    pub member_count: usize,
    pub selected_delay_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineProviderKind {
    Proxy,
    Rule,
}
impl EngineProviderKind {
    pub fn path(self) -> &'static str {
        match self {
            Self::Proxy => "proxies",
            Self::Rule => "rules",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineProviderState {
    pub kind: EngineProviderKind,
    pub name: String,
    /// Only proxy providers have a member roster. An empty roster is real data.
    pub members: Option<Vec<ProviderProxySummary>>,
    pub rule_count: Option<usize>,
    pub updated_at: Option<String>,
    pub vehicle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeRule {
    pub rule_type: String,
    pub payload: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDnsStatistics {
    pub handled_packets: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub recent_queries: Vec<DnsQuerySummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineMemoryScope {
    /// The resident memory of the owning process, including its host runtime.
    ProcessResident,
    /// Memory reported by the backend's own runtime allocator.
    EngineHeap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDnsConfig {
    pub model: String,
    pub listen: Option<String>,
    pub upstreams: Vec<String>,
    pub fallbacks: Vec<String>,
    pub nameserver_policy: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineMemory {
    pub scope: EngineMemoryScope,
    pub in_use_bytes: u64,
    pub limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineLogRecord {
    pub sequence: u64,
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineLogPage {
    pub entries: Vec<EngineLogRecord>,
    pub next_cursor: u64,
    pub high_watermark: u64,
    /// First retained sequence, when the requested cursor has been evicted.
    pub dropped_before: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConnectionEvent {
    pub sequence: u64,
    pub connection: ConnectionSummary,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConnectionPage {
    pub entries: Vec<EngineConnectionEvent>,
    pub next_cursor: u64,
    pub high_watermark: u64,
    pub dropped_before: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConnectionSummary {
    pub count: usize,
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub recent: Vec<ConnectionSummary>,
    pub latest_rule_node: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeRevisions {
    pub groups: u64,
    pub providers: u64,
    pub rules: u64,
    pub dns: u64,
    pub connections: u64,
    pub logs: u64,
}
