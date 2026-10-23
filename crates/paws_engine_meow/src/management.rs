use async_trait::async_trait;
use meow_tunnel::Tunnel;
use paws_engine::{
    EngineCapabilities, EngineCapability, EngineDnsStatistics, EngineError, EngineGroupSummary,
    EngineManagement, EngineProviderKind, EngineProviderState, EngineRuntimeRule,
};
use paws_model::{ConnectionSummary, VpnEngine};
use paws_vpn::TunSession;
use std::time::Duration;
pub fn proxy_groups_snapshot(tunnel: &Tunnel) -> Vec<paws_model::ProxyGroup> {
    let route = tunnel.route_snapshot();
    let mut groups = route
        .proxies
        .values()
        .filter_map(|proxy| group_snapshot(&route, proxy.as_ref()))
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}
fn group_snapshot(
    route: &meow_tunnel::tunnel::RouteTable,
    proxy: &dyn meow_common::Proxy,
) -> Option<paws_model::ProxyGroup> {
    let members = proxy.members()?;
    let children = proxy.member_proxies().unwrap_or_default();
    let selected = proxy.current();
    Some(paws_model::ProxyGroup {
        name: proxy.name().into(),
        group_type: proxy.adapter_type().to_string(),
        selected: selected.clone(),
        fixed: proxy.selection().and_then(|selection| selection.fixed()),
        proxies: members
            .into_iter()
            .map(|name| {
                let member = route
                    .proxies
                    .get(name.as_str())
                    .or_else(|| children.iter().find(|proxy| proxy.name() == name));
                paws_model::ProxyItem {
                    selected: selected.as_deref() == Some(name.as_str()),
                    proxy_type: member
                        .map(|proxy| proxy.adapter_type().to_string())
                        .unwrap_or_else(|| "Unknown".into()),
                    delay_ms: member
                        .map(|proxy| proxy.last_delay())
                        .filter(|delay| *delay > 0)
                        .map(u32::from),
                    name,
                }
            })
            .collect(),
    })
}
pub fn connection_snapshot(tunnel: &Tunnel) -> Vec<ConnectionSummary> {
    let mut connections: Vec<_> = tunnel
        .statistics()
        .active_connections()
        .iter()
        .map(connection_summary)
        .collect();
    connections.sort_by(|a, b| a.host.cmp(&b.host).then_with(|| a.id.cmp(&b.id)));
    connections
}
fn connection_summary(connection: &meow_tunnel::statistics::ConnectionInfo) -> ConnectionSummary {
    let chains: Vec<String> = connection.chains.iter().map(ToString::to_string).collect();
    let proxy = if chains.is_empty() {
        "DIRECT".into()
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
        domain_source: if !connection.metadata.sniff_host.is_empty() {
            "TLS SNI / HTTP Host".into()
        } else if !connection.metadata.rule_host().is_empty() {
            "DNS 映射或配置（未记录具体解析器）".into()
        } else {
            "纯 IP / 未识别".into()
        },
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
        upload_bytes: u64::try_from(connection.counters.upload_bytes()).unwrap_or(0),
        download_bytes: u64::try_from(connection.counters.download_bytes()).unwrap_or(0),
    }
}

pub(crate) struct ConnectionRecorder {
    pub events: paws_engine::EngineConnectionBuffer,
    active: std::sync::atomic::AtomicBool,
}
impl ConnectionRecorder {
    pub fn new() -> Self {
        Self {
            events: paws_engine::EngineConnectionBuffer::new(VpnEngine::Meow, 1024, 1024 * 1024),
            active: std::sync::atomic::AtomicBool::new(false),
        }
    }
    fn record(&self, connection: &meow_tunnel::statistics::ConnectionInfo, closed: bool) {
        if self.active.load(std::sync::atomic::Ordering::Acquire) {
            if let Err(error) = self.events.record(connection_summary(connection), closed) {
                tracing::error!(%error, "meow connection journal failed");
            }
        }
    }
}
impl meow_tunnel::statistics::ConnectionObserver for ConnectionRecorder {
    fn opened(&self, connection: &meow_tunnel::statistics::ConnectionInfo) {
        self.record(connection, false);
    }
    fn closed(&self, connection: &meow_tunnel::statistics::ConnectionInfo) {
        self.record(connection, true);
    }
}

pub(crate) struct MeowManagement {
    pub retiring: std::sync::atomic::AtomicBool,
    pub cancellation: tokio::sync::Notify,
    pub clock: std::sync::Arc<crate::observation::ObservationClock>,
    pub provider_clock: std::sync::Arc<crate::observation::ProviderClock>,
    pub connections: std::sync::Arc<ConnectionRecorder>,
    pub logs: std::sync::Arc<paws_engine::EngineLogBuffer>,
    pub lease: tokio::sync::RwLock<bool>,
    pub tunnel: Tunnel,
    pub vpn: TunSession,
    pub proxy_providers: std::collections::HashMap<
        String,
        std::sync::Arc<meow_config::proxy_provider::ProxyProvider>,
    >,
    pub rule_providers:
        std::collections::HashMap<String, std::sync::Arc<meow_config::rule_provider::RuleProvider>>,
}
impl MeowManagement {
    pub fn new(
        tunnel: Tunnel,
        vpn: TunSession,
        proxy_providers: std::collections::HashMap<
            String,
            std::sync::Arc<meow_config::proxy_provider::ProxyProvider>,
        >,
        rule_providers: std::collections::HashMap<
            String,
            std::sync::Arc<meow_config::rule_provider::RuleProvider>,
        >,
    ) -> Self {
        let clock = std::sync::Arc::new(crate::observation::ObservationClock::new());
        Self {
            provider_clock: std::sync::Arc::new(crate::observation::ProviderClock(clock.clone())),
            clock,
            retiring: Default::default(),
            cancellation: Default::default(),
            connections: std::sync::Arc::new(ConnectionRecorder::new()),
            logs: std::sync::Arc::new(paws_engine::EngineLogBuffer::new(
                VpnEngine::Meow,
                512,
                256 * 1024,
            )),
            tunnel,
            vpn,
            proxy_providers,
            rule_providers,
            lease: tokio::sync::RwLock::new(false),
        }
    }
    pub async fn activate(&self) -> Result<(), EngineError> {
        if self.retiring.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::NotRunning);
        }
        for provider in self.proxy_providers.values() {
            let observer: std::sync::Arc<dyn meow_config::observation::ProviderObserver> =
                self.provider_clock.clone();
            provider
                .observe_runtime(std::sync::Arc::downgrade(&observer))
                .await
                .map_err(|error| self.error("bind provider", error))?;
        }
        for provider in self.rule_providers.values() {
            let observer: std::sync::Arc<dyn meow_config::observation::ProviderObserver> =
                self.provider_clock.clone();
            provider
                .observe_runtime(std::sync::Arc::downgrade(&observer))
                .await
                .map_err(|error| self.error("bind rule provider", error))?;
        }
        self.clock.activate();
        let route = self.tunnel.route_snapshot();
        for proxy in route.proxies.values() {
            self.clock.bind(proxy);
        }
        for provider in self.proxy_providers.values() {
            for proxy in provider.proxies() {
                self.clock.bind(&proxy);
            }
        }
        *self.lease.write().await = true;
        self.connections
            .active
            .store(true, std::sync::atomic::Ordering::Release);
        self.tunnel
            .statistics()
            .set_connection_observer(Some(self.connections.clone()));
        Ok(())
    }
    pub async fn revoke(&self) {
        // Signal first: a provider request already holds the management read
        // lease. Waiting for write before cancellation would deadlock teardown
        // behind a download and its full network timeout.
        self.retiring
            .store(true, std::sync::atomic::Ordering::Release);
        self.cancellation.notify_waiters();
        for provider in self.proxy_providers.values() {
            provider.cancel_runtime();
        }
        for provider in self.rule_providers.values() {
            provider.cancel_runtime();
        }
        for provider in self.proxy_providers.values() {
            provider.retire_runtime().await;
        }
        for provider in self.rule_providers.values() {
            provider.retire_runtime().await;
        }
        *self.lease.write().await = false;
        self.clock.revoke();
        self.connections
            .active
            .store(false, std::sync::atomic::Ordering::Release);
    }
    async fn enter(&self) -> Result<tokio::sync::RwLockReadGuard<'_, bool>, EngineError> {
        if self.retiring.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::NotRunning);
        }
        let guard = self.lease.read().await;
        if !*guard || self.retiring.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::NotRunning);
        }
        Ok(guard)
    }

    async fn interruptible<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        let cancelled = self.cancellation.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        if self.retiring.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::NotRunning);
        }
        tokio::select! {
            result = future => result,
            _ = cancelled => Err(EngineError::NotRunning),
        }
    }

    fn error(&self, operation: &'static str, error: impl std::fmt::Display) -> EngineError {
        EngineError::Backend {
            engine: VpnEngine::Meow,
            operation,
            message: error.to_string(),
        }
    }
}
#[async_trait]
impl EngineManagement for MeowManagement {
    async fn runtime_revisions(&self) -> Result<paws_engine::EngineRuntimeRevisions, EngineError> {
        let _lease = self.enter().await?;
        let mut revisions = self.clock.snapshot()?;
        revisions.connections = self.connections.events.revision()?;
        revisions.logs = self.logs.revision()?;
        revisions.dns = self
            .vpn
            .traffic_counters()
            .map_or(0, |counters| counters.dns_packets);
        Ok(revisions)
    }
    async fn proxy_group(&self, name: &str) -> Result<paws_model::ProxyGroup, EngineError> {
        let _lease = self.enter().await?;
        let route = self.tunnel.route_snapshot();
        route
            .proxies
            .get(name)
            .and_then(|proxy| group_snapshot(&route, proxy.as_ref()))
            .ok_or_else(|| self.error("proxy group", "group not found"))
    }
    async fn connection_events(
        &self,
        after: u64,
        limit: u16,
    ) -> Result<paws_engine::EngineConnectionPage, EngineError> {
        let _lease = self.enter().await?;
        self.connections.events.page(after, limit)
    }
    async fn connection_summary(
        &self,
    ) -> Result<paws_engine::EngineConnectionSummary, EngineError> {
        let _lease = self.enter().await?;
        let mut recent: Vec<meow_tunnel::statistics::ConnectionInfo> = Vec::with_capacity(6);
        let mut summary = paws_engine::EngineConnectionSummary {
            count: 0,
            upload_bytes: 0,
            download_bytes: 0,
            recent: Vec::new(),
            latest_rule_node: self.connections.events.latest_rule_node()?,
        };
        for connection in &self.tunnel.statistics().connections {
            let info = connection.value();
            summary.count += 1;
            summary.upload_bytes = summary
                .upload_bytes
                .saturating_add(u64::try_from(info.counters.upload_bytes()).unwrap_or(0));
            summary.download_bytes = summary
                .download_bytes
                .saturating_add(u64::try_from(info.counters.download_bytes()).unwrap_or(0));
            if recent.len() < 5 || recent.last().is_some_and(|last| info.start > last.start) {
                recent.push(info.clone());
                recent.sort_by(|a, b| b.start.cmp(&a.start).then_with(|| a.id.cmp(&b.id)));
                recent.truncate(5);
            }
        }
        summary.recent = recent.iter().map(connection_summary).collect();
        Ok(summary)
    }
    async fn memory(&self) -> Result<paws_engine::EngineMemory, EngineError> {
        let _lease = self.enter().await?;
        crate::memory::ProcessMemory.sample()
    }
    async fn dns_config(&self) -> Result<paws_engine::EngineDnsConfig, EngineError> {
        let _lease = self.enter().await?;
        let options = self.vpn.options().ok_or(EngineError::NotRunning)?;
        Ok(paws_engine::EngineDnsConfig {
            model: "meow-tun".into(),
            listen: options.dns_hijacking.then(|| "tun:53".into()),
            upstreams: options.dns_servers,
            fallbacks: options.dns_fallbacks,
            nameserver_policy: options.dns_nameserver_policy,
        })
    }
    async fn runtime_logs(
        &self,
        after: u64,
        limit: u16,
    ) -> Result<paws_engine::EngineLogPage, EngineError> {
        let _lease = self.enter().await?;
        self.logs.page(after, limit)
    }
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            engine: VpnEngine::Meow,
            operations: vec![
                EngineCapability::ProxySelection,
                EngineCapability::ProxyUnfix,
                EngineCapability::Connections,
                EngineCapability::DnsCacheFlush,
                EngineCapability::FakeIpCacheFlush,
                EngineCapability::ProxyDelay,
                EngineCapability::ProxyEcho,
                EngineCapability::Providers,
            ],
        }
    }
    async fn group_summaries(&self) -> Result<Vec<EngineGroupSummary>, EngineError> {
        let _lease = self.enter().await?;
        let route = self.tunnel.route_snapshot();
        let mut groups = route
            .proxies
            .values()
            .filter_map(|proxy| {
                // member_proxies clones Arc handles, not the member names or
                // their telemetry. The home query does not export a roster.
                let members = proxy.member_proxies()?;
                let selected = proxy.current();
                let selected_delay_ms = members
                    .iter()
                    .find(|member| Some(member.name()) == selected.as_deref())
                    .map(|member| member.last_delay())
                    .filter(|delay| *delay > 0)
                    .map(u32::from);
                Some(EngineGroupSummary {
                    name: proxy.name().into(),
                    group_type: proxy.adapter_type().to_string(),
                    selected,
                    fixed: proxy.selection().and_then(|selection| selection.fixed()),
                    member_count: members.len(),
                    selected_delay_ms,
                })
            })
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(groups)
    }
    async fn provider_state(
        &self,
        kind: EngineProviderKind,
        name: &str,
    ) -> Result<EngineProviderState, EngineError> {
        let _lease = self.enter().await?;
        match kind {
            EngineProviderKind::Proxy => {
                let provider = self
                    .proxy_providers
                    .get(name)
                    .ok_or_else(|| self.error("provider state", "proxy provider not found"))?;
                let members = provider
                    .proxies()
                    .into_iter()
                    .map(|proxy| {
                        let delay = proxy.last_delay();
                        paws_model::ProviderProxySummary {
                            name: proxy.name().into(),
                            proxy_type: proxy.adapter_type().to_string(),
                            alive: (!proxy.delay_history().is_empty()).then(|| proxy.alive()),
                            delay_ms: (!proxy.delay_history().is_empty())
                                .then_some(u32::from(delay)),
                        }
                    })
                    .collect();
                Ok(EngineProviderState {
                    kind,
                    name: name.into(),
                    members: Some(members),
                    rule_count: None,
                    updated_at: (provider.updated_at_secs() > 0)
                        .then(|| provider.updated_at_secs().to_string()),
                    vehicle: Some(provider.vehicle_type.into()),
                })
            }
            EngineProviderKind::Rule => {
                let provider = self
                    .rule_providers
                    .get(name)
                    .ok_or_else(|| self.error("provider state", "rule provider not found"))?;
                Ok(EngineProviderState {
                    kind,
                    name: name.into(),
                    members: None,
                    rule_count: Some(provider.rule_count()),
                    updated_at: (provider.updated_at_secs() > 0)
                        .then(|| provider.updated_at_secs().to_string()),
                    vehicle: Some(provider.provider_type.to_string()),
                })
            }
        }
    }
    async fn runtime_rules(&self) -> Result<Vec<EngineRuntimeRule>, EngineError> {
        let _lease = self.enter().await?;
        Ok(self
            .tunnel
            .route_snapshot()
            .rules
            .iter()
            .map(|rule| EngineRuntimeRule {
                rule_type: rule.rule_type().to_string(),
                payload: rule.payload().into(),
                target: rule.adapter().into(),
            })
            .collect())
    }
    async fn dns_statistics(&self) -> Result<EngineDnsStatistics, EngineError> {
        let _lease = self.enter().await?;
        let stats = self.vpn.stats().ok_or(EngineError::NotRunning)?;
        Ok(EngineDnsStatistics {
            handled_packets: stats.dns_packets,
            cache_hits: stats.dns_cache_hits,
            cache_misses: stats.dns_cache_misses,
            recent_queries: stats.recent_dns_queries,
        })
    }
    async fn select_proxy(&self, group: &str, proxy: Option<&str>) -> Result<(), EngineError> {
        let _lease = self.enter().await?;
        let route = self.tunnel.route_snapshot();
        let group = route
            .proxies
            .get(group)
            .ok_or_else(|| self.error("proxy selection", "group not found"))?;
        let selection = group
            .selection()
            .ok_or_else(|| self.error("proxy selection", "group is not selectable"))?;
        if let Some(proxy) = proxy {
            selection
                .set(proxy)
                .await
                .map_err(|error| self.error("proxy selection", error))
        } else if selection.can_unfix() {
            selection.force_set(None);
            Ok(())
        } else {
            Err(self.error("proxy selection", "group cannot be unfixed"))
        }
    }
    async fn proxy_selection(
        &self,
        group: &str,
    ) -> Result<paws_engine::EngineProxySelection, EngineError> {
        let _lease = self.enter().await?;
        let route = self.tunnel.route_snapshot();
        let proxy = route
            .proxies
            .get(group)
            .ok_or_else(|| self.error("proxy selection", "group not found"))?;
        Ok(paws_engine::EngineProxySelection {
            selected: proxy.current(),
            fixed: proxy.selection().and_then(|selection| selection.fixed()),
        })
    }
    async fn proxy_groups(&self) -> Result<Vec<paws_model::ProxyGroup>, EngineError> {
        let _lease = self.enter().await?;
        Ok(proxy_groups_snapshot(&self.tunnel))
    }
    async fn proxy_echo(
        &self,
        proxy: &str,
        url: &str,
        payload: &str,
        timeout: Duration,
    ) -> Result<String, EngineError> {
        let _lease = self.enter().await?;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let proxy = self
            .tunnel
            .proxy(proxy)
            .ok_or_else(|| self.error("echo", "proxy not found"))?;
        let url = url::Url::parse(url).map_err(|error| self.error("echo URL", error))?;
        let host = url
            .host_str()
            .ok_or_else(|| self.error("echo URL", "target has no host"))?;
        let metadata = meow_common::Metadata {
            network: meow_common::Network::Tcp,
            conn_type: meow_common::ConnType::Inner,
            dst_port: url.port_or_known_default().unwrap_or(443),
            host: host.into(),
            in_name: "paws-echo".into(),
            ..Default::default()
        };
        self.interruptible(async {
            tokio::time::timeout(timeout, async {
                let mut stream = proxy
                    .dial_tcp(&metadata)
                    .await
                    .map_err(|error| self.error("echo connect", error))?;
                stream.write_all(payload.as_bytes()).await?;
                let mut response = vec![0u8; payload.len()];
                stream.read_exact(&mut response).await?;
                if response != payload.as_bytes() {
                    return Err(self.error("echo", "payload mismatch"));
                }
                String::from_utf8(response).map_err(|error| self.error("echo", error))
            })
            .await
            .map_err(|_| self.error("echo", "timed out"))?
        })
        .await
    }
    async fn connections(&self) -> Result<Vec<ConnectionSummary>, EngineError> {
        let _lease = self.enter().await?;
        Ok(connection_snapshot(&self.tunnel))
    }
    async fn close_connections(&self, id: Option<&str>) -> Result<(), EngineError> {
        let _lease = self.enter().await?;
        if let Some(id) = id {
            let id = id
                .parse()
                .map_err(|error| self.error("connection id", error))?;
            self.tunnel.statistics().close_connection(id);
        } else {
            self.tunnel.statistics().close_all_connections();
        }
        Ok(())
    }
    async fn flush_dns(&self, fake_ip: bool) -> Result<(), EngineError> {
        let _lease = self.enter().await?;
        if fake_ip {
            self.tunnel
                .resolver()
                .flush_fake_ip()
                .map_err(|error| self.error("fake-IP cache flush", error))?;
        } else {
            self.vpn
                .flush_dns_cache()
                .map_err(|error| self.error("TUN DNS flush", error))?;
            self.tunnel.resolver().clear_cache();
        }
        Ok(())
    }
    async fn proxy_delay(
        &self,
        proxy: &str,
        url: &str,
        timeout: Duration,
    ) -> Result<u16, EngineError> {
        let _lease = self.enter().await?;
        let proxy = self
            .tunnel
            .proxy(proxy)
            .ok_or_else(|| self.error("proxy delay", "proxy not found"))?;
        self.interruptible(async {
            meow_proxy::health::probe_and_record(&proxy, url, None, timeout)
                .await
                .map_err(|error| self.error("proxy delay", format!("{error:?}")))
        })
        .await
    }
    async fn provider_members(
        &self,
        name: &str,
    ) -> Result<Vec<paws_model::ProviderProxySummary>, EngineError> {
        let _lease = self.enter().await?;
        let provider = self
            .proxy_providers
            .get(name)
            .ok_or_else(|| self.error("provider members", "provider not found"))?;
        Ok(provider
            .proxies()
            .into_iter()
            .map(|proxy| {
                let delay = proxy.last_delay();
                paws_model::ProviderProxySummary {
                    name: proxy.name().into(),
                    proxy_type: proxy.adapter_type().to_string(),
                    alive: (!proxy.delay_history().is_empty()).then(|| proxy.alive()),
                    delay_ms: (!proxy.delay_history().is_empty()).then_some(u32::from(delay)),
                }
            })
            .collect())
    }
    async fn refresh_provider(&self, kind: &str, name: &str) -> Result<(), EngineError> {
        let _lease = self.enter().await?;
        match kind {
            "proxy" | "proxies" => self
                .proxy_providers
                .get(name)
                .ok_or_else(|| self.error("provider refresh", "provider not found"))?
                .refresh()
                .await
                .map_err(|error| self.error("provider refresh", error)),
            "rule" | "rules" => self
                .rule_providers
                .get(name)
                .ok_or_else(|| self.error("provider refresh", "provider not found"))?
                .refresh()
                .await
                .map_err(|error| self.error("provider refresh", error)),
            _ => Err(self.error("provider refresh", "unknown provider kind")),
        }
    }
    async fn provider_health(
        &self,
        name: &str,
        proxy: Option<&str>,
        url: Option<&str>,
        timeout: Duration,
        expected_status: Option<&str>,
    ) -> Result<Option<u16>, EngineError> {
        let _lease = self.enter().await?;
        self.interruptible(async {
            let provider = self
                .proxy_providers
                .get(name)
                .ok_or_else(|| self.error("provider health", "provider not found"))?;
            let url = url
                .or_else(|| {
                    provider
                        .health_check
                        .as_ref()
                        .map(|config| config.url.as_str())
                })
                .unwrap_or("https://www.gstatic.com/generate_204");
            let expected = expected_status
                .or_else(|| {
                    provider
                        .health_check
                        .as_ref()
                        .map(|config| config.expected_status.as_str())
                })
                .filter(|status| !status.is_empty());
            if let Some(name) = proxy {
                let proxy = provider
                    .proxies()
                    .into_iter()
                    .find(|proxy| proxy.name() == name)
                    .ok_or_else(|| self.error("provider health", "provider proxy not found"))?;
                return meow_proxy::health::probe_and_record(&proxy, url, expected, timeout)
                    .await
                    .map(Some)
                    .map_err(|error| self.error("provider health", format!("{error:?}")));
            }
            use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
            let mut probes: FuturesUnordered<
                futures::future::BoxFuture<'_, Result<u16, meow_proxy::health::UrlTestError>>,
            > = FuturesUnordered::new();
            for proxy in provider.proxies() {
                probes.push(
                    async move {
                        meow_proxy::health::probe_and_record(&proxy, url, expected, timeout).await
                    }
                    .boxed(),
                );
                if probes.len() >= 8 {
                    let _ = probes.next().await;
                }
            }
            while probes.next().await.is_some() {}
            Ok(None)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn revocation_waits_for_in_flight_operations_and_denies_obsolete_handles() {
        let resolver = std::sync::Arc::new(meow_dns::Resolver::new(
            Vec::new(),
            Vec::new(),
            meow_common::DnsMode::Normal,
            meow_trie::DomainTrie::new(),
            true,
            false,
        ));
        let management = std::sync::Arc::new(MeowManagement::new(
            Tunnel::new(resolver),
            TunSession::default(),
            Default::default(),
            Default::default(),
        ));
        management.activate().await.unwrap();
        let operation = management.enter().await.unwrap();
        let owner = management.clone();
        let mut stop = tokio::spawn(async move { owner.revoke().await });
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut stop)
            .await
            .is_err());
        drop(operation);
        stop.await.unwrap();
        assert!(matches!(
            management.flush_dns(false).await,
            Err(EngineError::NotRunning)
        ));
        assert!(matches!(
            management.connections().await,
            Err(EngineError::NotRunning)
        ));
    }
}
