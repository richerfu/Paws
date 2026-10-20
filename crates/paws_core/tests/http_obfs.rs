use base64::Engine;
use meow_transport::simple_obfs::client::HttpObfs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn request_uses_mihomo_header_order_and_websocket_key() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let mut transport = HttpObfs::new(client, "cover.example".into(), 8080).unwrap();
    transport.write_all(b"encrypted-payload").await.unwrap();
    transport.shutdown().await.unwrap();
    let mut request = String::new();
    peer.read_to_string(&mut request).await.unwrap();
    let (headers, payload) = request.split_once("\r\n\r\n").unwrap();
    let fields: Vec<_> = headers.lines().skip(1).collect();
    let names: Vec<_> = fields
        .iter()
        .map(|field| field.split_once(':').unwrap().0)
        .collect();
    assert_eq!(
        names,
        [
            "Host",
            "User-Agent",
            "Content-Length",
            "Connection",
            "Sec-Websocket-Key",
            "Upgrade",
        ]
    );
    assert_eq!(fields[0], "Host: cover.example:8080");
    assert_eq!(fields[2], "Content-Length: 17");
    let key = fields[4].split_once(": ").unwrap().1;
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE
            .decode(key)
            .unwrap()
            .len(),
        16
    );
    assert_eq!(payload, "encrypted-payload");
}

#[tokio::test]
async fn fragmented_headers_and_header_only_packet_do_not_report_eof() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let mut transport = HttpObfs::new(client, "cover.example".into(), 443).unwrap();
    let sender = tokio::spawn(async move {
        peer.write_all(b"HTTP/1.1 101 Switching").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        peer.write_all(b" Protocols\r\nConnection: Upgrade\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        peer.write_all(b"encrypted-response").await.unwrap();
    });
    let mut response = Vec::new();
    transport.read_to_end(&mut response).await.unwrap();
    sender.await.unwrap();
    assert_eq!(response, b"encrypted-response");
}

#[tokio::test]
async fn truncated_response_headers_are_an_error() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let mut transport = HttpObfs::new(client, "cover.example".into(), 443).unwrap();
    peer.write_all(b"HTTP/1.1 101 Switching Protocols\r\n")
        .await
        .unwrap();
    peer.shutdown().await.unwrap();
    let error = transport.read(&mut [0; 1]).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
}
