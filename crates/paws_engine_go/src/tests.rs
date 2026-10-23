use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
struct ScriptedBridge {
    calls: Mutex<Vec<&'static str>>,
    fail_start: AtomicBool,
    fail_reload: AtomicBool,
    fail_stop: AtomicBool,
    fail_services: AtomicBool,
    fail_trim: AtomicBool,
    sample: Mutex<EngineTraffic>,
    start_barrier: Mutex<Option<Arc<tokio::sync::Notify>>>,
}
impl ScriptedBridge {
    fn call(&self, name: &'static str, fail: bool) -> Result<(), EngineError> {
        self.calls.lock().unwrap().push(name);
        if fail {
            Err(EngineError::InvalidConfig(format!(
                "scripted {name} failure"
            )))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl GoBridge for ScriptedBridge {
    async fn start(&self, _: &EngineStart) -> Result<(), EngineError> {
        self.call("start", self.fail_start.load(Ordering::Relaxed))?;
        let barrier = self.start_barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.notified().await;
        }
        Ok(())
    }
    async fn confirm_services(
        &self,
        _: &ControllerClient,
    ) -> Result<paws_engine::EngineServices, EngineError> {
        if self.fail_services.load(Ordering::Relaxed) {
            return Err(EngineError::InvalidConfig(
                "scripted acknowledgment failure".into(),
            ));
        }
        Ok(paws_engine::EngineServices::default())
    }
    fn stop(&self) -> Result<(), EngineError> {
        self.call("stop", self.fail_stop.load(Ordering::Relaxed))
    }
    async fn reload(&self, _: &EngineStart) -> Result<(), EngineError> {
        self.call("reload", self.fail_reload.load(Ordering::Relaxed))
    }
    fn traffic(&self) -> Result<EngineTraffic, EngineError> {
        Ok(self.sample.lock().unwrap().clone())
    }
    fn version(&self) -> Result<String, EngineError> {
        Ok("test".into())
    }
    fn runtime_log(&self) -> Result<String, EngineError> {
        Ok("test-log".into())
    }
    fn trim_memory(&self) -> Result<(), EngineError> {
        self.call("trim", self.fail_trim.load(Ordering::Relaxed))
    }
}
fn request(engine: VpnEngine) -> EngineStart {
    EngineStart {
        tun_fd: 9,
        underlying_network: 42,
        options: Default::default(),
        config: match engine {
            VpnEngine::Mihomo => EngineConfig::Mihomo {
                init_params: "{}".into(),
                setup_params: "{}".into(),
                controller_endpoint: paws_engine::EngineControllerEndpoint {
                    address: "127.0.0.1:19090".parse().unwrap(), secret: Some("test-owner".into()),
                },
            },
            _ => EngineConfig::SingBox {
                config_json: r#"{"experimental":{"clash_api":{"external_controller":"127.0.0.1:19090","secret":"test-owner"}}}"#.into(),
                work_dir: "/tmp/paws-test".into(),
            },
        },
    }
}

#[tokio::test]
async fn failed_start_cleans_resources_before_retry_and_never_reports_running() {
    let bridge = Arc::new(ScriptedBridge::default());
    bridge.fail_start.store(true, Ordering::Relaxed);
    let session = GoEngineSession::with_bridge(VpnEngine::SingBox, bridge.clone());
    assert!(session.start(request(VpnEngine::SingBox)).await.is_err());
    assert!(!session.is_running());
    assert!(matches!(session.traffic(), Err(EngineError::NotRunning)));
    assert_eq!(*bridge.calls.lock().unwrap(), ["start", "stop"]);
    bridge.fail_start.store(false, Ordering::Relaxed);
    session.start(request(VpnEngine::SingBox)).await.unwrap();
    session.stop().await.unwrap();
    session.stop().await.unwrap();
    assert_eq!(
        *bridge.calls.lock().unwrap(),
        ["start", "stop", "start", "stop"]
    );
}
#[tokio::test]
async fn failed_cleanup_blocks_new_descriptor_until_stop_really_completes() {
    let bridge = Arc::new(ScriptedBridge::default());
    bridge.fail_start.store(true, Ordering::Relaxed);
    bridge.fail_stop.store(true, Ordering::Relaxed);
    let session = GoEngineSession::with_bridge(VpnEngine::Mihomo, bridge.clone());
    assert!(matches!(
        session.start(request(VpnEngine::Mihomo)).await,
        Err(EngineError::Cleanup { .. })
    ));
    assert!(matches!(
        session.start(request(VpnEngine::Mihomo)).await,
        Err(EngineError::AlreadyRunning)
    ));
    assert!(!session.is_running());
    bridge.fail_stop.store(false, Ordering::Relaxed);
    session.stop().await.unwrap();
    bridge.fail_start.store(false, Ordering::Relaxed);
    session.start(request(VpnEngine::Mihomo)).await.unwrap();
    assert!(session.is_running());
}
#[tokio::test]
async fn reload_failure_tears_down_instead_of_acknowledging_stale_runtime() {
    let bridge = Arc::new(ScriptedBridge::default());
    let session = GoEngineSession::with_bridge(VpnEngine::SingBox, bridge.clone());
    let request = request(VpnEngine::SingBox);
    session.start(request.clone()).await.unwrap();
    bridge.fail_reload.store(true, Ordering::Relaxed);
    assert!(session.reload(request.config, 42).await.is_err());
    assert!(!session.is_running());
    assert!(matches!(session.traffic(), Err(EngineError::NotRunning)));
    assert_eq!(*bridge.calls.lock().unwrap(), ["start", "reload", "stop"]);
}
#[tokio::test]
async fn wrong_engine_and_missing_network_are_rejected_before_native_calls() {
    let bridge = Arc::new(ScriptedBridge::default());
    let session = GoEngineSession::with_bridge(VpnEngine::SingBox, bridge.clone());
    assert!(session.start(request(VpnEngine::Mihomo)).await.is_err());
    let mut invalid = request(VpnEngine::SingBox);
    invalid.underlying_network = 0;
    assert!(session.start(invalid).await.is_err());
    assert!(bridge.calls.lock().unwrap().is_empty());
    assert!(GoEngineSession::new(VpnEngine::Auto).is_err());
}
#[tokio::test]
async fn idle_sample_zeros_rates_and_preserves_native_totals() {
    let bridge = Arc::new(ScriptedBridge::default());
    let session = GoEngineSession::with_bridge(VpnEngine::Mihomo, bridge.clone());
    session.start(request(VpnEngine::Mihomo)).await.unwrap();
    *bridge.sample.lock().unwrap() = EngineTraffic {
        upload_speed: 120,
        download_speed: 340,
        upload_total: 5000,
        download_total: 9000,
    };
    assert_eq!(session.traffic().unwrap().upload_speed, 120);
    *bridge.sample.lock().unwrap() = EngineTraffic {
        upload_speed: 0,
        download_speed: 0,
        upload_total: 5000,
        download_total: 9000,
    };
    assert_eq!(
        session.traffic().unwrap(),
        EngineTraffic {
            upload_speed: 0,
            download_speed: 0,
            upload_total: 5000,
            download_total: 9000
        }
    );
}
#[test]
fn maintenance_capabilities_use_adapter_ports_and_preserve_backend_failures() {
    for engine in [VpnEngine::SingBox, VpnEngine::Mihomo] {
        let bridge = Arc::new(ScriptedBridge::default());
        let session = GoEngineSession::with_bridge(engine, bridge.clone());
        let capabilities = session.capabilities();
        assert!(capabilities.supports(EngineCapability::TrimMemory));
        assert!(capabilities.supports(EngineCapability::RuntimeLog));
        assert_eq!(capabilities.operations.len(), 16);
        session.trim_memory().unwrap();
        assert_eq!(session.runtime_log().unwrap(), "test-log");
        bridge.fail_trim.store(true, Ordering::Relaxed);
        assert!(session.trim_memory().is_err());
        assert_eq!(*bridge.calls.lock().unwrap(), ["trim", "trim"]);
    }
}

#[tokio::test]
async fn timed_out_setup_is_quarantined_until_stop_joins_late_completion() {
    let bridge = Arc::new(ScriptedBridge::default());
    let barrier = Arc::new(tokio::sync::Notify::new());
    *bridge.start_barrier.lock().unwrap() = Some(barrier.clone());
    let mut session = GoEngineSession::with_bridge(VpnEngine::Mihomo, bridge.clone());
    session.settle_timeout = std::time::Duration::from_millis(10);
    assert!(session.start(request(VpnEngine::Mihomo)).await.is_err());
    assert!(session.stop().await.is_err());
    assert!(matches!(
        session.start(request(VpnEngine::Mihomo)).await,
        Err(EngineError::AlreadyRunning)
    ));
    assert_eq!(*bridge.calls.lock().unwrap(), ["start"]);
    barrier.notify_one();
    session.stop().await.unwrap();
    assert_eq!(*bridge.calls.lock().unwrap(), ["start", "stop"]);
    *bridge.start_barrier.lock().unwrap() = None;
    session.start(request(VpnEngine::Mihomo)).await.unwrap();
    assert!(session.is_running());
}

#[tokio::test]
async fn native_service_ack_failure_never_publishes_running_and_crosses_cleanup_barrier() {
    let bridge = Arc::new(ScriptedBridge::default());
    bridge.fail_services.store(true, Ordering::Relaxed);
    let session = GoEngineSession::with_bridge(VpnEngine::SingBox, bridge.clone());
    assert!(session.start(request(VpnEngine::SingBox)).await.is_err());
    assert!(!session.is_running());
    assert!(session.services().app_http_proxy.is_none());
    assert_eq!(*bridge.calls.lock().unwrap(), ["start", "stop"]);
    bridge.fail_services.store(false, Ordering::Relaxed);
    session.start(request(VpnEngine::SingBox)).await.unwrap();
    bridge.fail_services.store(true, Ordering::Relaxed);
    assert!(session
        .reload(request(VpnEngine::SingBox).config, 42)
        .await
        .is_err());
    assert!(!session.is_running());
    assert!(session.services().controller_endpoint.is_none());
    assert_eq!(
        *bridge.calls.lock().unwrap(),
        ["start", "stop", "start", "reload", "stop"]
    );
}
