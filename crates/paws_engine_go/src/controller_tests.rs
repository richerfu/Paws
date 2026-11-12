use super::*;
use paws_engine::{EngineControllerEndpoint, EngineManagement};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(
    replies: Vec<(u16, &'static str)>,
    secret: &str,
) -> (
    EngineControllerEndpoint,
    tokio::task::JoinHandle<Vec<String>>,
) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let secret = secret.to_owned();
    let endpoint_secret = secret.clone();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..count]);
                if data.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&data).into_owned();
            let authorized = request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {secret}").to_ascii_lowercase());
            let (status, body) = if authorized {
                (status, body)
            } else {
                (401, "{}")
            };
            stream.write_all(format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            requests.push(request);
        }
        requests
    });
    (
        EngineControllerEndpoint {
            address,
            secret: Some(endpoint_secret),
        },
        task,
    )
}
#[tokio::test]
async fn real_controller_mapping_preserves_native_identity_and_rejects_bad_schema() {
    let (mut endpoint,task)=server(vec![(200,r#"{"connections":[{"id":"native-1","metadata":{"host":"example.test","destinationIP":"203.0.113.9","destinationPort":"443","network":"tcp"},"upload":19,"download":37,"chains":["node-a","proxy"],"rule":"DomainSuffix","rulePayload":"test","start":"2026-10-21T00:00:00Z"}]}"#),(200,"{}")],"current").await;
    endpoint.secret = Some("current".into());
    let client = ControllerClient::new(VpnEngine::SingBox, endpoint).unwrap();
    let connections = client.connections().await.unwrap();
    assert_eq!(connections[0].id, "native-1");
    assert_eq!(connections[0].domain, "example.test");
    assert_eq!(connections[0].destination_port, 443);
    assert_eq!(connections[0].upload_bytes, 19);
    assert_eq!(connections[0].proxy, "node-a");
    assert!(client.connections().await.is_err());
    let requests = task.await.unwrap();
    assert!(requests
        .iter()
        .all(|request| request.starts_with("GET /connections ")));
}
#[tokio::test]
async fn obsolete_controller_credential_cannot_mutate_replacement_session() {
    let (mut endpoint, task) = server(vec![(204, ""), (204, "")], "new-owner").await;
    endpoint.secret = Some("old-owner".into());
    let obsolete = ControllerClient::new(VpnEngine::Mihomo, endpoint.clone()).unwrap();
    assert!(obsolete.close_connections(None).await.is_err());
    endpoint.secret = Some("new-owner".into());
    let current = ControllerClient::new(VpnEngine::Mihomo, endpoint).unwrap();
    current.close_connections(None).await.unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("old-owner"));
    assert!(requests[1].contains("new-owner"));
}
#[tokio::test]
async fn native_failure_and_missing_provider_delay_are_errors() {
    let (mut endpoint, task) = server(
        vec![(500, "{}"), (200, "{}"), (200, r#"{"delay":999999}"#)],
        "owner",
    )
    .await;
    endpoint.secret = Some("owner".into());
    let client = ControllerClient::new(VpnEngine::Mihomo, endpoint).unwrap();
    assert!(client.flush_dns(false).await.is_err());
    for _ in 0..2 {
        assert!(client
            .provider_health(
                "provider",
                Some("node"),
                Some("https://example.test"),
                std::time::Duration::from_secs(1),
                None
            )
            .await
            .is_err());
    }
    task.await.unwrap();
}
#[test]
fn native_provider_operations_are_described_and_credentials_are_redacted() {
    let endpoint = EngineControllerEndpoint {
        address: ([127, 0, 0, 1], 9090).into(),
        secret: Some("must-not-appear".into()),
    };
    assert!(!format!("{endpoint:?}").contains("must-not-appear"));
    assert!(ControllerClient::descriptor(VpnEngine::SingBox).supports(EngineCapability::Providers));
    assert!(
        ControllerClient::descriptor(VpnEngine::SingBox).supports(EngineCapability::DnsCacheFlush)
    );
}

#[tokio::test]
async fn sing_box_unfix_uses_the_native_authenticated_controller() {
    let (endpoint, task) = server(vec![(204, "")], "owner").await;
    let client = ControllerClient::new(VpnEngine::SingBox, endpoint).unwrap();
    client.select_proxy("auto", None).await.unwrap();
    let requests = task.await.unwrap();
    assert!(requests[0].starts_with("DELETE /proxies/auto "));
}

#[tokio::test]
async fn native_selection_and_unfix_use_the_authenticated_controller_and_propagate_failures() {
    let (endpoint, task) = server(vec![(204, ""), (500, "{}"), (204, "")], "owner").await;
    let client = ControllerClient::new(VpnEngine::Mihomo, endpoint).unwrap();
    client.select_proxy("a/b", Some("node")).await.unwrap();
    assert!(client.select_proxy("a/b", Some("bad-node")).await.is_err());
    client.select_proxy("a/b", None).await.unwrap();
    let requests = task.await.unwrap();
    assert!(requests[0].starts_with("PUT /proxies/a%2Fb "));
    assert!(requests[2].starts_with("DELETE /proxies/a%2Fb "));
    assert!(requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer owner")
    }));
}

#[test]
fn controller_credentials_are_injected_into_the_actual_backend_configuration() {
    for (engine, config) in [
        (
            VpnEngine::Mihomo,
            "external-controller: 127.0.0.1:9090\nsecret: user-secret\nlisteners: [{name: tinypaws-mihomo-benchmark, type: mixed, port: 18490, listen: 127.0.0.1}]\nrules: [MATCH,DIRECT]\n",
        ),
        (
            VpnEngine::SingBox,
            r#"{"experimental":{"clash_api":{"external_controller":"127.0.0.1:9090","secret":"user-secret"}},"inbounds":[{"type":"mixed","tag":"sing-box-benchmark","listen":"127.0.0.1","listen_port":18492}],"outbounds":[]}"#,
        ),
    ] {
        let first = config::secure_controller_config(engine, config, "attempt-a").unwrap();
        let first_endpoint = config::controller_endpoint(engine, &first).unwrap();
        let second = config::secure_controller_config(engine, config, "attempt-b").unwrap();
        let second_endpoint = config::controller_endpoint(engine, &second).unwrap();
        assert_eq!(first_endpoint.address, second_endpoint.address);
        assert_eq!(first_endpoint.secret.as_deref(), Some("attempt-a"));
        assert_eq!(second_endpoint.secret.as_deref(), Some("attempt-b"));
        assert!(!format!("{first_endpoint:?}").contains("attempt-a"));
    }
}
