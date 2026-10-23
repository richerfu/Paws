use meow_common::auth::{AuthConfig, Credentials};
use meow_listener::MixedListener;
use meow_tunnel::Tunnel;
use paws_engine::EngineHttpProxyEndpoint;
use paws_model::PawsError;
use std::collections::HashMap;
use std::sync::Arc;
pub struct AppHttpProxyRuntime {
    endpoint: EngineHttpProxyEndpoint,
    task: tokio::task::JoinHandle<()>,
}

impl AppHttpProxyRuntime {
    pub async fn start(tunnel: Tunnel) -> Result<Self, PawsError> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| PawsError::Core(format!("app HTTP proxy bind failed: {error}")))?;
        let addr = listener
            .local_addr()
            .map_err(|error| PawsError::Core(format!("app HTTP proxy address failed: {error}")))?;
        let port = addr.port();
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|error| {
            PawsError::Core(format!("app HTTP proxy token generation failed: {error}"))
        })?;
        use std::fmt::Write;
        let mut token = String::with_capacity(64);
        for byte in bytes {
            let _ = write!(token, "{byte:02x}");
        }
        let endpoint = EngineHttpProxyEndpoint { port, token };
        let credentials = Credentials::new(HashMap::from([(
            "paws-app".to_owned(),
            endpoint.token.clone(),
        )]));
        // Unlike normal user listeners, even loopback clients must authenticate.
        let auth = Arc::new(AuthConfig::new(Arc::new(credentials), Vec::new()));
        let mixed = MixedListener::new(tunnel, addr, "paws-app-http".to_owned())
            .with_auth(auth)
            .with_max_connections(16);
        let task = tokio::spawn(async move {
            if let Err(error) = mixed.run_on(listener).await {
                tracing::error!(target: "paws_core::app_http", "app HTTP proxy stopped: {error}");
            }
        });
        Ok(Self { endpoint, task })
    }

    pub fn endpoint(&self) -> EngineHttpProxyEndpoint {
        self.endpoint.clone()
    }

    pub async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for AppHttpProxyRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}
