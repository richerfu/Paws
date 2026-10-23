use async_trait::async_trait;
use paws_engine::{
    EngineCapabilities, EngineCapability, EngineConnectionPage, EngineConnectionSummary,
    EngineControllerEndpoint, EngineDnsConfig, EngineDnsStatistics, EngineError,
    EngineGroupSummary, EngineLogPage, EngineManagement, EngineMemory, EngineObservationDomain,
    EngineProviderKind, EngineProviderState, EngineRuntimeRevisions, EngineRuntimeRule,
};
use paws_model::{ConnectionSummary, VpnEngine};
use serde::Deserialize;
use std::time::Duration;

pub struct ControllerClient {
    engine: VpnEngine,
    endpoint: EngineControllerEndpoint,
    client: reqwest::Client,
}
impl ControllerClient {
    async fn observation<T: serde::de::DeserializeOwned>(
        &self,
        domain: EngineObservationDomain,
        path: &[&str],
        query: &[(&str, String)],
    ) -> Result<T, EngineError> {
        let response = self
            .request(reqwest::Method::GET, path)?
            .query(query)
            .send()
            .await
            .map_err(|_| self.error("runtime observation", "controller request failed"))?;
        if matches!(response.status().as_u16(), 404 | 501) {
            return Err(EngineError::ObservationUnavailable {
                engine: self.engine,
                domain,
                reason: "runtime observer is unavailable in this backend".into(),
            });
        }
        if !response.status().is_success() {
            return Err(self.error(
                "runtime observation",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        response
            .json()
            .await
            .map_err(|_| self.error("runtime observation", "invalid observation schema"))
    }
    /// An acknowledgment from the authenticated native owner. Only actual
    /// listener handles/registries may populate this response; it contains no
    /// credential. Reuse this client's captured epoch when constructing URLs.
    pub(crate) async fn applied_services(
        &self,
    ) -> Result<paws_engine::EngineServices, EngineError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Applied {
            benchmark_port: u16,
            public_controller_addr: Option<std::net::SocketAddr>,
            mixed_ready: bool,
        }
        let response = self
            .request(
                reqwest::Method::GET,
                &["paws", "observations", "applied-services"],
            )?
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .map_err(|_| self.error("applied services", "native acknowledgment failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "applied services",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let applied: Applied = response
            .json()
            .await
            .map_err(|_| self.error("applied services", "invalid acknowledgment schema"))?;
        let expected = match self.engine {
            VpnEngine::Mihomo => 18490,
            VpnEngine::SingBox => 18492,
            _ => {
                return Err(self.error(
                    "applied services",
                    "Go acknowledgment requested for a different engine",
                ));
            }
        };
        if applied.benchmark_port != expected {
            return Err(self.error(
                "applied services",
                "private proxy listener does not match this engine",
            ));
        }
        let token = self
            .endpoint
            .secret
            .as_ref()
            .filter(|secret| !secret.is_empty())
            .ok_or_else(|| self.error("applied services", "owner credential is absent"))?;
        let proxy = paws_engine::EngineHttpProxyEndpoint {
            port: applied.benchmark_port,
            token: token.clone(),
        };
        Ok(paws_engine::EngineServices {
            controller_endpoint: Some(self.endpoint.clone()),
            public_controller_addr: applied.public_controller_addr,
            app_http_proxy: Some(proxy.clone()),
            benchmark_proxy: Some(proxy),
            mixed_ready: applied.mixed_ready,
            warnings: Vec::new(),
        })
    }
    pub fn new(engine: VpnEngine, endpoint: EngineControllerEndpoint) -> Result<Self, EngineError> {
        if !matches!(
            engine,
            VpnEngine::Mihomo | VpnEngine::SingBox | VpnEngine::Meow
        ) {
            return Err(EngineError::InvalidConfig(
                "controller requires a concrete engine".into(),
            ));
        }
        if !endpoint.address.ip().is_loopback() {
            return Err(EngineError::InvalidConfig(
                "native controller must use loopback".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
        Ok(Self {
            engine,
            endpoint,
            client,
        })
    }
    fn error(&self, operation: &'static str, message: impl Into<String>) -> EngineError {
        EngineError::Backend {
            engine: self.engine,
            operation,
            message: message.into(),
        }
    }
    fn request(
        &self,
        method: reqwest::Method,
        path: &[&str],
    ) -> Result<reqwest::RequestBuilder, EngineError> {
        let mut url = reqwest::Url::parse(&format!("http://{}", self.endpoint.address))
            .map_err(|error| self.error("controller address", error.to_string()))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| self.error("controller address", "address has no path"))?;
            for part in path {
                segments.push(part);
            }
        }
        let mut request = self.client.request(method, url);
        if let Some(secret) = &self.endpoint.secret {
            request = request.bearer_auth(secret);
        }
        Ok(request)
    }
    async fn done(
        &self,
        request: reqwest::RequestBuilder,
        operation: &'static str,
    ) -> Result<(), EngineError> {
        let response = request
            .send()
            .await
            .map_err(|_| self.error(operation, "controller request failed"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(self.error(
                operation,
                format!("controller HTTP {}", response.status().as_u16()),
            ))
        }
    }
    pub fn descriptor(engine: VpnEngine) -> EngineCapabilities {
        let mut operations = vec![
            EngineCapability::ProxySelection,
            EngineCapability::Connections,
            EngineCapability::ProxyDelay,
            EngineCapability::FakeIpCacheFlush,
            EngineCapability::DnsCacheFlush,
        ];
        operations.extend([
            EngineCapability::ProxyEcho,
            EngineCapability::Providers,
            EngineCapability::ProxyUnfix,
        ]);
        EngineCapabilities { engine, operations }
    }
}
#[derive(Deserialize)]
struct Connections {
    connections: Vec<NativeConnection>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeConnection {
    id: String,
    metadata: NativeMetadata,
    #[serde(default)]
    upload: u64,
    #[serde(default)]
    download: u64,
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default)]
    start: String,
    #[serde(default)]
    rule: String,
    #[serde(default)]
    rule_payload: String,
}
#[derive(Deserialize)]
struct NativeMetadata {
    #[serde(default)]
    host: String,
    #[serde(rename = "destinationIP", default)]
    destination_ip: String,
    #[serde(rename = "destinationPort", default)]
    destination_port: serde_json::Value,
    #[serde(default)]
    network: String,
}
impl From<NativeConnection> for ConnectionSummary {
    fn from(value: NativeConnection) -> Self {
        let port = value
            .metadata
            .destination_port
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .or_else(|| {
                value
                    .metadata
                    .destination_port
                    .as_str()
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0);
        Self {
            id: value.id,
            host: if value.metadata.host.is_empty() {
                value.metadata.destination_ip.clone()
            } else {
                value.metadata.host.clone()
            },
            domain: value.metadata.host.clone(),
            domain_source: if value.metadata.host.is_empty() {
                "unavailable"
            } else {
                "engine-controller"
            }
            .into(),
            destination_ip: value.metadata.destination_ip,
            destination_port: port,
            network: value.metadata.network,
            rule: value.rule,
            rule_payload: value.rule_payload,
            proxy: value.chains.first().cloned().unwrap_or_default(),
            chains: value.chains,
            started_at: value.start,
            upload_bytes: value.upload,
            download_bytes: value.download,
        }
    }
}
#[async_trait]
impl EngineManagement for ControllerClient {
    async fn runtime_revisions(&self) -> Result<EngineRuntimeRevisions, EngineError> {
        self.observation(
            EngineObservationDomain::Groups,
            &["paws", "observations", "revisions"],
            &[],
        )
        .await
    }
    async fn connection_summary(&self) -> Result<EngineConnectionSummary, EngineError> {
        let summary: EngineConnectionSummary = self
            .observation(
                EngineObservationDomain::ConnectionEvents,
                &["paws", "observations", "connection-summary"],
                &[],
            )
            .await?;
        if summary.recent.len() > 5 {
            return Err(self.error(
                "connection summary",
                "recent entries exceed bounded summary",
            ));
        }
        Ok(summary)
    }
    async fn proxy_group(&self, name: &str) -> Result<paws_model::ProxyGroup, EngineError> {
        let group: paws_model::ProxyGroup = self
            .observation(
                EngineObservationDomain::Groups,
                &["paws", "observations", "proxy-groups", name],
                &[],
            )
            .await?;
        if group.name != name {
            return Err(self.error("proxy group", "group identity mismatched"));
        }
        Ok(group)
    }
    async fn group_summaries(&self) -> Result<Vec<EngineGroupSummary>, EngineError> {
        self.observation(
            EngineObservationDomain::Groups,
            &["paws", "observations", "groups"],
            &[],
        )
        .await
    }
    async fn provider_state(
        &self,
        kind: EngineProviderKind,
        name: &str,
    ) -> Result<EngineProviderState, EngineError> {
        let state: EngineProviderState = self
            .observation(
                EngineObservationDomain::Provider,
                &["paws", "observations", "providers", kind.path(), name],
                &[],
            )
            .await?;
        if state.name != name
            || state.kind != kind
            || (kind == EngineProviderKind::Proxy && state.members.is_none())
            || (kind == EngineProviderKind::Rule && state.rule_count.is_none())
        {
            return Err(self.error(
                "provider state",
                "provider identity or kind-specific data mismatched",
            ));
        }
        Ok(state)
    }
    async fn runtime_rules(&self) -> Result<Vec<EngineRuntimeRule>, EngineError> {
        self.observation(
            EngineObservationDomain::Rules,
            &["paws", "observations", "rules"],
            &[],
        )
        .await
    }
    async fn dns_statistics(&self) -> Result<EngineDnsStatistics, EngineError> {
        self.observation(
            EngineObservationDomain::Dns,
            &["paws", "observations", "dns"],
            &[],
        )
        .await
    }
    async fn dns_config(&self) -> Result<EngineDnsConfig, EngineError> {
        self.observation(
            EngineObservationDomain::Dns,
            &["paws", "observations", "dns-config"],
            &[],
        )
        .await
    }
    async fn memory(&self) -> Result<EngineMemory, EngineError> {
        self.observation(
            EngineObservationDomain::Memory,
            &["paws", "observations", "memory"],
            &[],
        )
        .await
    }
    async fn runtime_logs(&self, after: u64, limit: u16) -> Result<EngineLogPage, EngineError> {
        if !(1..=512).contains(&limit) {
            return Err(EngineError::InvalidConfig(
                "log page limit must be 1..512".into(),
            ));
        }
        let page: EngineLogPage = self
            .observation(
                EngineObservationDomain::Logs,
                &["paws", "observations", "logs"],
                &[("after", after.to_string()), ("limit", limit.to_string())],
            )
            .await?;
        if page.entries.len() > usize::from(limit)
            || page.next_cursor < after
            || page.next_cursor > page.high_watermark
            || page
                .entries
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
            || page
                .entries
                .iter()
                .any(|entry| entry.sequence <= after || entry.sequence > page.next_cursor)
        {
            return Err(self.error("runtime logs", "invalid journal cursor ordering"));
        }
        Ok(page)
    }
    async fn connection_events(
        &self,
        after: u64,
        limit: u16,
    ) -> Result<EngineConnectionPage, EngineError> {
        if !(1..=512).contains(&limit) {
            return Err(EngineError::InvalidConfig(
                "connection page limit must be 1..512".into(),
            ));
        }
        let page: EngineConnectionPage = self
            .observation(
                EngineObservationDomain::ConnectionEvents,
                &["paws", "observations", "connection-events"],
                &[("after", after.to_string()), ("limit", limit.to_string())],
            )
            .await?;
        if page.entries.len() > usize::from(limit)
            || page.next_cursor < after
            || page.next_cursor > page.high_watermark
            || page
                .entries
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
            || page
                .entries
                .iter()
                .any(|entry| entry.sequence <= after || entry.sequence > page.next_cursor)
        {
            return Err(self.error("connection events", "invalid journal cursor ordering"));
        }
        Ok(page)
    }
    fn capabilities(&self) -> EngineCapabilities {
        Self::descriptor(self.engine)
    }
    async fn select_proxy(&self, group: &str, proxy: Option<&str>) -> Result<(), EngineError> {
        self.capabilities()
            .require(EngineCapability::ProxySelection)?;
        if proxy.is_none() {
            self.capabilities().require(EngineCapability::ProxyUnfix)?;
        }
        let request = match proxy {
            Some(proxy) => self
                .request(reqwest::Method::PUT, &["proxies", group])?
                .json(&serde_json::json!({"name": proxy})),
            None => self.request(reqwest::Method::DELETE, &["proxies", group])?,
        };
        self.done(request, "proxy selection").await
    }
    async fn proxy_selection(
        &self,
        group: &str,
    ) -> Result<paws_engine::EngineProxySelection, EngineError> {
        self.capabilities()
            .require(EngineCapability::ProxySelection)?;
        let response = self
            .request(reqwest::Method::GET, &["proxies", group])?
            .send()
            .await
            .map_err(|_| self.error("proxy selection", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "proxy selection",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("proxy selection", "invalid controller response"))?;
        let selected = value
            .get("now")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| self.error("proxy selection", "response missing current selection"))?;
        Ok(paws_engine::EngineProxySelection {
            selected: (!selected.is_empty()).then(|| selected.into()),
            fixed: value
                .get("fixed")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
    async fn proxy_groups(&self) -> Result<Vec<paws_model::ProxyGroup>, EngineError> {
        self.capabilities()
            .require(EngineCapability::ProxySelection)?;
        let response = self
            .request(reqwest::Method::GET, &["proxies"])?
            .send()
            .await
            .map_err(|_| self.error("proxy groups", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "proxy groups",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("proxy groups", "invalid controller response"))?;
        let proxies = value
            .get("proxies")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| self.error("proxy groups", "response missing proxies"))?;
        let mut groups = Vec::new();
        for (name, value) in proxies {
            let Some(members) = value.get("all") else {
                continue;
            };
            let members = members
                .as_array()
                .ok_or_else(|| self.error("proxy groups", "invalid group members"))?;
            let selected = value
                .get("now")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| self.error("proxy groups", "group missing current selection"))?;
            let mut items = Vec::new();
            for member in members {
                let name = member
                    .as_str()
                    .ok_or_else(|| self.error("proxy groups", "invalid member name"))?;
                let proxy = proxies.get(name);
                let delay = proxy
                    .and_then(|proxy| proxy.get("history"))
                    .and_then(serde_json::Value::as_array)
                    .and_then(|history| history.last())
                    .and_then(|sample| sample.get("delay"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|delay| u32::try_from(delay).ok())
                    .filter(|delay| *delay > 0);
                items.push(paws_model::ProxyItem {
                    name: name.into(),
                    proxy_type: proxy
                        .and_then(|proxy| proxy.get("type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Unknown")
                        .into(),
                    delay_ms: delay,
                    selected: name == selected,
                });
            }
            groups.push(paws_model::ProxyGroup {
                name: name.clone(),
                group_type: value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| self.error("proxy groups", "group missing type"))?
                    .into(),
                selected: (!selected.is_empty()).then(|| selected.into()),
                fixed: value
                    .get("fixed")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                proxies: items,
            });
        }
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(groups)
    }
    async fn proxy_echo(
        &self,
        proxy: &str,
        url: &str,
        payload: &str,
        timeout: Duration,
    ) -> Result<String, EngineError> {
        self.capabilities().require(EngineCapability::ProxyEcho)?;
        let path = if self.engine == VpnEngine::Meow {
            vec!["proxies", proxy, "echo"]
        } else {
            vec!["paws", "proxies", proxy, "echo"]
        };
        let response=self.request(reqwest::Method::POST,&path)?.json(&serde_json::json!({"url":url,"payload":payload,"timeout":timeout.as_millis().min(30000) as u64})).timeout(timeout+Duration::from_secs(2)).send().await.map_err(|_|self.error("echo","controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "echo",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("echo", "invalid controller response"))?;
        value
            .get("payload")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| self.error("echo", "response missing payload"))
    }
    async fn connections(&self) -> Result<Vec<ConnectionSummary>, EngineError> {
        self.capabilities().require(EngineCapability::Connections)?;
        let response = self
            .request(reqwest::Method::GET, &["connections"])?
            .send()
            .await
            .map_err(|_| self.error("connections", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "connections",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: Connections = response
            .json()
            .await
            .map_err(|_| self.error("connections", "invalid controller response"))?;
        Ok(value.connections.into_iter().map(Into::into).collect())
    }
    async fn close_connections(&self, id: Option<&str>) -> Result<(), EngineError> {
        self.capabilities().require(EngineCapability::Connections)?;
        let mut path = vec!["connections"];
        if let Some(id) = id {
            path.push(id);
        }
        self.done(
            self.request(reqwest::Method::DELETE, &path)?,
            "close connections",
        )
        .await
    }
    async fn flush_dns(&self, fake_ip: bool) -> Result<(), EngineError> {
        self.capabilities().require(if fake_ip {
            EngineCapability::FakeIpCacheFlush
        } else {
            EngineCapability::DnsCacheFlush
        })?;
        self.done(
            self.request(
                reqwest::Method::POST,
                &["cache", if fake_ip { "fakeip" } else { "dns" }, "flush"],
            )?,
            "flush cache",
        )
        .await
    }
    async fn proxy_delay(
        &self,
        proxy: &str,
        url: &str,
        timeout: Duration,
    ) -> Result<u16, EngineError> {
        self.capabilities().require(EngineCapability::ProxyDelay)?;
        let response = self
            .request(reqwest::Method::GET, &["proxies", proxy, "delay"])?
            .query(&[
                ("url", url.to_owned()),
                ("timeout", timeout.as_millis().to_string()),
            ])
            .timeout(timeout + Duration::from_secs(2))
            .send()
            .await
            .map_err(|_| self.error("proxy delay", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "proxy delay",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("proxy delay", "invalid controller response"))?;
        value
            .get("delay")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| self.error("proxy delay", "controller response missing valid delay"))
    }
    async fn provider_members(
        &self,
        name: &str,
    ) -> Result<Vec<paws_model::ProviderProxySummary>, EngineError> {
        self.capabilities().require(EngineCapability::Providers)?;
        let response = self
            .request(reqwest::Method::GET, &["providers", "proxies", name])?
            .send()
            .await
            .map_err(|_| self.error("provider members", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "provider members",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("provider members", "invalid controller response"))?;
        let proxies = value
            .get("proxies")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| self.error("provider members", "response missing proxies"))?;
        proxies
            .iter()
            .map(|proxy| {
                let name = proxy
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| self.error("provider members", "proxy name missing"))?;
                let delay = proxy
                    .get("history")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|history| history.last())
                    .and_then(|sample| sample.get("delay"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|delay| u32::try_from(delay).ok())
                    .filter(|delay| *delay > 0);
                Ok(paws_model::ProviderProxySummary {
                    name: name.into(),
                    proxy_type: proxy
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Unknown")
                        .into(),
                    alive: proxy.get("alive").and_then(serde_json::Value::as_bool),
                    delay_ms: delay,
                })
            })
            .collect()
    }
    async fn refresh_provider(&self, kind: &str, name: &str) -> Result<(), EngineError> {
        self.capabilities().require(EngineCapability::Providers)?;
        let kind = match kind {
            "proxy" | "proxies" => "proxies",
            "rule" | "rules" => "rules",
            _ => return Err(EngineError::InvalidConfig("unknown provider kind".into())),
        };
        self.done(
            self.request(reqwest::Method::PUT, &["providers", kind, name])?,
            "refresh provider",
        )
        .await
    }
    async fn provider_health(
        &self,
        name: &str,
        proxy: Option<&str>,
        url: Option<&str>,
        timeout: Duration,
        expected_status: Option<&str>,
    ) -> Result<Option<u16>, EngineError> {
        self.capabilities().require(EngineCapability::Providers)?;
        let mut path = vec!["providers", "proxies", name];
        if let Some(proxy) = proxy {
            path.push(proxy);
        }
        path.push("healthcheck");
        let mut request = self
            .request(reqwest::Method::GET, &path)?
            .query(&[("timeout", timeout.as_millis().to_string())]);
        if let Some(url) = url {
            request = request.query(&[("url", url)]);
        }
        if let Some(expected) = expected_status {
            request = request.query(&[("expected", expected)]);
        }
        let response = request
            .timeout(timeout + Duration::from_secs(2))
            .send()
            .await
            .map_err(|_| self.error("provider health", "controller request failed"))?;
        if !response.status().is_success() {
            return Err(self.error(
                "provider health",
                format!("controller HTTP {}", response.status().as_u16()),
            ));
        }
        if proxy.is_none() {
            return Ok(None);
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| self.error("provider health", "invalid controller response"))?;
        value
            .get("delay")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| self.error("provider health", "controller response missing valid delay"))
    }
}
