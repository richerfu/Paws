use super::*;
use std::collections::HashMap;

fn request(method: Method, uri: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[test]
fn public_operation_table_cannot_reach_private_encoded_or_config_write_routes() {
    for path in [
        "/paws/observations/logs",
        "/%70aws/observations/logs",
        "/proxies/../paws",
        "/proxies/%2e%2e",
        "/proxies/%252e%252e",
        "/proxies/%00",
        "/proxies/name/echo",
        "/configs/path",
        "/configs?token=private",
    ] {
        assert!(
            PublicRequest::parse(&request(Method::GET, path)).is_err(),
            "{path}"
        );
    }
    for method in [Method::PUT, Method::PATCH, Method::POST, Method::DELETE] {
        assert!(PublicRequest::parse(&request(method, "/configs")).is_err());
    }
    assert!(
        PublicRequest::parse(&request(Method::GET, "/proxies/%E4%B8%AD%E6%96%87%20group")).is_ok()
    );
    assert!(PublicRequest::parse(&request(Method::GET, "/providers/rules/real-provider")).is_ok());
}

#[test]
fn query_token_only_authenticates_allowed_websocket_routes_and_never_rest() {
    assert!(PublicRequest::parse(&request(Method::GET, "/connections?token=user")).is_err());
    let mut ws = request(Method::GET, "/traffic?token=user&interval=250");
    ws.headers_mut().insert(
        header::UPGRADE,
        header::HeaderValue::from_static("websocket"),
    );
    let parsed = PublicRequest::parse(&ws).unwrap();
    assert!(parsed.authorized(&ws, Some("user")));
    assert!(!parsed.authorized(&ws, Some("private-secret")));
    ws.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Bearer wrong"),
    );
    assert!(!parsed.authorized(&ws, Some("user")));
    assert!(PublicRequest::parse(&request(Method::GET, "/logs?level=info&level=error")).is_err());
    let mut not_stream = request(Method::GET, "/configs?token=user");
    not_stream.headers_mut().insert(
        header::UPGRADE,
        header::HeaderValue::from_static("websocket"),
    );
    assert!(PublicRequest::parse(&not_stream).is_err());
}

#[test]
fn user_bearer_is_exact_and_duplicate_credentials_are_rejected() {
    let mut req = request(Method::GET, "/connections");
    let parsed = PublicRequest::parse(&req).unwrap();
    assert!(!parsed.authorized(&req, Some("user-token")));
    req.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Bearer user-token"),
    );
    assert!(parsed.authorized(&req, Some("user-token")));
    assert!(!parsed.authorized(&req, Some("user-token-suffix")));
    req.headers_mut().append(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Bearer user-token"),
    );
    assert!(!parsed.authorized(&req, Some("user-token")));
    assert!(parsed.authorized(&req, None));
}

#[test]
fn ordinary_and_websocket_json_share_the_same_encoded_byte_cap() {
    assert_eq!(
        encode_response(&json!({"value":7})).unwrap(),
        b"{\"value\":7}"
    );
    assert_eq!(
        encode_response(&json!({"value":"x".repeat(RESPONSE_LIMIT)})).unwrap_err(),
        StatusCode::BAD_GATEWAY
    );
    assert!(PublicRequest::parse(&request(Method::OPTIONS, "/connections/session-id")).is_ok());
    assert!(PublicRequest::parse(&request(Method::OPTIONS, "/cache/dns/flush")).is_ok());
}

#[test]
fn each_traffic_reader_samples_actual_monotonic_totals_without_a_shared_reset() {
    let now = Instant::now();
    let mut first = TrafficSampler::default();
    let mut second = TrafficSampler::default();
    assert_eq!(first.sample(now, (100, 200)), (0, 0));
    assert_eq!(second.sample(now, (100, 200)), (0, 0));
    assert_eq!(
        first.sample(now + Duration::from_secs(2), (140, 280)),
        (20, 40)
    );
    assert_eq!(
        second.sample(now + Duration::from_secs(4), (180, 360)),
        (20, 40)
    );
    assert_eq!(first.sample(now + Duration::from_secs(5), (0, 0)), (0, 0));
}

fn management() -> Arc<MeowManagement> {
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

async fn free_port() -> u16 {
    tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn network(port: u16, lan: bool) -> EngineNetworkServices {
    let mut network = EngineNetworkServices::default();
    network.ports.controller_enabled = true;
    network.ports.controller_port = port;
    if network.ports.mixed_port == port {
        network.ports.mixed_port = port.saturating_sub(1);
    }
    network.access.allow_lan = lan;
    network.access.secret = Some("public-user-token".to_owned());
    network
}

#[tokio::test]
async fn actual_public_controller_uses_user_auth_reads_real_connection_and_closes_it() {
    let management = management();
    management.activate().await.unwrap();
    let id = management.tunnel.statistics().track_connection(
        meow_common::Metadata::default(),
        "MATCH".into(),
        "".into(),
        Default::default(),
    );
    let counters = management
        .tunnel
        .statistics()
        .connection_counters(id)
        .unwrap();
    management.tunnel.statistics().record_upload(&counters, 37);
    let server = PublicController::start(management.clone(), network(free_port().await, false))
        .await
        .unwrap();
    assert!(server.address.ip().is_loopback());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}", server.address);
    assert_eq!(
        client
            .get(format!("{url}/connections"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("{url}/connections"))
            .bearer_auth("private-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let connections: Value = client
        .get(format!("{url}/connections"))
        .bearer_auth("public-user-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(connections["connections"][0]["id"], id.to_string());
    assert_eq!(connections["connections"][0]["upload"], 37);
    let config: Value = client
        .get(format!("{url}/configs"))
        .bearer_auth("public-user-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(config.get("secret").is_none());
    assert!(!config.to_string().contains("public-user-token"));
    assert_eq!(
        client
            .delete(format!("{url}/connections/{id}"))
            .bearer_auth("public-user-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(management.connections().await.unwrap().is_empty());
    for path in [
        "/paws/observations/logs",
        "/%70aws/observations/logs",
        "/proxies/%252e%252e",
    ] {
        assert!(!client
            .get(format!("{url}{path}"))
            .bearer_auth("public-user-token")
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    server.stop().await;
    management.revoke().await;
}

#[tokio::test]
async fn actual_lan_binding_and_public_bind_conflict_are_observable() {
    let management = management();
    management.activate().await.unwrap();
    let server = PublicController::start(management.clone(), network(free_port().await, true))
        .await
        .unwrap();
    assert!(server.address.ip().is_unspecified());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(client
        .get(format!(
            "http://127.0.0.1:{}/version",
            server.address.port()
        ))
        .bearer_auth("public-user-token")
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    assert!(
        PublicController::start(management.clone(), network(server.address.port(), true))
            .await
            .is_err()
    );
    let port = server.address.port();
    server.stop().await;
    let _released = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
        .await
        .unwrap();
    management.revoke().await;
}

#[tokio::test]
async fn actual_websocket_log_cursor_is_independent_and_stop_joins_upgraded_work() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let management = management();
    management.activate().await.unwrap();
    management
        .logs
        .record(paws_model::LogEntry {
            level: "info".into(),
            message: "before-connection".into(),
            timestamp: "1".into(),
        })
        .unwrap();
    let server = PublicController::start(management.clone(), network(free_port().await, false))
        .await
        .unwrap();
    let request = format!(
        "ws://{}/logs?token=public-user-token&level=info",
        server.address
    )
    .into_client_request()
    .unwrap();
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    management
        .logs
        .record(paws_model::LogEntry {
            level: "info".into(),
            message: "after-connection".into(),
            timestamp: "2".into(),
        })
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(message.to_text().unwrap().contains("after-connection"));
    assert!(!message.to_text().unwrap().contains("before-connection"));
    assert_eq!(
        management.runtime_logs(0, 10).await.unwrap().entries.len(),
        2
    );
    tokio::time::timeout(Duration::from_secs(3), server.stop())
        .await
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    assert!(!matches!(
        closed,
        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(_)))
    ));
    management.revoke().await;
}

#[tokio::test]
async fn actual_service_public_bind_failure_releases_other_started_listeners() {
    let management = management();
    management.activate().await.unwrap();
    let occupied = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let benchmark = free_port().await;
    let plan = paws_engine::EngineServicePlan {
        enabled: true,
        network_protected: true,
        network: network(occupied.local_addr().unwrap().port(), false),
        benchmark_port: benchmark,
    };
    let started = crate::services::MeowServices::start(
        management.tunnel.clone(),
        &plan,
        management.clone(),
        Arc::new(meow_common::AuthConfig::default()),
    )
    .await;
    assert!(started.is_err());
    let _benchmark_released = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, benchmark))
        .await
        .unwrap();
    management.revoke().await;
}

#[tokio::test]
async fn disabled_public_controller_never_reserves_the_configured_port() {
    let management = management();
    management.activate().await.unwrap();
    let occupied = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut network = network(occupied.local_addr().unwrap().port(), false);
    network.ports.controller_enabled = false;
    let plan = paws_engine::EngineServicePlan {
        enabled: true,
        network_protected: true,
        network,
        benchmark_port: free_port().await,
    };
    let services = crate::services::MeowServices::start(
        management.tunnel.clone(),
        &plan,
        management.clone(),
        Arc::new(meow_common::AuthConfig::default()),
    )
    .await
    .unwrap();
    assert!(services.snapshot().public_controller_addr.is_none());
    services.stop().await;
    management.revoke().await;
}

#[tokio::test]
async fn pre_revoked_watch_receiver_never_polls_an_actual_typed_mutation() {
    let management = management();
    management.activate().await.unwrap();
    let id = management.tunnel.statistics().track_connection(
        meow_common::Metadata::default(),
        "MATCH".into(),
        "".into(),
        Default::default(),
    );
    let server = PublicController::start(management.clone(), network(free_port().await, false))
        .await
        .unwrap();
    // Model a stop published just before execute subscribes. Keep the atomic
    // flag permissive so the level-triggered watch is the actual rejecting gate.
    server.state.cancelled.send_replace(true);
    let result = server
        .state
        .execute(async { management.close_connections(None).await.map_err(status) })
        .await;
    assert_eq!(result, Err(StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(
        management.connections().await.unwrap()[0].id,
        id.to_string()
    );
    server.stop().await;
    management.revoke().await;
}

#[tokio::test]
async fn public_mixed_listener_preserves_real_config_auth_separate_from_benchmark_nonce() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let management = management();
    management.activate().await.unwrap();
    let mut network = network(free_port().await, false);
    network.ports.controller_enabled = false;
    network.ports.mixed_enabled = true;
    network.ports.mixed_port = free_port().await;
    let mixed_port = network.ports.mixed_port;
    let plan = paws_engine::EngineServicePlan {
        enabled: true,
        network_protected: true,
        network,
        benchmark_port: free_port().await,
    };
    let auth = Arc::new(meow_common::AuthConfig::new(
        Arc::new(meow_common::Credentials::new(HashMap::from([(
            "qa-user".to_owned(),
            "qa-password".to_owned(),
        )]))),
        Vec::new(),
    ));
    let services = crate::services::MeowServices::start(
        management.tunnel.clone(),
        &plan,
        management.clone(),
        auth,
    )
    .await
    .unwrap();
    let mut missing = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, mixed_port))
        .await
        .unwrap();
    missing.write_all(&[5, 1, 0]).await.unwrap();
    let mut response = [0; 2];
    missing.read_exact(&mut response).await.unwrap();
    assert_eq!(response, [5, 255]);
    let mut present = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, mixed_port))
        .await
        .unwrap();
    present.write_all(&[5, 1, 2]).await.unwrap();
    present.read_exact(&mut response).await.unwrap();
    assert_eq!(response, [5, 2]);
    present
        .write_all(b"\x01\x07qa-user\x0bqa-password")
        .await
        .unwrap();
    present.read_exact(&mut response).await.unwrap();
    assert_eq!(response, [1, 0]);
    let benchmark = services.snapshot().benchmark_proxy.unwrap();
    assert!(benchmark.token != "qa-password");
    async fn benchmark_auth(
        port: u16,
        credentials: Option<(&str, &str)>,
    ) -> ([u8; 2], Option<[u8; 2]>) {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            stream
                .write_all(&[5, 1, if credentials.is_some() { 2 } else { 0 }])
                .await
                .unwrap();
            let mut method = [0; 2];
            stream.read_exact(&mut method).await.unwrap();
            let authentication = if let Some((username, password)) = credentials {
                if method == [5, 2] {
                    let mut frame = vec![1, u8::try_from(username.len()).unwrap()];
                    frame.extend_from_slice(username.as_bytes());
                    frame.push(u8::try_from(password.len()).unwrap());
                    frame.extend_from_slice(password.as_bytes());
                    stream.write_all(&frame).await.unwrap();
                    let mut status = [0; 2];
                    stream.read_exact(&mut status).await.unwrap();
                    Some(status)
                } else {
                    None
                }
            } else {
                None
            };
            (method, authentication)
        })
        .await
        .expect("actual benchmark authentication exceeded its deadline")
    }
    assert_eq!(benchmark_auth(benchmark.port, None).await, ([5, 255], None));
    assert_eq!(
        benchmark_auth(benchmark.port, Some(("qa-user", "qa-password"))).await,
        ([5, 2], Some([1, 1]))
    );
    assert_eq!(
        benchmark_auth(benchmark.port, Some(("paws-app", "qa-password"))).await,
        ([5, 2], Some([1, 1]))
    );
    assert_eq!(
        benchmark_auth(benchmark.port, Some(("paws-app", &benchmark.token))).await,
        ([5, 2], Some([1, 0]))
    );
    drop(present);
    drop(missing);
    services.stop().await;
    let rebound = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, benchmark.port))
        .await
        .expect("stopped actual benchmark listener must release its exact port");
    drop(rebound);
    management.revoke().await;
}
