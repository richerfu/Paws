use paws_app::{AppRequest, Application};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;

fn request(id: u64, command: &str, payload: Value) -> AppRequest {
    AppRequest {
        id,
        command: command.into(),
        payload,
    }
}

async fn blocked_subscription() -> (
    String,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}/profile", listener.local_addr().unwrap());
    let received = Arc::new(tokio::sync::Notify::new());
    let entered = received.clone();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        entered.notify_one();
        // A cancelled preparation must drop its actual HTTP connection.
        assert_eq!(socket.read(&mut bytes).await.unwrap(), 0);
    });
    (url, received, task)
}

// This binary has its own process-wide Core and temp home; it cannot race the
// unrelated converter unit tests' global Core initialization.
#[tokio::test]
async fn real_import_cancellation_and_diagnostics_preserve_the_transaction_boundary() {
    let root = std::env::temp_dir().join(format!("paws-app-lifecycle-{}", std::process::id()));
    paws_core::configure_app_home(&root).unwrap();
    let app = Arc::new(Application::shared());
    let yaml = format!(
        "{}\npaws:\n  mixed-enabled: false\n  controller-enabled: false\n",
        paws_profile::default_runtime_yaml()
    );

    assert_eq!(
        app.dispatch(request(1, "profiles.cancelImport", json!({"intentId":8})))
            .await
            .data,
        Some(json!(true))
    );
    let cancelled = app
        .dispatch(request(
            2,
            "profiles.importContent",
            json!({"intentId":8,"name":"cancelled","content":yaml}),
        ))
        .await;
    assert!(!cancelled.ok);
    assert!(cancelled.effect.is_none());
    assert!(paws_core::shared_core()
        .config_projection()
        .unwrap()
        .profiles
        .is_empty());

    // Cancel crosses an in-flight command, rather than waiting behind prepare.
    let (url, received, server) = blocked_subscription().await;
    let worker = app.clone();
    let pending = tokio::spawn(async move {
        worker
            .dispatch(request(
                3,
                "profiles.importUrl",
                json!({"intentId":9,"url":url}),
            ))
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), received.notified())
        .await
        .unwrap();
    assert_eq!(
        app.dispatch(request(4, "profiles.cancelImport", json!({"intentId":9})))
            .await
            .data,
        Some(json!(true))
    );
    let cancelled = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(!cancelled.ok);
    assert!(cancelled.error.unwrap().contains("cancelled"));
    assert!(cancelled.effect.is_none());
    assert!(paws_core::shared_core()
        .config_projection()
        .unwrap()
        .profiles
        .is_empty());
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();

    let imported = app
        .dispatch(request(
            5,
            "profiles.importContent",
            json!({"intentId":10,"name":"committed","content":yaml}),
        ))
        .await;
    assert!(imported.ok, "{:?}", imported.error);
    let before = paws_core::shared_core().config_projection().unwrap();
    assert_eq!(before.profiles.len(), 1);
    assert!(before.active_profile.is_some());
    let proxies = paws_core::shared_core()
        .query_runtime_domain(paws_core::RuntimeDomainKey {
            domain: paws_core::RuntimeDomain::Proxies,
            group_key: None,
        })
        .unwrap();
    let proxies = serde_json::to_value(proxies).unwrap();
    assert!(proxies["data"]["proxyGroups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|group| group["name"] == "Proxy"));

    let validation = app
        .dispatch(request(16, "profiles.validate", json!({"content":yaml})))
        .await;
    assert!(validation.ok, "{:?}", validation.error);
    assert_eq!(validation.data, Some(Value::Null));
    let invalid = app
        .dispatch(request(
            17,
            "profiles.validate",
            json!({"content":"proxy-groups: ["}),
        ))
        .await;
    assert!(!invalid.ok);
    assert_eq!(
        paws_core::shared_core()
            .config_projection()
            .unwrap()
            .revisions
            .config_revision,
        before.revisions.config_revision
    );
    let decision = app
        .dispatch(request(
            18,
            "engines.decision",
            json!({"options":before.vpn_options}),
        ))
        .await;
    assert!(decision.ok, "{:?}", decision.error);
    assert!(decision.data.unwrap()["selected"].is_string());

    let watch =
        json!({"enabled":true,"group":"Proxy","url":"https://www.gstatic.com/generate_204"});
    assert!(
        app.dispatch(request(6, "diagnostics.watchSet", watch.clone()))
            .await
            .ok
    );
    assert_eq!(
        app.dispatch(request(7, "diagnostics.watchGet", json!({})))
            .await
            .data,
        Some(watch)
    );
    let invalid = app
        .dispatch(request(
            8,
            "diagnostics.watchSet",
            json!({"enabled":true,"group":"Missing","url":"https://www.gstatic.com/generate_204"}),
        ))
        .await;
    assert!(!invalid.ok);
    assert_eq!(
        app.dispatch(request(9, "diagnostics.watchGet", json!({})))
            .await
            .data
            .unwrap()["enabled"],
        true
    );
    assert!(
        app.dispatch(request(
            10,
            "diagnostics.watchSet",
            json!({"enabled":false,"group":"Proxy","url":"https://www.gstatic.com/generate_204"})
        ))
        .await
        .ok
    );
    assert_eq!(
        app.dispatch(request(11, "diagnostics.watchGet", json!({})))
            .await
            .data
            .unwrap()["enabled"],
        false
    );
    assert_eq!(
        app.dispatch(request(12, "diagnostics.healthGet", json!({})))
            .await
            .data,
        Some(Value::Null)
    );

    let dns = app.dispatch(request(13,"diagnostics.serviceDns",json!({"domains":"+.Example.COM, .example.net","group":"Proxy","expectedRevision":before.revisions.config_revision.to_string()}))).await;
    assert!(dns.ok, "{:?}", dns.error);
    let after = paws_core::shared_core().config_projection().unwrap();
    assert_eq!(
        after.vpn_options.dns_servers,
        before.vpn_options.dns_servers
    );
    assert_eq!(
        after.vpn_options.dns_fallbacks,
        before.vpn_options.dns_fallbacks
    );
    assert_eq!(after.vpn_options.mtu, before.vpn_options.mtu);
    for (domain, servers) in &before.vpn_options.dns_nameserver_policy {
        assert_eq!(
            after.vpn_options.dns_nameserver_policy.get(domain),
            Some(servers)
        );
    }
    assert_eq!(
        after.vpn_options.dns_nameserver_policy["+.example.com"],
        vec!["https://1.1.1.1/dns-query#Proxy"]
    );
    assert_eq!(
        after.vpn_options.dns_nameserver_policy["+.example.net"],
        vec!["https://1.1.1.1/dns-query#Proxy"]
    );
    let stale = app.dispatch(request(14,"diagnostics.serviceDns",json!({"domains":"stale.example.com","group":"Proxy","expectedRevision":before.revisions.config_revision.to_string()}))).await;
    assert!(!stale.ok);
    assert!(stale.effect.is_none());
    assert_eq!(
        paws_core::shared_core()
            .config_projection()
            .unwrap()
            .vpn_options,
        after.vpn_options
    );

    let rule = app.dispatch(request(19,"rules.add",json!({"matchKind":"domain","value":"manual.example.com","target":"DIRECT","expectedRevision":after.revisions.config_revision.to_string(),"connectionId":"previous-connection","disconnectAfterSave":true}))).await;
    assert!(rule.ok, "{:?}", rule.error);
    let data = rule.data.unwrap();
    assert_eq!(data["connectionCloseRequested"], true);
    assert!(
        data["connectionCloseError"].is_string(),
        "a stopped session must report a partial-success warning"
    );
    assert!(data["applied"]["mutation"]["ruleId"].is_string());
    assert!(paws_core::shared_core()
        .config_projection()
        .unwrap()
        .rules
        .iter()
        .any(|entry| entry.line == "DOMAIN,manual.example.com,DIRECT"));

    app.close();
    assert_eq!(
        paws_core::shared_core()
            .config_projection()
            .unwrap()
            .active_profile,
        after.active_profile
    );
    let closed = app
        .dispatch(request(
            15,
            "profiles.importContent",
            json!({"intentId":11,"name":"closed","content":yaml}),
        ))
        .await;
    assert!(!closed.ok);
    assert_eq!(
        paws_core::shared_core()
            .config_projection()
            .unwrap()
            .profiles
            .len(),
        1
    );

    // Disposal also cancels a real URL preparation owned by another gateway.
    let closing = Arc::new(Application::shared());
    let (url, received, server) = blocked_subscription().await;
    let worker = closing.clone();
    let pending = tokio::spawn(async move {
        worker
            .dispatch(request(
                1,
                "profiles.importUrl",
                json!({"intentId":1,"url":url}),
            ))
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), received.notified())
        .await
        .unwrap();
    closing.close();
    let result = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(!result.ok);
    assert!(result.effect.is_none());
    assert_eq!(
        paws_core::shared_core()
            .config_projection()
            .unwrap()
            .profiles
            .len(),
        1
    );
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
