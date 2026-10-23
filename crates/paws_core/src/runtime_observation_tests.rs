use super::*;
use axum::{
    extract::{Query, State},
    http::{StatusCode, Uri},
    Json, Router,
};
use paws_engine::{EngineConnectionBuffer, EngineLogBuffer};

#[derive(Clone)]
struct Endpoint {
    calls: Arc<Mutex<Vec<String>>>,
    logs: Arc<EngineLogBuffer>,
    events: Arc<EngineConnectionBuffer>,
    hold: Option<(String, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: u64,
    #[serde(default = "limit")]
    limit: u16,
}
fn limit() -> u16 {
    512
}
fn connection(id: &str, bytes: u64) -> ConnectionSummary {
    ConnectionSummary {
        id: id.into(),
        domain_source: "native-sniffer".into(),
        host: "example.test".into(),
        domain: "example.test".into(),
        destination_ip: "192.0.2.10".into(),
        destination_port: 443,
        network: "TCP".into(),
        rule: "DomainSuffix".into(),
        rule_payload: "example.test".into(),
        proxy: "node".into(),
        chains: vec!["group".into(), "node".into()],
        started_at: "2026-10-22T00:00:00Z".into(),
        upload_bytes: bytes,
        download_bytes: bytes * 2,
    }
}
async fn handler(
    State(endpoint): State<Endpoint>,
    uri: Uri,
    Query(cursor): Query<Cursor>,
) -> (StatusCode, Json<serde_json::Value>) {
    use serde_json::json;
    let path = uri.path().to_owned();
    endpoint.calls.lock().unwrap().push(path.clone());
    if let Some((held, entered, release)) = &endpoint.hold {
        if &path == held {
            entered.notify_one();
            tokio::time::timeout(Duration::from_secs(5), release.notified())
                .await
                .expect("fixture release was not delivered");
        }
    }
    let value = match path.as_str() {
        "/paws/observations/revisions" => {
            json!({"groups":0,"providers":0,"rules":0,"dns":0,"connections":0,"logs":endpoint.logs.page(0,1).unwrap().high_watermark})
        }
        "/paws/observations/groups" => {
            json!([{ "name":"group", "groupType":"Selector", "selected":"node", "fixed":null, "memberCount":900, "selectedDelayMs":27 }])
        }
        "/paws/observations/connection-summary" => {
            json!({ "count":1,"uploadBytes":33,"downloadBytes":66,"recent":[connection("current",33)],"latestRuleNode":"node" })
        }
        "/paws/observations/proxy-groups/group" => {
            json!({ "name":"group","groupType":"Selector","selected":"node","fixed":null,"proxies":[{"name":"node","proxyType":"Socks5","delayMs":27,"selected":true}] })
        }
        "/paws/observations/rules" => {
            json!([{ "ruleType":"DomainSuffix","payload":"example.test","target":"group" }])
        }
        "/paws/observations/memory" => return (StatusCode::NOT_IMPLEMENTED, Json(json!({}))),
        "/paws/observations/dns-config" => {
            json!({"model":"native","listen":"127.0.0.1:15353","upstreams":["https://dns.example.test/query"],"fallbacks":[],"nameserverPolicy":{}})
        }
        "/paws/observations/dns" => return (StatusCode::NOT_IMPLEMENTED, Json(json!({}))),
        "/paws/observations/logs" => {
            serde_json::to_value(endpoint.logs.page(cursor.after, cursor.limit).unwrap()).unwrap()
        }
        "/paws/observations/connection-events" => {
            serde_json::to_value(endpoint.events.page(cursor.after, cursor.limit).unwrap()).unwrap()
        }
        "/connections" => {
            json!({"connections":[{"id":"current","metadata":{"host":"example.test","destinationIP":"192.0.2.10","destinationPort":443,"network":"TCP"},"upload":33,"download":66,"rule":"DomainSuffix","rulePayload":"example.test","chains":["group","node"],"start":"2026-10-22T00:00:00Z"}]})
        }
        _ => return (StatusCode::NOT_FOUND, Json(json!({}))),
    };
    (StatusCode::OK, Json(value))
}
struct Fixture {
    core: Arc<CoreHandle>,
    endpoint: Endpoint,
    root: PathBuf,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new(
        engine: VpnEngine,
        hold: Option<(String, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> Self {
        let endpoint = Endpoint {
            calls: Arc::default(),
            logs: Arc::new(EngineLogBuffer::new(engine, 2048, 2 * 1024 * 1024)),
            events: Arc::new(EngineConnectionBuffer::new(engine, 2048, 2 * 1024 * 1024)),
            hold,
        };
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().fallback(handler).with_state(endpoint.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let root = loop {
            let root = std::env::temp_dir().join(format!(
                "paws-observation-{}-{}-{}",
                std::process::id(),
                now_unix_nanos(),
                NEXT_ROOT.fetch_add(1, Ordering::SeqCst)
            ));
            match fs::create_dir(&root) {
                Ok(()) => break root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create unique observation fixture root: {error}"),
            }
        };
        let core = Arc::new(CoreHandle::new_with_profile_root(&root));
        {
            let mut state = core.lock_state().unwrap();
            state.platform_start_attempt_id = "owner-a".into();
            state.platform_vpn_running = true;
            state.platform_start_outcome = PlatformStartOutcome::Connected;
            state.platform_vpn_engine = Some(engine);
            state.platform_controller_endpoint = Some(paws_engine::EngineControllerEndpoint {
                address,
                secret: None,
            });
        }
        Self {
            core,
            endpoint,
            root,
            server,
        }
    }
    async fn read(&self, domain: RuntimeDomain, group_key: Option<&str>) -> serde_json::Value {
        serde_json::to_value(
            self.core
                .refresh_and_query_runtime_domain(RuntimeDomainKey {
                    domain,
                    group_key: group_key.map(ToOwned::to_owned),
                })
                .await
                .unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn live_queries_read_only_the_requested_domain_and_preserve_unavailable_semantics() {
    for engine in [VpnEngine::Meow, VpnEngine::Mihomo, VpnEngine::SingBox] {
        let fixture = Fixture::new(engine, None).await;
        let summary = fixture.read(RuntimeDomain::Summary, None).await;
        assert_eq!(summary["data"]["groups"][0]["memberCount"], 900);
        assert_eq!(summary["data"]["primarySelectedNode"], "node");
        assert_eq!(summary["observation"]["session"], "owner-a");
        assert_eq!(
            *fixture.endpoint.calls.lock().unwrap(),
            [
                "/paws/observations/groups",
                "/paws/observations/connection-summary"
            ]
        );
        fixture.endpoint.calls.lock().unwrap().clear();
        let group = fixture.read(RuntimeDomain::Proxies, Some("group")).await;
        assert_eq!(group["data"]["proxyGroups"][0]["proxies"][0]["delayMs"], 27);
        assert_eq!(
            *fixture.endpoint.calls.lock().unwrap(),
            ["/paws/observations/proxy-groups/group"]
        );
        let rules = fixture.read(RuntimeDomain::Rules, None).await;
        assert_eq!(rules["data"]["runtimeRules"][0]["payload"], "example.test");
        assert!(
            rules["data"]["rules"].as_array().unwrap().is_empty(),
            "profile rules remain a distinct editable source"
        );
        let traffic = fixture.read(RuntimeDomain::Traffic, None).await;
        assert_eq!(
            traffic["data"]["controllerDiagnostics"]["memoryAvailable"],
            false
        );
        assert_eq!(traffic["data"]["connectionSummary"]["uploadBytes"], 33);
        assert!(!traffic["observation"]["issues"]
            .as_array()
            .unwrap()
            .is_empty());
        let dns = fixture.read(RuntimeDomain::Dns, None).await;
        assert_eq!(dns["data"]["dns"]["listen"], "127.0.0.1:15353");
        assert_eq!(dns["data"]["dns"]["statisticsAvailable"], false);
        assert!(dns["data"]["dns"]["statisticsError"]
            .as_str()
            .unwrap()
            .contains("unavailable"));
    }
}

#[tokio::test]
async fn readback_from_an_old_session_or_configuration_never_commits_or_projects() {
    for replacement in [0, 1, 2] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let fixture = Fixture::new(
            VpnEngine::Mihomo,
            Some((
                "/paws/observations/dns-config".into(),
                entered.clone(),
                release.clone(),
            )),
        )
        .await;
        let core = fixture.core.clone();
        let pending = tokio::spawn(async move {
            core.refresh_and_query_runtime_domain(RuntimeDomainKey {
                domain: RuntimeDomain::Dns,
                group_key: None,
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .expect("fixture request was never sent");
        {
            let mut state = fixture.core.lock_state().unwrap();
            match replacement {
                0 => state.config_revision += 1,
                1 => state.platform_start_attempt_id = "owner-b".into(),
                _ => {
                    state.platform_controller_endpoint.as_mut().unwrap().secret =
                        Some("replacement-epoch".into())
                }
            }
        }
        release.notify_one();
        assert!(tokio::time::timeout(Duration::from_secs(10), pending)
            .await
            .expect("fixture command did not terminate")
            .unwrap()
            .is_err());
        assert_ne!(
            fixture.core.lock_state().unwrap().dns.listen,
            "127.0.0.1:15353"
        );
    }
}

#[tokio::test]
async fn cursor_queries_drain_a_known_backlog_and_clear_does_not_resurrect_old_records() {
    let fixture = Fixture::new(VpnEngine::Mihomo, None).await;
    assert!(
        fixture
            .core
            .set_log_recording_enabled(true)
            .unwrap()
            .enabled
    );
    {
        let state = fixture.core.lock_state().unwrap();
        assert!(
            state.logs.enabled(),
            "writer root {:?}, profiles root {:?}",
            state.logs.root(),
            state.profiles.root()
        );
    }
    for index in 0..700 {
        fixture
            .endpoint
            .logs
            .record(info_log(format!("fixture-{index}")))
            .unwrap();
        fixture
            .endpoint
            .events
            .record(connection(&format!("closed-{index}"), index), true)
            .unwrap();
    }
    let logs = fixture.read(RuntimeDomain::Logs, None).await;
    let info = {
        let state = fixture.core.lock_state().unwrap();
        (
            state.engine_observation.log_cursor,
            state.platform_logs.as_ref().map_or(0, Vec::len),
            state.logs.enabled(),
        )
    };
    assert!(
        logs["data"]["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["message"] == "fixture-699"),
        "state={info:?} messages={:?}",
        logs["data"]["logs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["message"].as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture
            .core
            .lock_state()
            .unwrap()
            .engine_observation
            .log_cursor,
        700
    );
    let requests = fixture.read(RuntimeDomain::Requests, None).await;
    assert_eq!(
        fixture
            .core
            .lock_state()
            .unwrap()
            .engine_observation
            .connection_cursor,
        700
    );
    let closed = requests["data"]["requestHistory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "closed-699")
        .unwrap();
    assert_eq!(closed["active"], false);
    assert_eq!(closed["rulePayload"], "example.test");
    assert_eq!(closed["chains"][1], "node");
    let active = requests["data"]["requestHistory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "current")
        .unwrap();
    assert_eq!(active["active"], true);
    assert_eq!(active["uploadBytes"], 33);
    fixture.core.clear_runtime_history(true).await.unwrap();
    fixture.core.clear_runtime_history(false).await.unwrap();
    fixture
        .endpoint
        .logs
        .record(info_log("fixture-after-clear"))
        .unwrap();
    let logs = fixture.read(RuntimeDomain::Logs, None).await;
    let messages = logs["data"]["logs"].as_array().unwrap();
    assert!(messages
        .iter()
        .any(|entry| entry["message"] == "fixture-after-clear"));
    assert!(!messages
        .iter()
        .any(|entry| entry["message"] == "fixture-699"));
    let requests = fixture.read(RuntimeDomain::Requests, None).await;
    let history = requests["data"]["requestHistory"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["id"], "current");
    assert_eq!(history[0]["active"], true);
}

#[tokio::test]
async fn closed_query_lease_cannot_commit_partial_errors_or_advance_cursors() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = Fixture::new(
        VpnEngine::Mihomo,
        Some((
            "/paws/observations/logs".into(),
            entered.clone(),
            release.clone(),
        )),
    )
    .await;
    fixture
        .endpoint
        .logs
        .record(info_log("never-committed"))
        .unwrap();
    let alive = Arc::new(AtomicBool::new(true));
    let query_alive = alive.clone();
    let core = fixture.core.clone();
    let pending = tokio::spawn(async move {
        core.refresh_and_query_runtime_domain_with_guard(
            RuntimeDomainKey {
                domain: RuntimeDomain::Logs,
                group_key: None,
            },
            move || query_alive.load(Ordering::SeqCst),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("fixture request was never sent");
    alive.store(false, Ordering::SeqCst);
    release.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("fixture command did not terminate")
        .unwrap()
        .is_err());
    let state = fixture.core.lock_state().unwrap();
    assert_eq!(state.engine_observation.log_cursor, 0);
    assert!(state.platform_logs.is_none());
    assert!(state.engine_observation.attempt.is_empty());
}

#[tokio::test]
async fn clear_cannot_use_an_old_client_with_a_new_same_session_credential_epoch() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = Fixture::new(
        VpnEngine::Mihomo,
        Some((
            "/paws/observations/logs".into(),
            entered.clone(),
            release.clone(),
        )),
    )
    .await;
    fixture.endpoint.logs.record(info_log("retained")).unwrap();
    let core = fixture.core.clone();
    let pending = tokio::spawn(async move { core.clear_runtime_history(true).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("fixture request was never sent");
    {
        let mut state = fixture.core.lock_state().unwrap();
        state.platform_controller_endpoint.as_mut().unwrap().secret =
            Some("new-same-session-epoch".into());
    }
    release.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("fixture command did not terminate")
        .unwrap()
        .is_err());
    assert_eq!(
        fixture
            .core
            .lock_state()
            .unwrap()
            .engine_observation
            .log_cursor,
        0
    );
}

#[tokio::test]
async fn native_archives_have_an_independent_cursor_and_off_on_cuts_disabled_backlog() {
    let fixture = Fixture::new(VpnEngine::Mihomo, None).await;
    fixture.core.set_log_recording_enabled(false).unwrap();
    fixture
        .endpoint
        .logs
        .record(info_log("before-enable"))
        .unwrap();
    assert!(
        fixture
            .core
            .set_engine_log_recording_enabled(true)
            .await
            .unwrap()
            .enabled
    );
    let boundary = log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .unwrap();
    assert_eq!(boundary.cursor, 1);
    fixture
        .endpoint
        .logs
        .record(info_log("after-enable"))
        .unwrap();
    let _ = fixture.read(RuntimeDomain::Logs, None).await;
    assert_eq!(
        log_recording::native_recording_boundary(&fixture.root)
            .unwrap()
            .unwrap()
            .cursor,
        1,
        "UI must not steal the archive cursor"
    );
    fixture.core.consume_engine_runtime_logs().await.unwrap();
    fixture.core.consume_engine_runtime_logs().await.unwrap();
    assert_eq!(
        log_recording::native_recording_boundary(&fixture.root)
            .unwrap()
            .unwrap()
            .cursor,
        2
    );
    fixture
        .core
        .set_engine_log_recording_enabled(false)
        .await
        .unwrap();
    fixture
        .endpoint
        .logs
        .record(info_log("while-disabled"))
        .unwrap();
    fixture
        .core
        .set_engine_log_recording_enabled(true)
        .await
        .unwrap();
    let next = log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .unwrap();
    assert_ne!(boundary.nonce, next.nonce);
    assert_eq!(next.cursor, 3);
    fixture
        .endpoint
        .logs
        .record(info_log("after-reenable"))
        .unwrap();
    fixture.core.consume_engine_runtime_logs().await.unwrap();
    let status = fixture.core.log_recording_status().unwrap();
    let body = status
        .archives
        .iter()
        .map(|archive| fixture.core.read_log_archive(&archive.file_name).unwrap())
        .collect::<String>();
    assert!(!body.contains("before-enable"));
    assert!(!body.contains("while-disabled"));
    assert_eq!(body.matches("after-enable").count(), 1);
    assert_eq!(body.matches("after-reenable").count(), 1);
}

#[tokio::test]
async fn a_native_log_page_received_after_recording_stops_cannot_append() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = Fixture::new(
        VpnEngine::Mihomo,
        Some((
            "/paws/observations/logs".into(),
            entered.clone(),
            release.clone(),
        )),
    )
    .await;
    fixture.core.set_log_recording_enabled(true).unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .core
            .initialize_native_log_recording(now_unix_nanos()),
    )
    .await
    .expect("archive initialization did not terminate")
    .unwrap();
    assert!(
        log_recording::native_recording_boundary(&fixture.root)
            .unwrap()
            .is_some(),
        "recording must be initialized before waiting for a held native page"
    );
    fixture
        .endpoint
        .logs
        .record(info_log("late-after-off"))
        .unwrap();
    let core = fixture.core.clone();
    let pending = tokio::spawn(async move { core.consume_engine_runtime_logs().await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("fixture request was never sent");
    fixture
        .core
        .set_engine_log_recording_enabled(false)
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("fixture command did not terminate")
        .unwrap()
        .unwrap();
    let status = fixture.core.log_recording_status().unwrap();
    assert!(!status.enabled);
    let body = status
        .archives
        .iter()
        .map(|archive| fixture.core.read_log_archive(&archive.file_name).unwrap())
        .collect::<String>();
    assert!(!body.contains("late-after-off"));
}

#[tokio::test]
async fn explicit_off_cancels_an_enable_waiting_for_the_native_watermark() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = Fixture::new(
        VpnEngine::Mihomo,
        Some((
            "/paws/observations/logs".into(),
            entered.clone(),
            release.clone(),
        )),
    )
    .await;
    fixture.core.set_log_recording_enabled(false).unwrap();
    let core = fixture.core.clone();
    let pending = tokio::spawn(async move { core.set_engine_log_recording_enabled(true).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("fixture request was never sent");
    assert!(
        !fixture
            .core
            .set_engine_log_recording_enabled(false)
            .await
            .unwrap()
            .enabled
    );
    release.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("fixture command did not terminate")
        .unwrap()
        .is_err());
    assert!(!fixture.core.log_recording_status().unwrap().enabled);
    assert!(log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_new_native_epoch_uses_its_own_archive_sequence_even_with_the_same_attempt_and_revision()
{
    let fixture = Fixture::new(VpnEngine::Mihomo, None).await;
    fixture.core.set_log_recording_enabled(true).unwrap();
    fixture
        .core
        .initialize_native_log_recording(now_unix_nanos())
        .await
        .unwrap();
    fixture
        .endpoint
        .logs
        .record(info_log("epoch-a-first"))
        .unwrap();
    fixture.core.consume_engine_runtime_logs().await.unwrap();
    let before = log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .unwrap();
    let replacement = Fixture::new(VpnEngine::Mihomo, None).await;
    let endpoint = replacement
        .core
        .lock_state()
        .unwrap()
        .platform_controller_endpoint
        .clone();
    {
        fixture
            .core
            .lock_state()
            .unwrap()
            .platform_controller_endpoint = endpoint;
    }
    replacement
        .endpoint
        .logs
        .record(info_log("epoch-b-first"))
        .unwrap();
    fixture
        .core
        .initialize_native_log_recording(now_unix_nanos())
        .await
        .unwrap();
    let after = log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .unwrap();
    assert_eq!(before.nonce, after.nonce);
    assert_ne!(before.source.epoch, after.source.epoch);
    assert_eq!(after.cursor, 0);
    fixture.core.consume_engine_runtime_logs().await.unwrap();
    let body = fixture
        .core
        .log_recording_status()
        .unwrap()
        .archives
        .iter()
        .map(|archive| fixture.core.read_log_archive(&archive.file_name).unwrap())
        .collect::<String>();
    assert_eq!(body.matches("epoch-a-first").count(), 1);
    assert_eq!(body.matches("epoch-b-first").count(), 1);
}

struct TrafficOwner {
    endpoint: paws_engine::EngineControllerEndpoint,
    samples: AtomicU64,
}
#[async_trait::async_trait]
impl paws_engine::EngineSession for TrafficOwner {
    fn engine(&self) -> VpnEngine {
        VpnEngine::Mihomo
    }
    fn capabilities(&self) -> paws_engine::EngineCapabilities {
        paws_engine_go::GoEngineSession::descriptor(self.engine()).unwrap()
    }
    fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
        Some(paws_engine::EngineManagementBinding {
            client: Arc::new(
                paws_engine_go::ControllerClient::new(self.engine(), self.endpoint.clone())
                    .unwrap(),
            ),
            endpoint: self.endpoint.clone(),
        })
    }
    fn is_running(&self) -> bool {
        true
    }
    async fn start(&self, _: paws_engine::EngineStart) -> Result<u64, paws_engine::EngineError> {
        Ok(1)
    }
    async fn stop(&self) -> Result<(), paws_engine::EngineError> {
        Ok(())
    }
    fn traffic(&self) -> Result<paws_engine::EngineTraffic, paws_engine::EngineError> {
        let sequence = self.samples.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(paws_engine::EngineTraffic {
            upload_speed: sequence,
            download_speed: sequence * 2,
            upload_total: sequence,
            download_total: sequence * 2,
        })
    }
    fn version(&self) -> Result<String, paws_engine::EngineError> {
        Ok("synthetic".into())
    }
}

#[tokio::test]
async fn a_held_archive_cannot_stall_traffic_publication_and_native_stop_joins_its_lane() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = Fixture::new(
        VpnEngine::Mihomo,
        Some((
            "/paws/observations/logs".into(),
            entered.clone(),
            release.clone(),
        )),
    )
    .await;
    fixture.core.set_log_recording_enabled(true).unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .core
            .initialize_native_log_recording(now_unix_nanos()),
    )
    .await
    .expect("archive boundary initialization")
    .unwrap();
    assert!(log_recording::native_recording_boundary(&fixture.root)
        .unwrap()
        .is_some());
    let endpoint = fixture
        .core
        .lock_state()
        .unwrap()
        .platform_controller_endpoint
        .clone()
        .unwrap();
    *fixture.core.active_engine.lock().unwrap() = Some(engine::ActiveEngineSession {
        attempt: "owner-a".into(),
        session: Arc::new(TrafficOwner {
            endpoint,
            samples: AtomicU64::new(0),
        }),
    });
    fixture
        .endpoint
        .logs
        .record(info_log("held-archive-record"))
        .unwrap();
    let observer_core = fixture.core.clone();
    let observer = tokio::spawn(async move { observer_core.poll_engine_observations().await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("archive HTTP request never started");
    let before = fixture.core.telemetry_publications.lock().unwrap().len();
    for sequence in [1, 2] {
        tokio::time::timeout(
            Duration::from_millis(250),
            fixture.core.poll_engine_traffic(),
        )
        .await
        .expect("traffic stalled behind archive")
        .unwrap();
        let publications = fixture.core.telemetry_publications.lock().unwrap();
        let dto = serde_json::to_value(publications.last().unwrap()).unwrap();
        assert_eq!(dto["traffic"]["uploadBytes"], sequence);
        assert!(dto.get("logs").is_none());
    }
    assert_eq!(
        fixture.core.telemetry_publications.lock().unwrap().len(),
        before + 2
    );
    assert!(!observer.is_finished());
    tokio::time::timeout(Duration::from_secs(1), fixture.core.stop_active_engine())
        .await
        .expect("stop failed to cancel and join observation lane")
        .unwrap();
    observer.await.unwrap().unwrap();
    release.notify_one();
    let body = fixture
        .core
        .log_recording_status()
        .unwrap()
        .archives
        .iter()
        .map(|archive| fixture.core.read_log_archive(&archive.file_name).unwrap())
        .collect::<String>();
    assert!(!body.contains("held-archive-record"));
}

#[tokio::test]
async fn archive_outcomes_are_bounded_and_cannot_clear_control_errors_or_revive_after_off() {
    let fixture = Fixture::new(VpnEngine::Mihomo, None).await;
    fixture.core.set_log_recording_enabled(true).unwrap();
    let target = fixture.core.runtime_management().unwrap().unwrap();
    let nonce = {
        let mut state = fixture.core.lock_state().unwrap();
        state
            .logs
            .record_control_error(&PawsError::Core("control failure remains visible".into()));
        state.logs.session_id().unwrap().to_owned()
    };
    let failure = PawsError::Core("记录失败🙂".repeat(300));
    fixture
        .core
        .record_native_archive_outcome(&target, &nonce, Some(&failure))
        .unwrap();
    {
        let state = fixture.core.lock_state().unwrap();
        let frame = platform_vpn_telemetry_projection(&state);
        let error = frame
            .recording_error
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap();
        assert!(error.len() <= 1024);
        assert!(error.is_char_boundary(error.len()));
        let wire = serde_json::to_value(frame).unwrap();
        assert!(wire["recordingError"]["error"].as_str().unwrap().len() <= 1024);
        assert!(wire.get("logs").is_none());
        assert!(wire.get("requestHistory").is_none());
        let status = log_recording_error(&state).unwrap();
        assert!(status.contains("control failure remains visible"));
        assert!(status.contains("记录失败"));
    }
    fixture.core.persist_vpn_telemetry().unwrap();
    fixture.core.persist_vpn_telemetry().unwrap();
    {
        let samples = fixture.core.telemetry_publications.lock().unwrap();
        assert!(
            samples[1].updated_at > samples[0].updated_at,
            "independent lane captures need a strict order even on a coarse clock"
        );
    }
    fixture
        .core
        .record_native_archive_outcome(&target, &nonce, None)
        .unwrap();
    {
        let state = fixture.core.lock_state().unwrap();
        assert!(state.archive_status.as_ref().unwrap().error.is_none());
        assert!(log_recording_error(&state)
            .unwrap()
            .contains("control failure remains visible"));
        let projection = fixture
            .core
            .project_runtime_domain_locked(
                &state,
                RuntimeDomainKey {
                    domain: RuntimeDomain::Logs,
                    group_key: None,
                },
            )
            .unwrap();
        let wire = serde_json::to_value(projection).unwrap();
        assert!(wire["data"]["recordingError"]
            .as_str()
            .unwrap()
            .contains("control failure remains visible"));
    }
    fixture.core.set_log_recording_enabled(false).unwrap();
    fixture
        .core
        .record_native_archive_outcome(&target, &nonce, Some(&failure))
        .unwrap();
    let state = fixture.core.lock_state().unwrap();
    assert!(platform_vpn_telemetry_projection(&state)
        .recording_error
        .is_none());
}

#[tokio::test]
async fn archive_metadata_rejects_older_owner_epoch_nonce_and_error_or_clear_frames() {
    let fixture = Fixture::new(VpnEngine::Mihomo, None).await;
    fixture.core.set_log_recording_enabled(true).unwrap();
    let target = fixture.core.runtime_management().unwrap().unwrap();
    let nonce = fixture
        .core
        .lock_state()
        .unwrap()
        .logs
        .session_id()
        .unwrap()
        .to_owned();
    fixture
        .core
        .record_native_archive_outcome(
            &target,
            &nonce,
            Some(&PawsError::Core("archive failed".into())),
        )
        .unwrap();
    let mut state = fixture.core.lock_state().unwrap();
    let mut frame = platform_vpn_telemetry_projection(&state);
    frame.updated_at = 30;
    apply_platform_archive_status(&mut state, &frame);
    assert!(state
        .platform_archive_status
        .as_ref()
        .unwrap()
        .error
        .is_some());
    let mut success = frame.clone();
    success.updated_at = 40;
    success.recording_error.as_mut().unwrap().error = None;
    apply_platform_archive_status(&mut state, &success);
    assert!(state
        .platform_archive_status
        .as_ref()
        .unwrap()
        .error
        .is_none());
    frame.updated_at = 35;
    assert!(!platform_telemetry_is_current(&state, &frame));
    apply_platform_archive_status(&mut state, &frame);
    assert!(state
        .platform_archive_status
        .as_ref()
        .unwrap()
        .error
        .is_none());
    let mut obsolete = success.clone();
    obsolete.updated_at = 50;
    obsolete.session_id = "old-owner".into();
    assert!(!platform_telemetry_is_current(&state, &obsolete));
    obsolete.session_id = state.platform_start_attempt_id.clone();
    obsolete.management_epoch = Some("old-credential-epoch".into());
    assert!(!platform_telemetry_is_current(&state, &obsolete));
    // A credential-only reload is a different actual config even if attempt
    // and Core configRevision are unchanged; both stale errors and clears lose.
    state.platform_controller_endpoint.as_mut().unwrap().secret =
        Some("new-private-credential".into());
    assert!(!platform_telemetry_is_current(&state, &frame));
    assert!(!platform_telemetry_is_current(&state, &success));
    drop(state);
    assert!(fixture
        .core
        .record_native_archive_outcome(&target, &nonce, None)
        .is_err());
    fixture.core.set_log_recording_enabled(false).unwrap();
    let mut state = fixture.core.lock_state().unwrap();
    let mut after_off = platform_vpn_telemetry_projection(&state);
    after_off.updated_at = 60;
    after_off.recording_error = frame.recording_error;
    apply_platform_archive_status(&mut state, &after_off);
    assert!(
        state.platform_archive_status.is_none(),
        "closed recording nonce cannot be resurrected"
    );
}
