//! Explicit group probes and probes routed by the current VPN configuration.
use crate::{http_client, CoreHandle};
use paws_model::PawsError;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

impl CoreHandle {
    /// No group means the current mode/rules decide the route. Never fall back
    /// to a host-direct request when the VPN or its app ingress is unavailable.
    pub async fn probe_connection(&self, group: Option<&str>, url: &str) -> Result<u32, PawsError> {
        let url = probe_url(url)?;
        let before = self.command_context()?;
        let session = before
            .vpn_session_id
            .filter(|_| before.vpn_running)
            .ok_or(PawsError::VpnNotRunning)?;
        let result = if let Some(group) = group {
            if group.trim().is_empty() {
                return Err(PawsError::Core("select a proxy group to probe".into()));
            }
            self.test_proxy_delay_via_controller(group, Some(url.as_str()), Some(8000))
                .await
                .map(u32::from)
        } else {
            let (proxy_url, route) = self.external_http_route()?;
            let proxy_url = proxy_url
                .filter(|_| route == http_client::ExternalHttpRoute::CurrentProxy)
                .ok_or(PawsError::VpnNotRunning)?;
            CurrentConnectionProbe::new(&proxy_url)?.run(&url).await
        };
        // A reconnect/config change invalidates both successes and failures.
        if !self.is_platform_vpn_session_current_at_revision(&session, before.config_revision)? {
            return Err(PawsError::Core(
                "VPN session or configuration changed during probe; run again".into(),
            ));
        }
        result
    }
}

fn probe_url(value: &str) -> Result<reqwest::Url, PawsError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| PawsError::Core("probe requires a valid HTTPS URL".into()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(PawsError::Core(
            "probe requires an HTTPS URL without credentials, query or fragment".into(),
        ));
    }
    Ok(url)
}

struct CurrentConnectionProbe<'a> {
    client: reqwest::Client,
    proxy_url: &'a str,
}

impl<'a> CurrentConnectionProbe<'a> {
    fn new(proxy_url: &'a str) -> Result<Self, PawsError> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| PawsError::Core("current VPN proxy address is invalid".into()))?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(PROBE_TIMEOUT)
            .user_agent(concat!("Paws/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| PawsError::Core("could not initialize connection probe".into()))?;
        Ok(Self { client, proxy_url })
    }

    async fn run(&self, url: &reqwest::Url) -> Result<u32, PawsError> {
        let started = Instant::now();
        // Measure a fresh request through the captured engine ingress. Do not
        // download arbitrary target bodies or follow a redirect to another URL.
        let response = self
            .client
            .get(url.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(|error| {
                PawsError::Core(format!(
                    "current connection HTTPS probe failed: {}",
                    http_client::request_error_message(error, Some(self.proxy_url))
                ))
            })?;
        if !response.status().is_success() {
            return Err(PawsError::Core(format!(
                "probe target returned HTTP {}; expected 2xx (redirects are not followed)",
                response.status().as_u16()
            )));
        }
        Ok(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn probe_urls_do_not_admit_credentials_or_redirect_inputs() {
        assert!(probe_url("https://www.gstatic.com/generate_204").is_ok());
        for value in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/?token=secret",
            "https://example.com/#fragment",
            "not a URL",
        ] {
            assert!(probe_url(value).is_err(), "{value}");
        }
    }

    #[tokio::test]
    async fn current_probe_uses_authenticated_ingress_and_rejects_non_success_without_redirect() {
        for status in ["204 No Content", "302 Found", "503 Service Unavailable"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_url = format!(
                "http://paws:probe-secret@{}",
                listener.local_addr().unwrap()
            );
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                let size = stream.read(&mut bytes).await.unwrap();
                let request = String::from_utf8_lossy(&bytes[..size]).to_lowercase();
                assert!(request.starts_with("get http://probe.invalid/check "));
                assert!(request.contains("proxy-authorization: basic "));
                stream.write_all(format!("HTTP/1.1 {status}\r\nLocation: http://must-not-follow.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            // Exercise the transport with plain HTTP so the fixture needs no
            // trust-store override. The public entry validates HTTPS separately.
            let result = CurrentConnectionProbe::new(&proxy_url)
                .unwrap()
                .run(&reqwest::Url::parse("http://probe.invalid/check").unwrap())
                .await;
            assert_eq!(result.is_ok(), status.starts_with("204"));
            if let Err(error) = result {
                assert!(error.to_string().contains(&status[..3]));
                assert!(!error.to_string().contains("probe-secret"));
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn stopped_vpn_cannot_fall_back_to_host_direct() {
        let root = std::env::temp_dir().join(format!("paws-probe-{}", crate::now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        assert!(matches!(
            core.probe_connection(None, "https://probe.invalid/check")
                .await,
            Err(PawsError::VpnNotRunning)
        ));
        drop(core);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn running_probe_requires_ingress_and_rejects_results_after_reconnect() {
        let root =
            std::env::temp_dir().join(format!("paws-probe-owner-{}", crate::now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        {
            let mut state = core.lock_state().unwrap();
            state.platform_vpn_running = true;
            state.platform_start_attempt_id = "probe-owner".into();
            state.platform_start_outcome = crate::PlatformStartOutcome::Connected;
            state.platform_vpn_engine = Some(paws_model::VpnEngine::Mihomo);
        }
        assert!(core
            .probe_connection(None, "https://probe.invalid/check")
            .await
            .unwrap_err()
            .to_string()
            .contains("ingress is not ready"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        core.lock_state().unwrap().platform_app_http_proxy =
            Some(crate::app_http::AppHttpProxyEndpoint {
                port: listener.local_addr().unwrap().port(),
                token: "probe-secret".into(),
            });
        let probe = core.probe_connection(None, "https://probe.invalid/check");
        let replace = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            assert!(request.starts_with("CONNECT probe.invalid:443 "));
            assert!(request
                .to_lowercase()
                .contains("proxy-authorization: basic "));
            core.lock_state().unwrap().platform_start_attempt_id = "replacement-owner".into();
            stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(probe, replace);
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("session or configuration changed"),
            "{message}"
        );
        assert!(!message.contains("probe-secret"));
        drop(core);
        let _ = std::fs::remove_dir_all(root);
    }
}
