//! One bounded startup probe through the actual engine-owned ingress.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupCheck {
    Passed,
    Failed,
    NotTested,
    NotApplicable,
}

/// Evidence for one actual VPN session. A cached result from another session
/// or engine must never turn the current connection's indicators green.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupHealth {
    pub session: String,
    pub engine: VpnEngine,
    pub dns: StartupCheck,
    pub node_tcp: StartupCheck,
    pub https: StartupCheck,
    pub elapsed_ms: u64,
}

impl StartupHealth {
    pub(crate) fn read_current(
        root: &Path,
        session: Option<&str>,
        engine: Option<VpnEngine>,
    ) -> Option<Self> {
        let session = session?;
        let engine = engine?;
        let text = process_evidence::read_bounded(&root.join("engine-startup-health.json"), 8192)?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        Self::from_evidence(&value, session, engine)
    }

    fn from_evidence(value: &serde_json::Value, session: &str, engine: VpnEngine) -> Option<Self> {
        if value["session"].as_str()? != session
            || value["actualEngine"].as_str()? != engine.as_str()
        {
            return None;
        }
        let dns = match value["dns"]["status"].as_str() {
            Some("physical-bootstrap-available") => StartupCheck::Passed,
            Some("physical-bootstrap-failed-or-timed-out") => StartupCheck::Failed,
            Some("literal-no-dns-required") => StartupCheck::NotApplicable,
            _ => StartupCheck::NotTested,
        };
        let node_tcp = match value["nodeTcp"]["status"].as_str() {
            Some("reachable") => StartupCheck::Passed,
            Some("failed-or-timed-out") => StartupCheck::Failed,
            Some("not-applicable-udp-protocol") => StartupCheck::NotApplicable,
            _ => StartupCheck::NotTested,
        };
        Some(Self {
            session: session.to_owned(),
            engine,
            dns,
            node_tcp,
            https: if value["success"].as_bool()? {
                StartupCheck::Passed
            } else {
                StartupCheck::Failed
            },
            elapsed_ms: value["elapsedMs"].as_u64().unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    #[test]
    fn cached_health_requires_the_current_session_and_actual_engine() {
        let value = serde_json::json!({"session":"old", "actualEngine":"sing-box", "success":true});
        assert!(StartupHealth::from_evidence(&value, "new", VpnEngine::SingBox).is_none());
        assert!(StartupHealth::from_evidence(&value, "old", VpnEngine::Mihomo).is_none());
        assert!(
            StartupHealth::from_evidence(&serde_json::json!({}), "old", VpnEngine::SingBox)
                .is_none()
        );
    }

    #[test]
    fn udp_or_literal_addresses_are_skipped_without_claiming_probe_success() {
        let value = serde_json::json!({"session":"current", "actualEngine":"meow", "success":false,
            "dns":{"status":"literal-no-dns-required"}, "nodeTcp":{"status":"not-applicable-udp-protocol"}});
        let health = StartupHealth::from_evidence(&value, "current", VpnEngine::Meow).unwrap();
        assert_eq!(health.dns, StartupCheck::NotApplicable);
        assert_eq!(health.node_tcp, StartupCheck::NotApplicable);
        assert_eq!(health.https, StartupCheck::Failed);
    }
}
impl CoreHandle {
    /// Read only the bounded evidence for the current session. Diagnostics do
    /// not need to clone connections, DNS records, history and log projections.
    pub fn startup_health(&self) -> Result<Option<StartupHealth>, PawsError> {
        let (root, session, engine) = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            (
                state.profiles.root().to_path_buf(),
                platform_vpn_session_id(&state),
                state.platform_vpn_engine,
            )
        };
        let health = StartupHealth::read_current(&root, session.as_deref(), engine);
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        Ok(
            (platform_vpn_session_id(&state) == session && state.platform_vpn_engine == engine)
                .then_some(health)
                .flatten(),
        )
    }

    pub async fn validate_engine_startup(
        &self,
        session: &str,
    ) -> Result<serde_json::Value, PawsError> {
        let root = { self.lock_state()?.profiles.root().to_path_buf() };
        let mut evidence = None;
        for _ in 0..40 {
            evidence = process_evidence::read_bounded(&root.join("engine-session.json"), 8192)
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .filter(|r| r["session"].as_str() == Some(session));
            if evidence.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let evidence = evidence
            .ok_or_else(|| PawsError::Core("actual engine session is unavailable".into()))?;
        let engine = evidence["engine"].as_str().unwrap_or("unknown");
        let proxy_url = {
            let state = self.lock_state()?;
            ensure_platform_attempt_active(&state, session)?;
            state
                .platform_app_http_proxy
                .as_ref()
                .ok_or_else(|| {
                    PawsError::Core("actual engine authenticated ingress is unavailable".into())
                })?
                .proxy_url()
        };
        let node = {
            let state = self.lock_state()?;
            let name = crate::engine_policy::selected_node(&state);
            let raw = state.profiles.active_raw_yaml()?;
            let parsed: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap_or_default();
            parsed
                .get("proxies")
                .and_then(serde_yaml::Value::as_sequence)
                .and_then(|nodes| {
                    nodes.iter().find(|p| {
                        p.get("name").and_then(serde_yaml::Value::as_str) == Some(name.as_str())
                    })
                })
                .and_then(|node| {
                    Some((
                        node.get("server")?.as_str()?.to_owned(),
                        u16::try_from(node.get("port")?.as_u64()?).ok()?,
                        node.get("type")?.as_str()?.to_owned(),
                    ))
                })
        };
        let started = Instant::now();
        let https_probe = async {
            let result: Result<(u16, usize), String> = async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .proxy(
                        reqwest::Proxy::all(&proxy_url)
                            .map_err(|_| "proxy-configuration".to_owned())?,
                    )
                    .timeout(Duration::from_secs(8))
                    .build()
                    .map_err(|_| "probe-configuration".to_owned())?;
                let mut response = client
                    .get("https://cp.cloudflare.com/generate_204")
                    .send()
                    .await
                    .map_err(|error| -> String {
                        if error.is_timeout() {
                            "timeout".into()
                        } else if error.is_connect() {
                            "dns-or-node-or-tls".into()
                        } else {
                            "https".into()
                        }
                    })?;
                let status = response.status().as_u16();
                let mut returned = response
                    .headers()
                    .iter()
                    .map(|(k, v)| k.as_str().len() + v.len() + 4)
                    .sum::<usize>();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "return-traffic".to_owned())?
                {
                    returned = returned.saturating_add(chunk.len());
                    if returned > 65536 {
                        return Err("response-too-large".into());
                    }
                }
                if !(200..400).contains(&status) || returned == 0 {
                    return Err("https-status-or-return-traffic".into());
                }
                Ok((status, returned))
            }
            .await;
            result
        };
        let (result, independent) = tokio::join!(https_probe, probe_selected_node(node));
        let (success, status, returned, stage) = match result {
            Ok((status, returned)) => (true, status, returned, "return-traffic".to_owned()),
            Err(stage) => (false, 0, 0, stage),
        };
        self.record_engine_health(
            session,
            success,
            if success { "return-traffic" } else { "https" },
        )?;
        let rollback = !success
            && engine != "mihomo"
            && option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC") != Some("1");
        let report = serde_json::json!({"session":session,"actualEngine":engine,"success":success,"elapsedMs":started.elapsed().as_millis(),"httpsStatus":status,"applicationReturnBytes":returned,"stage":stage,"rollbackToBaseline":rollback,
            "dns":independent["dns"],"nodeTcp":independent["tcp"],"nodeHandshake":{"status":if success{"authenticated-proxy-path-returned-https"}else{"unconfirmed"}},"targetHttps":success,
            "limits":["Independent DNS/TCP probes target the selected GLOBAL node; a rule-routed target can use another node","Physical bootstrap DNS does not certify a remote proxy DNS resolver","TCP connectivity alone does not authenticate the proxy protocol","Application return bytes exclude encrypted transport overhead"]});
        write_atomic_file(
            &root.join("engine-startup-health.json"),
            &serde_json::to_vec(&report).map_err(|e| PawsError::Core(e.to_string()))?,
        )?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn https_health_check_uses_the_engine_proxy_and_records_refusal() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let root = std::env::temp_dir().join(format!("paws-engine-health-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        core.import_profile_from_content(
            "local",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        {
            let mut state = core.lock_state().unwrap();
            state.platform_start_attempt_id = "probe-test".into();
            state.platform_extension_attached = true;
            state.platform_start_outcome = PlatformStartOutcome::Connected;
            state.platform_vpn_running = true;
            state.platform_vpn_engine = Some(VpnEngine::SingBox);
            state.platform_app_http_proxy = Some(AppHttpProxyEndpoint {
                port,
                token: "synthetic-health-token".into(),
            });
        }
        std::fs::write(
            root.join("engine-session.json"),
            serde_json::json!({"session":"probe-test", "engine":"sing-box", "ingressPort":port})
                .to_string(),
        )
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 2048];
            let size = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..size])
                .starts_with("CONNECT cp.cloudflare.com:443 "));
            assert!(
                String::from_utf8_lossy(&request[..size])
                    .lines()
                    .any(|line| line
                        .to_ascii_lowercase()
                        .starts_with("proxy-authorization: basic ")),
                "startup health must use the authenticated engine ingress"
            );
            stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let health = core.validate_engine_startup("probe-test").await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(health["success"], false);
        assert_eq!(health["rollbackToBaseline"], true);
        assert_eq!(health["applicationReturnBytes"], 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}

async fn probe_selected_node(node: Option<(String, u16, String)>) -> serde_json::Value {
    use serde_json::json;
    let Some((host, port, protocol)) = node else {
        return json!({"dns":{"status":"selected-node-unresolved","independentProbe":false},"tcp":{"status":"not-tested"}});
    };
    let literal = host.parse::<std::net::IpAddr>().is_ok();
    let started = Instant::now();
    let addresses = match tokio::time::timeout(
        Duration::from_secs(3),
        crate::network_binding::resolve_physical_host(&host),
    )
    .await
    {
        Ok(Ok(addresses)) if !addresses.is_empty() => addresses,
        _ => {
            return json!({"dns":{"status":"physical-bootstrap-failed-or-timed-out","independentProbe":!literal,"elapsedMs":started.elapsed().as_millis()},"tcp":{"status":"not-tested"}})
        }
    };
    let dns = json!({"status":if literal{"literal-no-dns-required"}else{"physical-bootstrap-available"},"independentProbe":!literal,"scope":"selected-global-node-physical-bootstrap","addressCount":addresses.len(),"elapsedMs":started.elapsed().as_millis()});
    if matches!(protocol.as_str(), "hysteria2" | "hy2" | "tuic") {
        return json!({"dns":dns,"tcp":{"status":"not-applicable-udp-protocol","protocolAuthentication":"not-tested"}});
    };
    let tcp_started = Instant::now();
    let mut attempts = tokio::task::JoinSet::new();
    for address in addresses.into_iter().take(4) {
        attempts.spawn(async move {
            meow_common::connect_tcp(std::net::SocketAddr::new(address, port))
                .await
                .map(drop)
        });
    }
    let reachable = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = attempts.join_next().await {
            if matches!(result, Ok(Ok(()))) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    attempts.abort_all();
    json!({"dns":dns,"tcp":{"status":if reachable{"reachable"}else{"failed-or-timed-out"},"independentProbe":true,"elapsedMs":tcp_started.elapsed().as_millis(),"protocolAuthentication":"requires-return-traffic-through-engine"}})
}

#[cfg(test)]
mod independent_probe_tests {
    use super::*;
    #[tokio::test]
    async fn tcp_reachability_does_not_claim_dns_or_protocol_authentication() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let result = probe_selected_node(Some((
            "127.0.0.1".into(),
            listener.local_addr().unwrap().port(),
            "ss".into(),
        )))
        .await;
        assert_eq!(result["dns"]["independentProbe"], false);
        assert_eq!(result["tcp"]["status"], "reachable");
        assert_eq!(
            result["tcp"]["protocolAuthentication"],
            "requires-return-traffic-through-engine"
        );
        let udp = probe_selected_node(Some(("127.0.0.1".into(), 1, "tuic".into()))).await;
        assert_eq!(udp["tcp"]["status"], "not-applicable-udp-protocol");
    }
}
