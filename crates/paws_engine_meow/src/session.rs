//! Native meow lifecycle adapter. Application transactions live in the parent.
use async_trait::async_trait;
use paws_engine::{
    EngineCapabilities, EngineCapability, EngineConfig, EngineError, EngineServicePlan,
    EngineServices, EngineSession, EngineStart, EngineTraffic, EngineTunnel,
};
use paws_model::VpnEngine;
use paws_vpn::TunSession;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct MeowRuntimeConfig {
    pub tunnel: meow_tunnel::Tunnel,
    pub sniffer: meow_common::sniffer::SnifferConfig,
    pub auth: Arc<meow_common::AuthConfig>,
    pub proxy_providers:
        std::collections::HashMap<String, Arc<meow_config::proxy_provider::ProxyProvider>>,
    pub rule_providers:
        std::collections::HashMap<String, Arc<meow_config::rule_provider::RuleProvider>>,
}
pub struct MeowEngineSession {
    vpn: TunSession,
    tunnel: Arc<dyn EngineTunnel>,
    native_tunnel: meow_tunnel::Tunnel,
    auth: Arc<meow_common::AuthConfig>,
    management: Arc<crate::management::MeowManagement>,
    services_plan: EngineServicePlan,
    services: Mutex<Option<crate::services::MeowServices>>,
    sample: Mutex<Option<(Instant, u64, u64)>>,
    started: std::sync::atomic::AtomicBool,
}
impl MeowEngineSession {
    pub fn new(
        vpn: TunSession,
        runtime: MeowRuntimeConfig,
        services_plan: EngineServicePlan,
    ) -> Self {
        let management = Arc::new(crate::management::MeowManagement::new(
            runtime.tunnel.clone(),
            vpn.clone(),
            runtime.proxy_providers,
            runtime.rule_providers,
        ));
        Self {
            vpn,
            management,
            tunnel: Arc::new(crate::MeowTunnel::new(
                runtime.tunnel.clone(),
                runtime.sniffer,
            )),
            native_tunnel: runtime.tunnel,
            auth: runtime.auth,
            services_plan,
            services: Mutex::new(None),
            sample: Mutex::new(None),
            started: std::sync::atomic::AtomicBool::new(false),
        }
    }
    async fn shutdown_services(&self) {
        let services = self.services.lock().ok().and_then(|mut slot| slot.take());
        if let Some(services) = services {
            services.stop().await;
        }
    }
    fn reset_sample(&self) -> Result<(), EngineError> {
        *self.sample.lock().map_err(|_| EngineError::Backend {
            engine: VpnEngine::Meow,
            operation: "traffic",
            message: "sampler lock poisoned".into(),
        })? = Some((Instant::now(), 0, 0));
        Ok(())
    }
    pub fn descriptor() -> EngineCapabilities {
        EngineCapabilities {
            engine: VpnEngine::Meow,
            operations: vec![
                EngineCapability::Tun,
                EngineCapability::Tcp,
                EngineCapability::Udp,
                EngineCapability::Dns,
                EngineCapability::Traffic,
                EngineCapability::RuntimeLog,
                EngineCapability::TrimMemory,
                EngineCapability::Reload,
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
}
#[async_trait]
impl EngineSession for MeowEngineSession {
    fn runtime_log_sink(&self) -> Option<Arc<paws_engine::EngineLogBuffer>> {
        Some(self.management.logs.clone())
    }
    fn runtime_log(&self) -> Result<String, EngineError> {
        Ok(self
            .management
            .logs
            .page(0, 512)?
            .entries
            .into_iter()
            .map(|entry| format!("{} {} {}", entry.timestamp, entry.level, entry.message))
            .collect::<Vec<_>>()
            .join("\n"))
    }
    fn trim_memory(&self) -> Result<(), EngineError> {
        if !self.is_running() {
            return Err(EngineError::NotRunning);
        }
        // Dead provider slots are weak derived caches. Pruning them and the
        // ordinary DNS response cache never resets active fake-IP mappings.
        for provider in self.management.proxy_providers.values() {
            provider.prune_dead_derived();
        }
        self.vpn
            .flush_dns_cache()
            .map_err(|error| EngineError::Backend {
                engine: VpnEngine::Meow,
                operation: "trim DNS cache",
                message: error.to_string(),
            })?;
        self.management.logs.trim()
    }
    fn engine(&self) -> VpnEngine {
        VpnEngine::Meow
    }
    fn capabilities(&self) -> EngineCapabilities {
        Self::descriptor()
    }
    fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
        let services = self.services.lock().ok()?;
        let endpoint = services.as_ref()?.snapshot().controller_endpoint?;
        Some(paws_engine::EngineManagementBinding {
            client: self.management.clone(),
            endpoint,
        })
    }
    fn management(&self) -> Option<Arc<dyn paws_engine::EngineManagement>> {
        Some(self.management.clone())
    }
    fn controller_endpoint(&self) -> Option<paws_engine::EngineControllerEndpoint> {
        self.services().controller_endpoint
    }
    fn services(&self) -> EngineServices {
        self.services
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|services| services.snapshot()))
            .unwrap_or_default()
    }
    fn stop_services(&self) -> Result<(), EngineError> {
        self.services
            .lock()
            .map_err(|_| EngineError::InvalidConfig("services lock poisoned".into()))?
            .take();
        Ok(())
    }
    fn stop_public_listener(&self) -> Result<(), EngineError> {
        if let Some(services) = self
            .services
            .lock()
            .map_err(|_| EngineError::InvalidConfig("services lock poisoned".into()))?
            .as_mut()
        {
            services.stop_public();
        }
        Ok(())
    }
    async fn start(&self, request: EngineStart) -> Result<u64, EngineError> {
        if !matches!(request.config, EngineConfig::Meow) {
            return Err(EngineError::InvalidConfig(
                "meow session received another engine's configuration".into(),
            ));
        }
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(EngineError::AlreadyRunning);
        }
        self.management.activate().await?;
        let generation = match self
            .vpn
            .start(request.tun_fd, request.options, self.tunnel.clone())
            .await
        {
            Ok(generation) => generation,
            Err(error) => {
                self.management.revoke().await;
                return Err(EngineError::Backend {
                    engine: self.engine(),
                    operation: "start",
                    message: error.to_string(),
                });
            }
        };
        self.reset_sample()?;
        match crate::services::MeowServices::start(
            self.native_tunnel.clone(),
            &self.services_plan,
            self.management.clone(),
            self.auth.clone(),
        )
        .await
        {
            Ok(services) => {
                *self
                    .services
                    .lock()
                    .map_err(|_| EngineError::InvalidConfig("services lock poisoned".into()))? =
                    Some(services)
            }
            Err(error) => {
                self.management.revoke().await;
                self.vpn
                    .stop()
                    .await
                    .map_err(|cleanup| EngineError::Cleanup {
                        primary: Box::new(EngineError::InvalidConfig(error.to_string())),
                        cleanup: Box::new(EngineError::InvalidConfig(cleanup.to_string())),
                    })?;
                return Err(error);
            }
        }
        Ok(generation)
    }
    async fn stop(&self) -> Result<(), EngineError> {
        self.management.revoke().await;
        self.shutdown_services().await;
        self.vpn.stop().await.map_err(|error| EngineError::Backend {
            engine: self.engine(),
            operation: "stop",
            message: error.to_string(),
        })
    }
    async fn reload(&self, config: EngineConfig, _: i32) -> Result<(), EngineError> {
        if !matches!(config, EngineConfig::Meow) {
            return Err(EngineError::InvalidConfig(
                "meow reload received another engine's configuration".into(),
            ));
        }
        let _management_barrier = self.management.lease.write().await;
        let fd = self.vpn.fd().ok_or(EngineError::NotRunning)?;
        let options = self.vpn.options().ok_or(EngineError::NotRunning)?;
        self.vpn
            .start(fd, options, self.tunnel.clone())
            .await
            .map(|_| ())
            .map_err(|error| EngineError::Backend {
                engine: self.engine(),
                operation: "reload",
                message: error.to_string(),
            })?;
        self.reset_sample()
    }
    fn is_running(&self) -> bool {
        self.vpn.is_running()
    }
    fn traffic(&self) -> Result<EngineTraffic, EngineError> {
        if !self.is_running() {
            return Err(EngineError::NotRunning);
        }
        let stats = self.vpn.traffic_counters().ok_or(EngineError::NotRunning)?;
        let now = Instant::now();
        let mut sample = self.sample.lock().map_err(|_| EngineError::Backend {
            engine: self.engine(),
            operation: "traffic",
            message: "sampler lock poisoned".into(),
        })?;
        let (upload_speed, download_speed) =
            sample
                .as_ref()
                .map_or((0, 0), |(previous, upload, download)| {
                    let elapsed = now.duration_since(*previous).as_secs_f64();
                    if elapsed <= 0.0 {
                        (0, 0)
                    } else {
                        (
                            (stats.rx_bytes.saturating_sub(*upload) as f64 / elapsed) as u64,
                            (stats.tx_bytes.saturating_sub(*download) as f64 / elapsed) as u64,
                        )
                    }
                });
        *sample = Some((now, stats.rx_bytes, stats.tx_bytes));
        Ok(EngineTraffic {
            upload_total: stats.rx_bytes,
            download_total: stats.tx_bytes,
            upload_speed,
            download_speed,
        })
    }
    fn version(&self) -> Result<String, EngineError> {
        Ok("0.22.0".into())
    }
}
