//! Go engine adapters. The host UI never loads or invokes native engines.
use async_trait::async_trait;
use futures::future::{BoxFuture, FutureExt, Shared};
use paws_engine::{
    EngineCapabilities, EngineCapability, EngineConfig, EngineError, EngineSession, EngineStart,
    EngineTraffic,
};
use paws_model::VpnEngine;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub mod config;
mod controller;
pub use controller::ControllerClient;
mod mihomo;
#[cfg(all(test, unix, not(target_env = "ohos")))]
mod native_provider_tests;
#[cfg(any(target_env = "ohos", test))]
mod provider_bridge;
mod sing_box;

pub fn register_process_resolver(resolver: Option<Arc<dyn paws_engine::ProcessResolver>>) {
    mihomo::register_process_resolver(resolver);
}

#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to boxed futures"
)]
#[async_trait]
trait GoBridge: Send + Sync {
    async fn start(&self, request: &EngineStart) -> Result<(), EngineError>;
    async fn confirm_services(
        &self,
        controller: &ControllerClient,
    ) -> Result<paws_engine::EngineServices, EngineError>;
    fn stop(&self) -> Result<(), EngineError>;
    async fn reload(&self, request: &EngineStart) -> Result<(), EngineError>;
    fn traffic(&self) -> Result<EngineTraffic, EngineError>;
    fn version(&self) -> Result<String, EngineError>;
    fn runtime_log(&self) -> Result<String, EngineError>;
    fn trim_memory(&self) -> Result<(), EngineError>;
}

struct NativeBridge(VpnEngine);
fn bridge_error(engine: VpnEngine, operation: &'static str, message: String) -> EngineError {
    EngineError::Backend {
        engine,
        operation,
        message,
    }
}
#[async_trait]
impl GoBridge for NativeBridge {
    async fn start(&self, request: &EngineStart) -> Result<(), EngineError> {
        match &request.config {
            EngineConfig::SingBox {
                config_json,
                work_dir,
            } => sing_box::start(
                request.tun_fd,
                request.underlying_network,
                config_json,
                work_dir,
            )
            .map_err(|error| bridge_error(self.0, "start", error)),
            EngineConfig::Mihomo {
                init_params,
                setup_params,
                ..
            } => mihomo::start(request, init_params, setup_params)
                .await
                .map_err(|error| bridge_error(self.0, "start", error)),
            _ => Err(EngineError::InvalidConfig(
                "Go adapter received a native Rust configuration".into(),
            )),
        }
    }
    async fn confirm_services(
        &self,
        controller: &ControllerClient,
    ) -> Result<paws_engine::EngineServices, EngineError> {
        controller.applied_services().await
    }
    fn stop(&self) -> Result<(), EngineError> {
        match self.0 {
            VpnEngine::SingBox => sing_box::stop(),
            VpnEngine::Mihomo => mihomo::stop(),
            _ => unreachable!(),
        }
        .map_err(|error| bridge_error(self.0, "stop", error))
    }
    async fn reload(&self, request: &EngineStart) -> Result<(), EngineError> {
        match &request.config {
            EngineConfig::Mihomo {
                init_params,
                setup_params,
                ..
            } => mihomo::reload(init_params, setup_params, request.underlying_network)
                .await
                .map_err(|error| bridge_error(self.0, "reload", error)),
            EngineConfig::SingBox { .. } => {
                self.stop()?;
                self.start(request).await
            }
            _ => Err(EngineError::InvalidConfig(
                "Go adapter received a native Rust configuration".into(),
            )),
        }
    }
    fn traffic(&self) -> Result<EngineTraffic, EngineError> {
        match self.0 {
            VpnEngine::SingBox => sing_box::traffic(),
            VpnEngine::Mihomo => mihomo::traffic(),
            _ => unreachable!(),
        }
        .map_err(|error| bridge_error(self.0, "traffic", error))
    }
    fn version(&self) -> Result<String, EngineError> {
        match self.0 {
            VpnEngine::SingBox => sing_box::version(),
            VpnEngine::Mihomo => mihomo::version(),
            _ => unreachable!(),
        }
        .map_err(|error| bridge_error(self.0, "version", error))
    }
    fn runtime_log(&self) -> Result<String, EngineError> {
        match self.0 {
            VpnEngine::SingBox => {
                sing_box::runtime_log().map_err(|error| bridge_error(self.0, "runtime log", error))
            }
            VpnEngine::Mihomo => {
                mihomo::runtime_log().map_err(|error| bridge_error(self.0, "runtime log", error))
            }
            _ => unreachable!(),
        }
    }
    fn trim_memory(&self) -> Result<(), EngineError> {
        match self.0 {
            VpnEngine::SingBox => {
                sing_box::trim_memory().map_err(|error| bridge_error(self.0, "trim memory", error))
            }
            VpnEngine::Mihomo => {
                mihomo::trim_memory().map_err(|error| bridge_error(self.0, "trim memory", error))
            }
            _ => Err(EngineError::Unsupported {
                engine: self.0,
                capability: EngineCapability::TrimMemory,
            }),
        }
    }
}

type PendingOperation = Shared<BoxFuture<'static, Result<(), Arc<EngineError>>>>;
struct Running {
    request: EngineStart,
    committed: bool,
    pending: Option<PendingOperation>,
    controller: ControllerOwner,
    services: paws_engine::EngineServices,
}
#[derive(Clone)]
struct ControllerOwner {
    endpoint: paws_engine::EngineControllerEndpoint,
    client: Arc<ControllerClient>,
}
/// One serialized, ownership-aware session. Failed startup/reload always crosses
/// a teardown barrier before another descriptor can be attached.
pub struct GoEngineSession {
    engine: VpnEngine,
    bridge: Arc<dyn GoBridge>,
    operation: tokio::sync::Mutex<()>,
    state: Mutex<Option<Running>>,
    generation: AtomicU64,
    expected_controller: Option<paws_engine::EngineControllerEndpoint>,
    settle_timeout: std::time::Duration,
}
impl GoEngineSession {
    pub fn descriptor(engine: VpnEngine) -> Result<EngineCapabilities, EngineError> {
        if !matches!(engine, VpnEngine::SingBox | VpnEngine::Mihomo) {
            return Err(EngineError::InvalidConfig(
                "Go adapter requires mihomo or sing-box".into(),
            ));
        }
        let mut operations = vec![
            EngineCapability::Tun,
            EngineCapability::Tcp,
            EngineCapability::Udp,
            EngineCapability::Dns,
            EngineCapability::Traffic,
            EngineCapability::Reload,
        ];
        operations.push(EngineCapability::TrimMemory);
        operations.push(EngineCapability::RuntimeLog);
        operations.extend(ControllerClient::descriptor(engine).operations);
        Ok(EngineCapabilities { engine, operations })
    }
    pub fn with_controller(
        engine: VpnEngine,
        endpoint: paws_engine::EngineControllerEndpoint,
    ) -> Result<Self, EngineError> {
        let mut session = Self::new(engine)?;
        ControllerClient::new(engine, endpoint.clone())?;
        session.expected_controller = Some(endpoint);
        Ok(session)
    }
    fn new(engine: VpnEngine) -> Result<Self, EngineError> {
        if !matches!(engine, VpnEngine::SingBox | VpnEngine::Mihomo) {
            return Err(EngineError::InvalidConfig(
                "Go adapter requires mihomo or sing-box".into(),
            ));
        }
        Ok(Self::with_bridge(engine, Arc::new(NativeBridge(engine))))
    }
    fn with_bridge(engine: VpnEngine, bridge: Arc<dyn GoBridge>) -> Self {
        Self {
            engine,
            bridge,
            operation: tokio::sync::Mutex::new(()),
            state: Mutex::new(None),
            generation: AtomicU64::new(0),
            expected_controller: None,
            settle_timeout: std::time::Duration::from_secs(30),
        }
    }
    fn controller_for(&self, config: &EngineConfig) -> Result<ControllerOwner, EngineError> {
        let endpoint = match config {
            EngineConfig::SingBox { config_json, .. } => {
                config::controller_endpoint(self.engine, config_json)?
            }
            EngineConfig::Mihomo {
                controller_endpoint,
                ..
            } => controller_endpoint.clone(),
            EngineConfig::Meow => {
                return Err(EngineError::InvalidConfig(
                    "Go controller requires a Go engine".into(),
                ));
            }
        };
        Ok(ControllerOwner {
            client: Arc::new(ControllerClient::new(self.engine, endpoint.clone())?),
            endpoint,
        })
    }
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, Option<Running>>, EngineError> {
        self.state
            .lock()
            .map_err(|_| bridge_error(self.engine, "state", "session lock poisoned".into()))
    }
    fn cleanup_failure(&self, primary: EngineError) -> EngineError {
        match self.bridge.stop() {
            Ok(()) => {
                if let Ok(mut state) = self.state.lock() {
                    *state = None;
                }
                primary
            }
            Err(cleanup) => EngineError::Cleanup {
                primary: Box::new(primary),
                cleanup: Box::new(cleanup),
            },
        }
    }
    fn begin_operation(&self, request: EngineStart, reload: bool) -> PendingOperation {
        let bridge = self.bridge.clone();
        let engine = self.engine;
        // The task owns its FFI operation even if the NAPI caller cancels its
        // future. A stop must join it before touching process-global Go state.
        let task = tokio::spawn(async move {
            if reload {
                bridge.reload(&request).await
            } else {
                bridge.start(&request).await
            }
        });
        async move {
            task.await
                .map_err(|error| Arc::new(bridge_error(engine, "native task", error.to_string())))?
                .map_err(Arc::new)
        }
        .boxed()
        .shared()
    }
    async fn settle(
        &self,
        pending: PendingOperation,
    ) -> Result<Result<(), Arc<EngineError>>, EngineError> {
        tokio::time::timeout(self.settle_timeout, pending)
            .await
            .map_err(|_| {
                bridge_error(
                    self.engine,
                    "native operation",
                    "still in flight; session quarantined until teardown joins its completion"
                        .into(),
                )
            })
    }
}
#[async_trait]
impl EngineSession for GoEngineSession {
    fn engine(&self) -> VpnEngine {
        self.engine
    }
    fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
        self.state.lock().ok().and_then(|state| {
            state.as_ref().filter(|state| state.committed).map(|state| {
                paws_engine::EngineManagementBinding {
                    client: state.controller.client.clone(),
                    endpoint: state.controller.endpoint.clone(),
                }
            })
        })
    }
    fn management(&self) -> Option<Arc<dyn paws_engine::EngineManagement>> {
        self.state.lock().ok().and_then(|state| {
            state.as_ref().filter(|state| state.committed).map(|state| {
                state.controller.client.clone() as Arc<dyn paws_engine::EngineManagement>
            })
        })
    }
    fn controller_endpoint(&self) -> Option<paws_engine::EngineControllerEndpoint> {
        self.state.lock().ok().and_then(|state| {
            state
                .as_ref()
                .filter(|state| state.committed)
                .map(|state| state.controller.endpoint.clone())
        })
    }
    fn services(&self) -> paws_engine::EngineServices {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .as_ref()
                    .filter(|state| state.committed)
                    .map(|state| state.services.clone())
            })
            .unwrap_or_default()
    }
    fn capabilities(&self) -> EngineCapabilities {
        Self::descriptor(self.engine).unwrap_or(EngineCapabilities {
            engine: self.engine,
            operations: Vec::new(),
        })
    }
    async fn start(&self, request: EngineStart) -> Result<u64, EngineError> {
        let _operation = self.operation.lock().await;
        if request.config.engine() != self.engine {
            return Err(EngineError::InvalidConfig(
                "configuration belongs to a different engine".into(),
            ));
        }
        if request.tun_fd < 0 || request.underlying_network <= 0 {
            return Err(EngineError::InvalidConfig(
                "TUN descriptor and physical network must be valid".into(),
            ));
        }
        if self.lock_state()?.is_some() {
            return Err(EngineError::AlreadyRunning);
        }
        let controller = self.controller_for(&request.config)?;
        if self
            .expected_controller
            .as_ref()
            .is_some_and(|expected| expected != &controller.endpoint)
        {
            return Err(EngineError::InvalidConfig(
                "start controller does not match the prepared owner".into(),
            ));
        }
        let pending = self.begin_operation(request.clone(), false);
        *self.lock_state()? = Some(Running {
            request,
            committed: false,
            pending: Some(pending.clone()),
            controller: controller.clone(),
            services: Default::default(),
        });
        let result = self.settle(pending).await?;
        if let Some(state) = self.lock_state()?.as_mut() {
            state.pending = None;
        }
        if let Err(error) = result {
            return Err(self.cleanup_failure(bridge_error(
                self.engine,
                "start",
                error.to_string(),
            )));
        }
        let services = match self.bridge.confirm_services(&controller.client).await {
            Ok(services) => services,
            Err(error) => return Err(self.cleanup_failure(error)),
        };
        if let Some(state) = self.lock_state()?.as_mut() {
            state.services = services;
            state.committed = true;
        }
        Ok(self.generation.fetch_add(1, Ordering::AcqRel) + 1)
    }
    async fn stop(&self) -> Result<(), EngineError> {
        let _operation = self.operation.lock().await;
        if self.lock_state()?.is_none() {
            return Ok(());
        }
        let pending = self
            .lock_state()?
            .as_ref()
            .and_then(|state| state.pending.clone());
        if let Some(pending) = pending {
            // A timeout is not cancellation of quickSetup. Keep ownership and
            // reject new starts until the old callback has actually settled.
            let _native_result = self.settle(pending).await?;
            if let Some(state) = self.lock_state()?.as_mut() {
                state.pending = None;
            }
        }
        if let Some(state) = self.lock_state()?.as_mut() {
            state.committed = false;
        }
        self.bridge.stop()?;
        *self.lock_state()? = None;
        Ok(())
    }
    async fn reload(
        &self,
        config: EngineConfig,
        underlying_network: i32,
    ) -> Result<(), EngineError> {
        let _operation = self.operation.lock().await;
        if config.engine() != self.engine || underlying_network <= 0 {
            return Err(EngineError::InvalidConfig(
                "reload changed engine or has no physical network".into(),
            ));
        }
        let mut request = self
            .lock_state()?
            .as_ref()
            .filter(|state| state.committed)
            .ok_or(EngineError::NotRunning)?
            .request
            .clone();
        request.config = config;
        request.underlying_network = underlying_network;
        let controller = self.controller_for(&request.config)?;
        let pending = self.begin_operation(request.clone(), true);
        if let Some(state) = self.lock_state()?.as_mut() {
            state.committed = false;
            state.pending = Some(pending.clone());
        }
        let result = self.settle(pending).await?;
        if let Some(state) = self.lock_state()?.as_mut() {
            state.pending = None;
        }
        if let Err(error) = result {
            return Err(self.cleanup_failure(bridge_error(
                self.engine,
                "reload",
                error.to_string(),
            )));
        }
        let services = match self.bridge.confirm_services(&controller.client).await {
            Ok(services) => services,
            Err(error) => return Err(self.cleanup_failure(error)),
        };
        *self.lock_state()? = Some(Running {
            request,
            committed: true,
            pending: None,
            controller,
            services,
        });
        Ok(())
    }
    fn is_running(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.as_ref().is_some_and(|state| state.committed))
    }
    fn traffic(&self) -> Result<EngineTraffic, EngineError> {
        if !self.is_running() {
            return Err(EngineError::NotRunning);
        }
        self.bridge.traffic()
    }
    fn version(&self) -> Result<String, EngineError> {
        self.bridge.version()
    }
    fn runtime_log(&self) -> Result<String, EngineError> {
        self.capabilities().require(EngineCapability::RuntimeLog)?;
        self.bridge.runtime_log()
    }
    fn trim_memory(&self) -> Result<(), EngineError> {
        self.capabilities().require(EngineCapability::TrimMemory)?;
        self.bridge.trim_memory()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod controller_tests;
