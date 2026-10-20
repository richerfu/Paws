use super::*;
use meow_common::auth::{AuthConfig, Credentials};

/// Ephemeral, authenticated loopback endpoint owned by the current VPN
/// process. It travels only over the session's private ashmem channel.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppHttpProxyEndpoint {
    port: u16,
    token: String,
}

impl std::fmt::Debug for AppHttpProxyEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHttpProxyEndpoint")
            .field("port", &self.port)
            .field("token", &"[redacted]")
            .finish()
    }
}

impl AppHttpProxyEndpoint {
    pub(super) fn proxy_url(&self) -> String {
        format!("http://paws-app:{}@127.0.0.1:{}", self.token, self.port)
    }
}

pub(super) struct AppHttpProxyRuntime {
    endpoint: AppHttpProxyEndpoint,
    task: tokio::task::JoinHandle<()>,
}

impl AppHttpProxyRuntime {
    pub(super) async fn start(tunnel: Tunnel) -> Result<Self, PawsError> {
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
        let endpoint = AppHttpProxyEndpoint { port, token };
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

    pub(super) fn endpoint(&self) -> AppHttpProxyEndpoint {
        self.endpoint.clone()
    }
}

impl Drop for AppHttpProxyRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn private_proxy_supports_app_requests_with_public_services_disabled_and_requires_auth() {
        let root = std::env::temp_dir().join(format!("paws-app-http-test-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        let config = load_meow_config("rules:\n  - MATCH,DIRECT\n")
            .await
            .unwrap();
        let runtime = AppHttpProxyRuntime::start(tunnel_from_config(config, RuntimeMode::Direct))
            .await
            .unwrap();
        let endpoint = runtime.endpoint();
        let proxy_url = endpoint.proxy_url();
        let secret = endpoint.token.clone();
        {
            let mut state = core.lock_state().unwrap();
            state.platform_vpn_running = true;
            state.platform_app_http_proxy = Some(endpoint);
            assert!(!state.network_ports.mixed_enabled);
            assert!(!state.network_ports.controller_enabled);
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let url = format!(
            "http://{}/subscription?token=private-subscription-token",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.contains("clash.meta/0.22.0"));
            let body = "rules:\n  - MATCH,DIRECT\n";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let response = core
            .download_subscription(&url, "profile refresh")
            .await
            .unwrap();
        assert!(response.body.contains("MATCH,DIRECT"));
        server.await.unwrap();
        assert_eq!(
            core.external_http_route().unwrap().1,
            http_client::ExternalHttpRoute::CurrentProxy
        );

        let anonymous_url = format!(
            "http://127.0.0.1:{}",
            reqwest::Url::parse(&proxy_url).unwrap().port().unwrap()
        );
        let anonymous = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(anonymous_url).unwrap())
            .build()
            .unwrap();
        let denied = anonymous.get(&url).send().await.unwrap();
        assert_eq!(
            denied.status(),
            reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED
        );

        core.set_log_recording_enabled(true).unwrap();
        drop(runtime);
        let error = core
            .download_subscription(&url, "profile refresh")
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-subscription-token"), "{error}");
        assert!(!error.contains(&secret), "{error}");
        assert!(error.contains("connect"), "{error}");
        assert!(core.snapshot().unwrap().logs.iter().any(
            |entry| entry.level == "error" && entry.message.contains("profile refresh failed")
        ));
        drop(core);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_engines_keep_app_http_available_with_public_services_disabled() {
        let root =
            std::env::temp_dir().join(format!("paws-external-http-test-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        for engine in [VpnEngine::Mihomo, VpnEngine::SingBox] {
            {
                let mut state = core.lock_state().unwrap();
                state.platform_vpn_running = true;
                state.platform_vpn_engine = Some(engine);
                assert!(!state.network_ports.mixed_enabled);
                assert!(!state.network_ports.controller_enabled);
                // The UI process must learn the actual selected engine over IPC.
                let frame = serde_json::to_string(&platform_vpn_state(&state)).unwrap();
                let remote: PlatformVpnState = serde_json::from_str(&frame).unwrap();
                assert_eq!(remote.engine, Some(engine));
            }
            let (url, route) = core.external_http_route().unwrap();
            assert_eq!(
                url,
                Some(format!(
                    "http://127.0.0.1:{}",
                    engine_policy::benchmark_port(engine)
                ))
            );
            assert_eq!(route, http_client::ExternalHttpRoute::CurrentProxy);
        }
        core.lock_state().unwrap().platform_vpn_running = false;
        assert_eq!(
            core.external_http_route().unwrap(),
            (None, http_client::ExternalHttpRoute::Direct)
        );
        drop(core);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn endpoint_credentials_stay_out_of_debug_diagnostics() {
        let endpoint = AppHttpProxyEndpoint {
            port: 10240,
            token: "private-proxy-token".to_owned(),
        };
        let state = PlatformVpnState {
            app_http_proxy: Some(endpoint.clone()),
            ..PlatformVpnState::default()
        };
        assert!(!format!("{state:?}").contains("private-proxy-token"));
        let restored: PlatformVpnState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.app_http_proxy, Some(endpoint));
    }
}
