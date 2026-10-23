use crate::app_http::AppHttpProxyRuntime;
use meow_listener::MixedListener;
use meow_tunnel::Tunnel;
use paws_engine::{EngineError, EngineServicePlan, EngineServices};
use paws_model::VpnEngine;
use std::net::SocketAddr;
use tokio::task::JoinHandle;
#[path = "public_controller.rs"]
mod public_controller;
use public_controller::PublicController;

pub(crate) struct MeowServices {
    app: Option<AppHttpProxyRuntime>,
    public: Option<JoinHandle<()>>,
    benchmark: Option<JoinHandle<()>>,
    snapshot: EngineServices,
    controller: Option<crate::controller::PrivateController>,
    public_controller: Option<PublicController>,
}
impl MeowServices {
    pub async fn start(
        tunnel: Tunnel,
        plan: &EngineServicePlan,
        management: std::sync::Arc<crate::management::MeowManagement>,
        auth: std::sync::Arc<meow_common::AuthConfig>,
    ) -> Result<Self, EngineError> {
        plan.network.validate()?;
        let mut services = Self {
            app: None,
            public: None,
            benchmark: None,
            snapshot: Default::default(),
            controller: None,
            public_controller: None,
        };
        if !plan.enabled {
            return Ok(services);
        }
        if !plan.network_protected {
            return Err(EngineError::Backend {
                engine: VpnEngine::Meow,
                operation: "listeners",
                message: "VPN process egress is not protected".into(),
            });
        }
        let controller = crate::controller::PrivateController::start(management.clone()).await?;
        services.snapshot.controller_endpoint = Some(controller.endpoint.clone());
        services.controller = Some(controller);
        let app = match AppHttpProxyRuntime::start(tunnel.clone()).await {
            Ok(app) => app,
            Err(error) => {
                services.stop().await;
                return Err(EngineError::Backend {
                    engine: VpnEngine::Meow,
                    operation: "app HTTP proxy",
                    message: error.to_string(),
                });
            }
        };
        let benchmark_endpoint = paws_engine::EngineHttpProxyEndpoint {
            port: plan.benchmark_port,
            token: app.endpoint().token,
        };
        services.snapshot.benchmark_proxy = Some(benchmark_endpoint.clone());
        services.snapshot.app_http_proxy = Some(app.endpoint());
        services.app = Some(app);
        let benchmark_addr = SocketAddr::from(([127, 0, 0, 1], plan.benchmark_port));
        let public_addr = plan
            .network
            .ports
            .mixed_enabled
            .then_some(plan.network.ports.mixed_port)
            .map(|port| {
                SocketAddr::from((
                    if plan.network.access.allow_lan {
                        [0, 0, 0, 0]
                    } else {
                        [127, 0, 0, 1]
                    },
                    port,
                ))
            });
        if let Some(address) = public_addr {
            match tokio::net::TcpListener::bind(address).await {
                Ok(socket) => {
                    let listener = MixedListener::new(tunnel.clone(), address, "paws-mixed".into())
                        .with_auth(auth.clone())
                        .with_max_connections(64);
                    services.public = Some(tokio::spawn(async move {
                        if let Err(error) = listener.run_on(socket).await {
                            tracing::warn!(%error,"public mixed listener stopped");
                        }
                    }));
                    services.snapshot.mixed_ready = true;
                }
                Err(error) => {
                    let failure = EngineError::Backend {
                        engine: VpnEngine::Meow,
                        operation: "public mixed listener",
                        message: format!("could not bind {address}: {error}"),
                    };
                    services.stop().await;
                    return Err(failure);
                }
            }
        }
        match tokio::net::TcpListener::bind(benchmark_addr).await {
            Ok(socket) => {
                let listener =
                    MixedListener::new(tunnel, benchmark_addr, "paws-meow-benchmark".into())
                        .with_auth(std::sync::Arc::new(meow_common::auth::AuthConfig::new(
                            std::sync::Arc::new(meow_common::auth::Credentials::new(
                                std::collections::HashMap::from([(
                                    "paws-app".into(),
                                    benchmark_endpoint.token.clone(),
                                )]),
                            )),
                            Vec::new(),
                        )))
                        .with_max_connections(64);
                services.benchmark = Some(tokio::spawn(async move {
                    if let Err(error) = listener.run_on(socket).await {
                        tracing::warn!(%error,"benchmark listener stopped");
                    }
                }));
            }
            Err(error) => {
                let failure = EngineError::Backend {
                    engine: VpnEngine::Meow,
                    operation: "benchmark listener",
                    message: format!("could not bind {benchmark_addr}: {error}"),
                };
                services.stop().await;
                return Err(failure);
            }
        }
        if plan.network.ports.controller_enabled {
            match PublicController::start(management, plan.network.clone()).await {
                Ok(controller) => {
                    services.snapshot.public_controller_addr = Some(controller.address);
                    services.public_controller = Some(controller);
                }
                Err(error) => {
                    services.stop().await;
                    return Err(error);
                }
            }
        }
        Ok(services)
    }
    pub fn snapshot(&self) -> EngineServices {
        let mut snapshot = self.snapshot.clone();
        if !self
            .public_controller
            .as_ref()
            .is_some_and(PublicController::is_running)
        {
            snapshot.public_controller_addr = None;
        }
        snapshot
    }
    pub fn stop_public(&mut self) {
        if let Some(task) = self.public.take() {
            task.abort();
        }
        self.snapshot.mixed_ready = false;
        if let Some(controller) = &self.public_controller {
            controller.revoke();
        }
        self.snapshot.public_controller_addr = None;
    }
    async fn stop_listeners(&mut self) {
        if let Some(controller) = self.public_controller.take() {
            controller.stop().await;
        }
        for task in [self.public.take(), self.benchmark.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
            let _ = task.await;
        }
    }
    pub async fn stop(mut self) {
        self.stop_listeners().await;
        if let Some(app) = self.app.take() {
            app.stop().await;
        }
        if let Some(controller) = self.controller.take() {
            controller.stop().await;
        }
    }
}
impl Drop for MeowServices {
    fn drop(&mut self) {
        if let Some(task) = &self.public {
            task.abort();
        }
        if let Some(task) = &self.benchmark {
            task.abort();
        }
    }
}
