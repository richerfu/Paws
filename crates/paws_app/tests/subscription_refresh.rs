use paws_app::{AppRequest, Application};
use serde_json::json;
use std::os::{fd::AsRawFd, unix::net::UnixDatagram};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PROFILE: &str = "proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\ndns:\n  nameserver: [223.5.5.5]\n  nameserver-policy: {}\npaws:\n  mixed-enabled: false\n  controller-enabled: false\n";

// This binary owns a separate process-wide Core and a native VPN over a socket
// pair. It exercises real session ownership without changing any OS routes.
#[tokio::test]
async fn subscription_commands_restart_only_for_changed_active_configuration() {
    let root = std::env::temp_dir().join(format!("paws-sub-refresh-{}", std::process::id()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/subscription", listener.local_addr().unwrap());
    // Host Core has no Harmony IPC and therefore no private engine ingress.
    // This HTTP fixture also stands in for that loopback app proxy.
    let profile_content = PROFILE.replace(
        "mixed-enabled: false",
        &format!(
            "mixed-enabled: true\n  mixed-port: {}",
            listener.local_addr().unwrap().port()
        ),
    );
    let body = Arc::new(Mutex::new(profile_content.clone()));
    let failing = Arc::new(AtomicBool::new(false));
    let server_failing = failing.clone();
    let served = body.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            let body = served.lock().unwrap().clone();
            let status = if server_failing.load(Ordering::Acquire) {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nSubscription-Userinfo: upload=100; download=200; total=1000\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let mut store = paws_profile::ProfileStore::open(root.clone()).unwrap();
    let profile = store
        .import_profile_content_with_subscription_metadata(
            "active",
            &url,
            &profile_content,
            Some(url.clone()),
            None,
            Some(paws_model::SubscriptionMetadata {
                update_interval_hours: Some(1),
                ..Default::default()
            }),
        )
        .unwrap();
    store.set_active(&profile).unwrap();
    let inactive = store
        .import_profile_content("inactive", &url, &profile_content, Some(url.clone()))
        .unwrap();
    // Make the active subscription due before Core loads its store, so the
    // bootstrap case performs a real HTTP refresh instead of skipping it.
    let index = root.join("profiles.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
    for entry in saved["profiles"].as_array_mut().unwrap() {
        if entry["id"] == profile {
            entry["updatedAt"] = json!("0");
        }
    }
    std::fs::write(&index, serde_json::to_vec(&saved).unwrap()).unwrap();
    paws_core::configure_app_home(&root).unwrap();
    let app = Application::shared();
    let core = paws_core::shared_core();
    core.prepare_active_vpn().await.unwrap();
    let intent = core.advance_platform_vpn_intent().unwrap();
    let owner = core.begin_platform_vpn_start_for_intent(intent).unwrap();
    core.bind_platform_vpn_start(&owner).unwrap();
    let (tun, _peer) = UnixDatagram::pair().unwrap();
    let mut options = core.command_context().unwrap().vpn_options;
    options.engine = paws_model::VpnEngine::Meow;
    let options = serde_json::to_string(&options).unwrap();
    let plan = core.prepare_engine(&options, &owner).await.unwrap();
    core.start_engine(tun.as_raw_fd(), 0, &options, &owner, &plan)
        .await
        .unwrap();
    assert_eq!(
        core.external_http_route().unwrap().0.as_deref(),
        Some(url.trim_end_matches("/subscription"))
    );

    for (id, command) in [
        (1, "bootstrap"),
        (2, "profiles.refresh"),
        (3, "profiles.refreshAll"),
    ] {
        let before = core.command_context().unwrap().config_revision;
        let reply = app
            .dispatch(AppRequest {
                id,
                command: command.into(),
                payload: if command == "profiles.refresh" {
                    json!({"profileId":profile})
                } else {
                    json!({})
                },
            })
            .await;
        assert!(reply.ok, "{command}: {:?}", reply.error);
        assert!(
            reply.effect.is_none(),
            "{command} restarted an unchanged subscription"
        );
        assert!(
            core.command_context().unwrap().config_revision > before,
            "{command} must really refresh metadata"
        );
        assert_eq!(core.current_platform_vpn_session_id().unwrap(), owner);
    }

    // Comments, mapping order and YAML formatting do not change the engine.
    *body.lock().unwrap() = format!(
        "# subscription-userinfo: upload=10; download=20\n{}",
        profile_content.replace("rules: ['MATCH,DIRECT']\n", "")
    ) + "rules:\n  - MATCH,DIRECT\n";
    let reply = app
        .dispatch(AppRequest {
            id: 4,
            command: "profiles.refresh".into(),
            payload: json!({"profileId":profile}),
        })
        .await;
    assert!(reply.ok, "{:?}", reply.error);
    assert!(reply.effect.is_none());

    failing.store(true, Ordering::Release);
    for command in ["profiles.refresh", "profiles.refreshAll"] {
        let reply = app
            .dispatch(AppRequest {
                id: 7,
                command: command.into(),
                payload: if command == "profiles.refresh" {
                    json!({"profileId":profile})
                } else {
                    json!({})
                },
            })
            .await;
        assert!(!reply.ok, "server failure must be reported");
        assert!(
            reply.effect.is_none(),
            "failed download cannot restart the VPN"
        );
        assert_eq!(core.current_platform_vpn_session_id().unwrap(), owner);
    }
    failing.store(false, Ordering::Release);

    *body.lock().unwrap() = profile_content.replace("MATCH,DIRECT", "MATCH,REJECT");
    let reply = app
        .dispatch(AppRequest {
            id: 5,
            command: "profiles.refresh".into(),
            payload: json!({"profileId":inactive}),
        })
        .await;
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        reply.effect.is_none(),
        "inactive changes must preserve the VPN"
    );

    let reply = app
        .dispatch(AppRequest {
            id: 6,
            command: "profiles.refreshAll".into(),
            payload: json!({}),
        })
        .await;
    assert!(reply.ok, "{:?}", reply.error);
    let effect = reply
        .effect
        .expect("changed active rules still require reconfiguration");
    assert_eq!(effect.kind, "restart");
    assert_eq!(effect.owner, owner);
    assert_eq!(
        effect.config_revision,
        core.command_context().unwrap().config_revision.to_string()
    );
    server.abort();
    core.stop_platform_vpn(&owner).await.unwrap();
    core.complete_platform_vpn_cleanup(&owner).unwrap();
    app.close();
    let _ = std::fs::remove_dir_all(root);
}
