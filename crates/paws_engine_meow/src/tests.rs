use crate::management::MeowManagement;
use paws_engine::{EngineManagement, EngineServicePlan};
use std::{collections::HashMap, sync::Arc};

fn new_management() -> Arc<MeowManagement> {
    let resolver = Arc::new(meow_dns::Resolver::new(
        Vec::new(),
        Vec::new(),
        meow_common::DnsMode::Normal,
        meow_trie::DomainTrie::new(),
        true,
        false,
    ));
    Arc::new(MeowManagement::new(
        meow_tunnel::Tunnel::new(resolver),
        paws_vpn::TunSession::default(),
        HashMap::new(),
        HashMap::new(),
    ))
}

#[tokio::test]
async fn closed_short_connections_are_captured_without_any_query_and_keep_their_owner() {
    let first = new_management();
    first.activate().await.unwrap();
    let statistics = first.tunnel.statistics();
    let id = statistics.track_connection(
        meow_common::Metadata::default(),
        "MATCH".into(),
        "".into(),
        Default::default(),
    );
    let counters = statistics.connection_counters(id).unwrap();
    statistics.record_upload(&counters, 19);
    statistics.close_connection(id);
    statistics.close_connection(id); // Native/manual close and guard Drop may both run.
    let page = first.connection_events(0, 10).await.unwrap();
    assert_eq!(page.entries.len(), 2);
    assert!(!page.entries[0].closed);
    assert!(page.entries[1].closed);
    assert_eq!(page.entries[1].connection.upload_bytes, 19);
    assert_eq!(page.high_watermark, 2);
    assert!(first.connections().await.unwrap().is_empty());

    let late = statistics.track_connection(
        meow_common::Metadata::default(),
        "MATCH".into(),
        "".into(),
        Default::default(),
    );
    first.revoke().await;
    let replacement = Arc::new(crate::management::ConnectionRecorder::new());
    statistics.set_connection_observer(Some(replacement.clone()));
    statistics.close_connection(late);
    assert_eq!(replacement.events.revision().unwrap(), 0);
    assert!(matches!(
        first.connection_events(0, 10).await,
        Err(paws_engine::EngineError::NotRunning)
    ));
}

#[tokio::test]
async fn private_observations_are_authenticated_and_stop_revokes_retained_clients() {
    let management = new_management();
    management.activate().await.unwrap();
    management
        .logs
        .record(paws_model::LogEntry {
            level: "info".into(),
            message: "native meow event".into(),
            timestamp: "1".into(),
        })
        .unwrap();
    let controller = crate::controller::PrivateController::start(management.clone())
        .await
        .unwrap();
    let endpoint = &controller.endpoint;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!(
        "http://{}/paws/observations/logs?after=0&limit=1",
        endpoint.address
    );
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let page: paws_engine::EngineLogPage = client
        .get(&url)
        .bearer_auth(endpoint.secret.as_ref().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.entries[0].message, "native meow event");
    assert_eq!(page.high_watermark, 1);
    management.revoke().await;
    assert!(!client
        .get(&url)
        .bearer_auth(endpoint.secret.as_ref().unwrap())
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    controller.stop().await;
}

#[tokio::test]
async fn private_controller_rotates_credentials_and_closes_its_port() {
    let management = new_management();
    management.activate().await.unwrap();
    let first = crate::controller::PrivateController::start(management.clone())
        .await
        .unwrap();
    let endpoint = first.endpoint.clone();
    let url = format!("http://{}/connections", endpoint.address);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(client
        .get(&url)
        .bearer_auth(endpoint.secret.as_ref().unwrap())
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    management.revoke().await;
    // The client may reuse an accepted keepalive connection after revocation.
    assert!(!client
        .delete(&url)
        .bearer_auth(endpoint.secret.as_ref().unwrap())
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    first.stop().await;
    assert!(tokio::net::TcpStream::connect(endpoint.address)
        .await
        .is_err());

    let replacement_management = new_management();
    replacement_management.activate().await.unwrap();
    let replacement = crate::controller::PrivateController::start(replacement_management)
        .await
        .unwrap();
    assert_ne!(endpoint.secret, replacement.endpoint.secret);
    let replacement_url = format!("http://{}/connections", replacement.endpoint.address);
    assert_eq!(
        client
            .delete(&replacement_url)
            .bearer_auth(endpoint.secret.as_ref().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(matches!(
        management.connections().await,
        Err(paws_engine::EngineError::NotRunning)
    ));
    replacement.stop().await;
}

#[tokio::test]
async fn occupied_mixed_port_is_not_reported_as_a_ready_listener_and_cleanup_closes_private_ports()
{
    let occupied = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let management = new_management();
    management.activate().await.unwrap();
    let plan = EngineServicePlan {
        enabled: true,
        network_protected: true,
        network: paws_engine::EngineNetworkServices {
            ports: paws_model::NetworkPortConfig {
                mixed_enabled: true,
                mixed_port: occupied.local_addr().unwrap().port(),
                ..Default::default()
            },
            ..Default::default()
        },
        benchmark_port: 0,
    };
    assert!(crate::services::MeowServices::start(
        management.tunnel.clone(),
        &plan,
        management.clone(),
        Arc::new(meow_common::AuthConfig::default()),
    )
    .await
    .is_err());
    let plan = EngineServicePlan {
        network: Default::default(),
        ..plan
    };
    let services = crate::services::MeowServices::start(
        management.tunnel.clone(),
        &plan,
        management.clone(),
        Arc::new(meow_common::AuthConfig::default()),
    )
    .await
    .unwrap();
    let snapshot = services.snapshot();
    assert!(!snapshot.mixed_ready);
    assert!(snapshot.warnings.is_empty());
    let controller = snapshot.controller_endpoint.unwrap();
    let proxy = snapshot.app_http_proxy.unwrap();
    assert!(tokio::net::TcpStream::connect(controller.address)
        .await
        .is_ok());
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy.port))
            .await
            .is_ok()
    );
    management.revoke().await;
    services.stop().await;
    assert!(tokio::net::TcpStream::connect(controller.address)
        .await
        .is_err());
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy.port))
            .await
            .is_err()
    );
    assert!(
        tokio::net::TcpStream::connect(occupied.local_addr().unwrap())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn unprotected_service_start_fails_before_binding_any_endpoint() {
    let management = new_management();
    let result = crate::services::MeowServices::start(
        management.tunnel.clone(),
        &EngineServicePlan {
            enabled: true,
            network_protected: false,
            network: Default::default(),
            benchmark_port: 0,
        },
        management,
        Arc::new(meow_common::AuthConfig::default()),
    )
    .await;
    assert!(matches!(
        result,
        Err(paws_engine::EngineError::Backend {
            operation: "listeners",
            ..
        })
    ));
}

#[tokio::test]
async fn selection_port_changes_the_actual_meow_group_and_reports_current_state() {
    let management = new_management();
    let config = meow_config::load_config_from_str("proxy-groups:\n  - name: group\n    type: select\n    proxies: [DIRECT, REJECT]\nrules:\n  - MATCH,group\n").await.unwrap();
    management
        .tunnel
        .update_routing(config.proxies, config.rules, config.dialer_registry);
    management.activate().await.unwrap();
    management
        .select_proxy("group", Some("REJECT"))
        .await
        .unwrap();
    assert_eq!(
        management
            .proxy_selection("group")
            .await
            .unwrap()
            .selected
            .as_deref(),
        Some("REJECT")
    );
    assert!(management
        .select_proxy("group", Some("missing"))
        .await
        .is_err());
    assert!(
        management.select_proxy("group", None).await.is_err(),
        "manual selectors cannot be unfixed"
    );
    management.revoke().await;
    assert!(matches!(
        management.select_proxy("group", Some("DIRECT")).await,
        Err(paws_engine::EngineError::NotRunning)
    ));
}

#[tokio::test]
async fn management_revoke_cancels_a_real_hanging_provider_request_before_draining_its_lease() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (waiting, observed) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = first.read(&mut request).await.unwrap();
        let body = "proxies:\n  - {name: provider-direct, type: direct}\n";
        first
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        drop(first);
        let (mut hanging, _) = listener.accept().await.unwrap();
        let _ = hanging.read(&mut request).await.unwrap();
        let _ = waiting.send(());
        // Actual socket read stays pending until the provider's cancelled
        // acquisition drops it. No mock management/projection is involved.
        let mut byte = [0_u8; 1];
        let _ = hanging.read(&mut byte).await;
    });
    let raw = meow_config::parse_raw_yaml(&format!(
        "proxy-providers:\n  p:\n    type: http\n    url: http://127.0.0.1:{port}/proxies\n    interval: 0\nproxy-groups:\n  - name: main\n    type: select\n    use: [p]\nrules: [\"MATCH,main\"]\n"
    )).unwrap();
    let config = meow_config::build_config(raw, None).await.unwrap();
    let resolver = config.dns.resolver.clone();
    let management = Arc::new(MeowManagement::new(
        meow_tunnel::Tunnel::new(resolver),
        paws_vpn::TunSession::default(),
        config.proxy_providers,
        config.rule_providers,
    ));
    management.activate().await.unwrap();
    let owner = management.clone();
    let refreshing = tokio::spawn(async move { owner.refresh_provider("proxies", "p").await });
    tokio::time::timeout(std::time::Duration::from_secs(2), observed)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), management.revoke())
        .await
        .expect("revoke waited for the network timeout instead of cancelling");
    assert!(refreshing.await.unwrap().is_err());
    assert!(matches!(
        management.runtime_revisions().await,
        Err(paws_engine::EngineError::NotRunning)
    ));
    assert!(management.refresh_provider("proxies", "p").await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cache_only_candidate_preserves_sources_without_http_or_dns_bootstrap() {
    let http = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let http_port = http.local_addr().unwrap().port();
    let dns = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let dns_port = dns.local_addr().unwrap().port();
    let cache = std::env::temp_dir().join(format!(
        "paws-meow-cache-policy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    tokio::fs::create_dir_all(&cache).await.unwrap();
    tokio::fs::write(
        cache.join("proxy.yaml"),
        "proxies:\n  - {name: cached-socks, type: socks5, server: 127.0.0.1, port: 1080}\n",
    )
    .await
    .unwrap();
    tokio::fs::write(cache.join("rules.yaml"), "payload: [example.com]\n")
        .await
        .unwrap();
    let raw = meow_config::parse_raw_yaml(&format!(
        "proxy-providers:\n  p:\n    type: http\n    url: http://127.0.0.1:{http_port}/proxies\n    path: proxy.yaml\nrule-providers:\n  r:\n    type: http\n    behavior: domain\n    url: http://127.0.0.1:{http_port}/rules\n    path: rules.yaml\nproxy-groups:\n  - {{name: main, type: select, use: [p]}}\ndns:\n  enable: true\n  nameserver: [https://resolver.invalid/dns-query]\n  proxy-server-nameserver: [https://proxy-resolver.invalid/dns-query]\n  default-nameserver: ['127.0.0.1:{dns_port}']\n  nameserver-policy:\n    '+.example.com': https://policy-resolver.invalid/dns-query\nrules: [\"RULE-SET,r,main\", \"MATCH,main\"]\n"
    )).unwrap();
    let loaded = meow_config::build_config_with_options(
        raw,
        Some(&cache),
        meow_config::ConfigLoadOptions {
            provider_network: meow_config::ProviderNetworkPolicy::CacheOnly,
        },
    )
    .await
    .unwrap();
    assert_eq!(loaded.proxy_providers["p"].vehicle_type, "HTTP");
    assert_eq!(
        loaded.proxy_providers["p"].proxies()[0].name(),
        "cached-socks"
    );
    assert_eq!(loaded.rule_providers["r"].rule_count(), 1);
    assert_eq!(
        loaded.raw.proxy_providers.as_ref().unwrap()["p"]
            .url
            .as_deref(),
        Some(format!("http://127.0.0.1:{http_port}/proxies").as_str())
    );
    assert!(loaded.proxy_providers["p"].refresh().await.is_err());
    assert!(loaded.rule_providers["r"].refresh().await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), http.accept())
            .await
            .is_err()
    );
    let mut bytes = [0_u8; 512];
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), dns.recv(&mut bytes))
            .await
            .is_err()
    );
    tokio::fs::remove_dir_all(cache).await.unwrap();
}
