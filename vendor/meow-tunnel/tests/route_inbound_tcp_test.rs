/// Integration tests for `route_inbound_tcp` — the shared blind-tunnel
/// routing tail extracted from SOCKS5 CONNECT / HTTP CONNECT / `handle_tcp`.
///
/// These tests verify the `prefix` parameter (pipelined bytes that the
/// listener already buffered ahead of the relay) is correctly forwarded
/// to the remote before the bidirectional copy begins, and that upload
/// counters reflect the prefix length.
use meow_common::{ConnType, Metadata, Network};
use meow_dns::Resolver;
use meow_trie::DomainTrie;
use meow_tunnel::{tcp::route_inbound_tcp, Tunnel};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Build a minimal `Tunnel` in `Direct` mode — every connection dials
/// the real destination IP directly, which is what we need for loopback
/// test servers.
fn direct_tunnel() -> Tunnel {
    let hosts = DomainTrie::new();
    let use_hosts = false;
    let ipv6 = false;
    let resolver = Arc::new(Resolver::new(
        vec![],
        vec![],
        meow_common::DnsMode::Normal,
        hosts,
        use_hosts,
        ipv6,
    ));
    let tunnel = Tunnel::new(resolver);
    tunnel.set_mode(meow_common::TunnelMode::Direct);
    tunnel
}

/// Spawn a local TCP server that reads exactly `expected_len` bytes,
/// echoes them back, then half-closes its write side. Returns the bound
/// address so the test can target it in `Metadata`.
async fn spawn_echo_server(expected_len: usize) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut conn, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0u8; expected_len];
        conn.read_exact(&mut buf).await.unwrap();
        conn.write_all(&buf).await.unwrap();
        // Half-close to signal EOF to the relay.
        conn.shutdown().await.unwrap();
    });
    addr
}

/// Bind a loopback listener, accept one connection, and return both halves.
async fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (accept_res, connect_res) = tokio::join!(listener.accept(), TcpStream::connect(addr));
    let (server, _) = accept_res.unwrap();
    let client = connect_res.unwrap();
    (server, client)
}

#[tokio::test]
async fn route_inbound_tcp_prefix_is_forwarded_to_remote() {
    // The prefix simulates pipelined bytes an HTTP CONNECT listener already
    // read past the 200 OK (e.g. a TLS ClientHello). The shared routing path
    // must re-emit them to the remote before the bidirectional copy.
    let prefix: Vec<u8> = b"PREFIX_DATA_HELLO".to_vec();
    let echo_addr = spawn_echo_server(prefix.len()).await;

    let tunnel = direct_tunnel();
    let (mut server_stream, mut client_stream) = loopback_pair().await;

    let metadata = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Https,
        dst_ip: Some(echo_addr.ip()),
        dst_port: echo_addr.port(),
        ..Default::default()
    };

    let inner = Arc::clone(tunnel.inner());
    let prefix_clone = prefix.clone();

    // Spawn route_inbound_tcp with the prefix.
    let handle = tokio::spawn(async move {
        route_inbound_tcp(&inner, &mut server_stream, metadata, &prefix_clone).await;
    });

    // The echo server should receive the prefix first, then echo it back.
    // We read it from the client side (the relay forwards the echo back).
    let mut received = vec![0u8; prefix.len()];
    client_stream
        .read_exact(&mut received)
        .await
        .expect("should receive echoed prefix");
    assert_eq!(
        received, prefix,
        "remote must receive the prefix bytes before the relay copy"
    );

    // Half-close the client side so the relay exits promptly instead of
    // waiting out RELAY_HALF_CLOSE_LINGER for more client data.
    client_stream.shutdown().await.unwrap();

    // Wait for the relay to finish (echo server half-closes → relay ends).
    let _ = handle.await;
}

#[tokio::test]
async fn route_inbound_tcp_empty_prefix_is_no_op() {
    // With an empty prefix (the SOCKS5 CONNECT case), route_inbound_tcp
    // should proceed straight to the bidirectional relay without any
    // prefix write.
    let echo_addr = spawn_echo_server(5).await;

    let tunnel = direct_tunnel();
    let (mut server_stream, mut client_stream) = loopback_pair().await;

    let metadata = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Socks5,
        dst_ip: Some(echo_addr.ip()),
        dst_port: echo_addr.port(),
        ..Default::default()
    };

    let inner = Arc::clone(tunnel.inner());

    let handle = tokio::spawn(async move {
        route_inbound_tcp(&inner, &mut server_stream, metadata, &[]).await;
    });

    // Send some data from the client side — the relay should forward it
    // to the echo server, which echoes it back.
    let payload = b"hello";
    client_stream.write_all(payload).await.unwrap();

    let mut received = vec![0u8; 5];
    client_stream
        .read_exact(&mut received)
        .await
        .expect("should receive echoed payload");
    assert_eq!(&received, payload, "relay should forward and echo data");

    // Half-close the client side so the relay exits promptly instead of
    // waiting out RELAY_HALF_CLOSE_LINGER for more client data.
    client_stream.shutdown().await.unwrap();

    let _ = handle.await;
}

#[tokio::test]
async fn route_inbound_tcp_prefix_counts_as_upload() {
    // The prefix bytes must be counted as upload in the connection stats
    // so that byte counters stay accurate.
    let prefix: Vec<u8> = vec![0x16, 0x03, 0x01, 0x00, 0x05]; // fake TLS ClientHello fragment
    let echo_addr = spawn_echo_server(prefix.len()).await;

    let tunnel = direct_tunnel();
    let stats = Arc::clone(tunnel.statistics());
    let (mut server_stream, mut client_stream) = loopback_pair().await;

    let metadata = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Https,
        dst_ip: Some(echo_addr.ip()),
        dst_port: echo_addr.port(),
        ..Default::default()
    };

    let inner = Arc::clone(tunnel.inner());
    let prefix_clone = prefix.clone();

    let handle = tokio::spawn(async move {
        route_inbound_tcp(&inner, &mut server_stream, metadata, &prefix_clone).await;
    });

    // Read the echoed prefix back so the relay can complete.
    let mut received = vec![0u8; prefix.len()];
    client_stream.read_exact(&mut received).await.unwrap();
    assert_eq!(received, prefix);

    // Half-close the client side so the relay exits promptly instead of
    // waiting out RELAY_HALF_CLOSE_LINGER for more client data.
    client_stream.shutdown().await.unwrap();

    let _ = handle.await;

    // After the relay completes, the upload counter should include at least
    // the prefix length (the echo server's response is counted as download).
    let (total_upload, _total_download) = stats.snapshot();
    assert!(
        total_upload >= prefix.len() as meow_common::atomic::Int,
        "upload counter ({total_upload}) should include prefix length ({}), \
         plus any relay upload",
        prefix.len()
    );
}

/// Issue #618 caller-level pin: `route_inbound_tcp` must honour the
/// `PreHandleVerdict::Drop` and return **before** rule matching or the
/// adapter dial. A counting GLOBAL adapter proves the dial never runs —
/// the stale fake IP under TUN `auto-route` would otherwise re-enter the
/// device and self-saturate `max-connections`.
#[tokio::test]
async fn route_inbound_tcp_drop_verdict_never_dials() {
    use meow_common::{
        AdapterType, DnsMode, MeowError, Proxy, ProxyAdapter, ProxyConn, ProxyPacketConn,
    };
    use meow_dns::fakeip::{MemoryStore, Pool};
    use meow_proxy::dialer::ProxyRegistry;
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingProxy {
        dials: Arc<AtomicUsize>,
        health: meow_common::ProxyHealth,
    }

    #[async_trait::async_trait]
    impl ProxyAdapter for CountingProxy {
        fn name(&self) -> &str {
            "counting"
        }
        fn health(&self) -> &meow_common::ProxyHealth {
            &self.health
        }
        fn adapter_type(&self) -> AdapterType {
            AdapterType::Direct
        }
        fn addr(&self) -> &str {
            "counting://local"
        }
        fn support_udp(&self) -> bool {
            false
        }
        async fn dial_tcp(&self, _m: &Metadata) -> meow_common::Result<Box<dyn ProxyConn>> {
            self.dials.fetch_add(1, Ordering::SeqCst);
            Err(MeowError::Other("counting proxy dial".into()))
        }
        async fn dial_udp(&self, _m: &Metadata) -> meow_common::Result<Box<dyn ProxyPacketConn>> {
            Err(MeowError::UdpNotSupported)
        }
    }

    impl Proxy for CountingProxy {
        fn alive(&self) -> bool {
            true
        }
        fn alive_for_url(&self, _url: &str) -> bool {
            true
        }
        fn last_delay(&self) -> u16 {
            0
        }
        fn last_delay_for_url(&self, _url: &str) -> u16 {
            0
        }
        fn delay_history(&self) -> Vec<meow_common::DelayHistory> {
            vec![]
        }
    }

    // Fake-IP resolver: 198.18.0.0/16 pool, nothing allocated.
    let mut resolver = Resolver::new(
        vec![],
        vec![],
        DnsMode::FakeIp,
        DomainTrie::new(),
        true,
        true,
    );
    resolver.set_fakeip_v4(Arc::new(
        Pool::new(
            "198.18.0.0/16".parse().unwrap(),
            Arc::new(MemoryStore::new(1024)),
        )
        .unwrap(),
    ));
    let tunnel = Tunnel::new(Arc::new(resolver));
    tunnel.set_mode(meow_common::TunnelMode::Global);
    let counter = Arc::new(AtomicUsize::new(0));
    tunnel.update_proxies(
        HashMap::from([(
            "GLOBAL".into(),
            Arc::new(CountingProxy {
                dials: Arc::clone(&counter),
                health: meow_common::ProxyHealth::default(),
            }) as Arc<dyn Proxy>,
        )]),
        ProxyRegistry::default(),
    );
    let inner = Arc::clone(tunnel.inner());

    // Stale fake IP — in range, never allocated → Drop → no dial.
    let (mut s, _c) = loopback_pair().await;
    let md = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Socks5,
        dst_ip: Some(IpAddr::from([198, 18, 0, 9])),
        dst_port: 443,
        ..Default::default()
    };
    route_inbound_tcp(&inner, &mut s, md, &[]).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "a Drop verdict must return before the adapter dial"
    );

    // Control: a real destination still reaches the dialer.
    let (mut s, _c) = loopback_pair().await;
    let md = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Socks5,
        dst_ip: Some(IpAddr::from([127, 0, 0, 1])),
        dst_port: 1,
        ..Default::default()
    };
    route_inbound_tcp(&inner, &mut s, md, &[]).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "a routable destination must still reach GLOBAL's dial_tcp"
    );
}
