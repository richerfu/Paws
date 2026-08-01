use super::*;
use meow_common::{DnsMode, MeowError, TunnelMode};
use meow_dns::Resolver;
use meow_trie::DomainTrie;
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::io::AsyncWriteExt;

fn tls_client_hello(hostname: &str) -> Vec<u8> {
    let hostname = hostname.as_bytes();
    let mut sni = Vec::new();
    sni.extend_from_slice(&[0x00, 0x00]);
    sni.extend_from_slice(&((5 + hostname.len()) as u16).to_be_bytes());
    sni.extend_from_slice(&((3 + hostname.len()) as u16).to_be_bytes());
    sni.push(0);
    sni.extend_from_slice(&(hostname.len() as u16).to_be_bytes());
    sni.extend_from_slice(hostname);

    let mut hello = Vec::new();
    hello.extend_from_slice(&[0x03, 0x03]);
    hello.extend_from_slice(&[0; 32]);
    hello.push(0);
    hello.extend_from_slice(&[0, 2, 0, 0x2f]);
    hello.extend_from_slice(&[1, 0]);
    hello.extend_from_slice(&(sni.len() as u16).to_be_bytes());
    hello.extend_from_slice(&sni);

    let mut handshake = vec![
        1,
        ((hello.len() >> 16) & 0xff) as u8,
        ((hello.len() >> 8) & 0xff) as u8,
        (hello.len() & 0xff) as u8,
    ];
    handshake.extend_from_slice(&hello);

    let mut record = vec![0x16, 0x03, 0x01];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

#[tokio::test]
async fn harmony_sniffer_extracts_tls_sni_and_replays_client_hello() {
    let sniffer = HarmonyTcpSniffer::from_config(SnifferConfig {
        enable: true,
        override_destination: true,
        tls_ports: vec![443],
        http_ports: Vec::new(),
        ..SnifferConfig::default()
    })
    .expect("enabled sniffer");
    let payload = tls_client_hello("tls.example.test");
    let (mut client, mut server) = tokio::io::duplex(SNIFF_BUFFER_SIZE * 2);
    client.write_all(&payload).await.unwrap();
    client.shutdown().await.unwrap();
    let mut metadata = Metadata {
        network: Network::Tcp,
        dst_ip: Some(Ipv4Addr::new(203, 0, 113, 1).into()),
        dst_port: 443,
        ..Metadata::default()
    };

    let prefix = sniffer.sniff(&mut server, &mut metadata).await;
    let mut replay = ReplayConn::new(server, prefix);
    let mut received = Vec::new();
    replay.read_to_end(&mut received).await.unwrap();

    assert_eq!(metadata.sniff_host, "tls.example.test");
    assert_eq!(metadata.host, "tls.example.test");
    assert_eq!(received, payload);
}

#[tokio::test]
async fn harmony_sniffer_extracts_http_host_without_overriding_destination() {
    let sniffer = HarmonyTcpSniffer::from_config(SnifferConfig {
        enable: true,
        override_destination: false,
        tls_ports: Vec::new(),
        http_ports: vec![80],
        ..SnifferConfig::default()
    })
    .expect("enabled sniffer");
    let payload = b"GET / HTTP/1.1\r\nHost: http.example.test\r\n\r\n".to_vec();
    let (mut client, mut server) = tokio::io::duplex(SNIFF_BUFFER_SIZE * 2);
    client.write_all(&payload).await.unwrap();
    client.shutdown().await.unwrap();
    let mut metadata = Metadata {
        network: Network::Tcp,
        dst_port: 80,
        host: "203.0.113.2".into(),
        ..Metadata::default()
    };

    let prefix = sniffer.sniff(&mut server, &mut metadata).await;
    let mut replay = ReplayConn::new(server, prefix);
    let mut received = Vec::new();
    replay.read_to_end(&mut received).await.unwrap();

    assert_eq!(metadata.sniff_host, "http.example.test");
    assert_eq!(metadata.host, "203.0.113.2");
    assert_eq!(received, payload);
}

struct NoopPacketConn;

#[async_trait::async_trait]
impl ProxyPacketConn for NoopPacketConn {
    async fn read_packet(&self, _buf: &mut [u8]) -> meow_common::Result<(usize, SocketAddr)> {
        Err(MeowError::Other("noop packet conn closed".to_owned()))
    }

    async fn write_packet(&self, buf: &[u8], _addr: &SocketAddr) -> meow_common::Result<usize> {
        Ok(buf.len())
    }

    fn local_addr(&self) -> meow_common::Result<SocketAddr> {
        Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
    }

    fn close(&self) -> meow_common::Result<()> {
        Ok(())
    }
}

enum ScriptedPacketRead {
    Packet(Vec<u8>, SocketAddr),
    Error(&'static str),
}

struct ScriptedPacketConn {
    reads: Mutex<VecDeque<ScriptedPacketRead>>,
}

impl ScriptedPacketConn {
    fn new(reads: Vec<ScriptedPacketRead>) -> Self {
        Self {
            reads: Mutex::new(reads.into()),
        }
    }
}

#[async_trait::async_trait]
impl ProxyPacketConn for ScriptedPacketConn {
    async fn read_packet(&self, buf: &mut [u8]) -> meow_common::Result<(usize, SocketAddr)> {
        let read = self
            .reads
            .lock()
            .expect("scripted reads lock")
            .pop_front()
            .unwrap_or(ScriptedPacketRead::Error("script exhausted"));
        match read {
            ScriptedPacketRead::Packet(data, addr) => {
                let size = data.len().min(buf.len());
                buf[..size].copy_from_slice(&data[..size]);
                Ok((size, addr))
            }
            ScriptedPacketRead::Error(message) => Err(MeowError::Other(message.to_owned())),
        }
    }

    async fn write_packet(&self, buf: &[u8], _addr: &SocketAddr) -> meow_common::Result<usize> {
        Ok(buf.len())
    }

    fn local_addr(&self) -> meow_common::Result<SocketAddr> {
        Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
    }

    fn close(&self) -> meow_common::Result<()> {
        Ok(())
    }
}

fn direct_mode_tunnel() -> Tunnel {
    let resolver = Arc::new(Resolver::new(
        Vec::new(),
        Vec::new(),
        DnsMode::Normal,
        DomainTrie::new(),
        true,
    ));
    let tunnel = Tunnel::new(resolver);
    tunnel.set_mode(TunnelMode::Direct);
    tunnel
}

#[test]
fn duplicate_fd_rejects_invalid_fd() {
    assert!(duplicate_fd(-1).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn lwip_stack_accepts_and_replies_to_tun_udp_packets() {
    let (mut stack, _tcp_listener, udp_socket) =
        lwip::NetStack::with_buffer_size(16, 16).expect("lwip stack");
    let (udp_write, mut udp_read) = udp_socket.split();
    let local = SocketAddr::new(Ipv4Addr::new(172, 19, 0, 1).into(), 40123);
    let remote = SocketAddr::new(Ipv4Addr::new(203, 0, 113, 42).into(), 443);
    let packet = build_udp_packet(
        u32::from_ne_bytes([172, 19, 0, 1]),
        local.port(),
        u32::from_ne_bytes([203, 0, 113, 42]),
        remote.port(),
        b"request",
    );

    stack.send(packet).await.expect("send packet into lwip");
    let (payload, src, dst) = tokio::time::timeout(Duration::from_secs(1), udp_read.next())
        .await
        .expect("lwip UDP receive timeout")
        .expect("lwip UDP receive");
    assert_eq!(payload, b"request");
    assert_eq!(src, local);
    assert_eq!(dst, remote);

    udp_write
        .send_to(b"response", &remote, &local)
        .expect("send lwip UDP response");
    let response = tokio::time::timeout(Duration::from_secs(1), stack.next())
        .await
        .expect("lwip egress timeout")
        .expect("lwip egress frame")
        .expect("lwip egress packet");
    let parsed = parse_udp_packet(&response).expect("parse lwip UDP response");
    assert_eq!(parsed.1, remote.port());
    assert_eq!(parsed.3, local.port());
    assert_eq!(parsed.4, b"response");
}

#[test]
fn udp_parser_classifies_ipv4_dns() {
    let packet = [
        0x45, 0, 0, 28, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 2, 1, 1, 1, 1, 0x12, 0x34, 0, 53, 0, 8,
        0, 0,
    ];
    let parsed = parse_udp_packet(&packet).expect("udp packet");
    assert_eq!(parsed.1, 0x1234);
    assert_eq!(parsed.3, 53);
}

#[test]
fn udp_parser_rejects_truncated_payload() {
    let mut packet = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    packet.truncate(packet.len() - 1);

    assert!(parse_udp_packet(&packet).is_none());
}

#[test]
fn udp_parser_rejects_invalid_ipv4_lengths() {
    let mut short_ihl = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    short_ihl[0] = 0x44;
    assert!(parse_udp_packet(&short_ihl).is_none());

    let mut short_total_len = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    short_total_len[2..4].copy_from_slice(&24_u16.to_be_bytes());
    assert!(parse_udp_packet(&short_total_len).is_none());

    let mut oversized_total_len = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    oversized_total_len[2..4].copy_from_slice(&128_u16.to_be_bytes());
    assert!(parse_udp_packet(&oversized_total_len).is_none());
}

#[test]
fn udp_parser_rejects_ipv4_fragments() {
    let mut more_fragments = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    more_fragments[6..8].copy_from_slice(&0x2000_u16.to_be_bytes());
    assert!(parse_udp_packet(&more_fragments).is_none());

    let mut fragment_offset = build_udp_packet(
        u32::from_ne_bytes([10, 0, 0, 2]),
        0x1234,
        u32::from_ne_bytes([1, 1, 1, 1]),
        53,
        b"abcd",
    );
    fragment_offset[6..8].copy_from_slice(&0x0001_u16.to_be_bytes());
    assert!(parse_udp_packet(&fragment_offset).is_none());
}

#[test]
fn udp_packet_builder_roundtrips_payload_and_endpoints() {
    let src_ip = u32::from_ne_bytes([172, 19, 0, 2]);
    let dst_ip = u32::from_ne_bytes([203, 0, 113, 42]);
    let payload = b"udp-session-payload";

    let packet = build_udp_packet(src_ip, 40123, dst_ip, 443, payload);
    let parsed = parse_udp_packet(&packet).expect("built UDP packet parses");

    assert_eq!(parsed.0, src_ip);
    assert_eq!(parsed.1, 40123);
    assert_eq!(parsed.2, dst_ip);
    assert_eq!(parsed.3, 443);
    assert_eq!(parsed.4, payload);
}

#[test]
fn dns_hijack_respects_vpn_option() {
    let packet = build_udp_packet(
        u32::from_ne_bytes([172, 19, 0, 2]),
        40123,
        u32::from_ne_bytes([172, 19, 0, 2]),
        53,
        b"dns-query",
    );

    let hijacked = tun_dns_query_from_packet(true, &packet).expect("dns packet");
    assert_eq!(hijacked.1, 40123);
    assert_eq!(hijacked.3, 53);
    assert_eq!(hijacked.4, b"dns-query");
    assert!(tun_dns_query_from_packet(false, &packet).is_none());

    let non_dns = build_udp_packet(
        u32::from_ne_bytes([172, 19, 0, 2]),
        40123,
        u32::from_ne_bytes([203, 0, 113, 42]),
        443,
        b"udp",
    );
    assert!(tun_dns_query_from_packet(true, &non_dns).is_none());
}

#[test]
fn dns_query_and_response_roundtrip() {
    let query = dns_query("example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    assert_eq!(parsed.name, "example.test");
    assert_eq!(parsed.kind, DnsRecordKind::A);
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(&response[0..2], &query[0..2]);
    assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    assert_eq!(&response[response.len() - 4..], &[1, 2, 3, 4]);
    assert_eq!(
        parse_dns_response_records(&response),
        vec![(
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
            "example.test".to_owned(),
            60
        )]
    );
}

#[test]
fn dns_aaaa_query_and_response_roundtrip() {
    let query = dns_query("ipv6.example.test", 28);
    let parsed = parse_dns_query(&query).expect("dns query");
    assert_eq!(parsed.name, "ipv6.example.test");
    assert_eq!(parsed.kind, DnsRecordKind::Aaaa);
    let ip = Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 42);
    let response = build_dns_response(&query, &parsed, IpAddr::V6(ip));
    assert_eq!(&response[0..2], &query[0..2]);
    assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    assert_eq!(
        parse_dns_response_records(&response),
        vec![(IpAddr::V6(ip), "ipv6.example.test".to_owned(), 60)]
    );
}

#[test]
fn dns_response_records_map_cname_addresses_to_original_question() {
    let query = dns_query("app.example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_cname_response(
        &query,
        &parsed,
        "edge.example.test",
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 88)),
    );

    assert_eq!(
        parse_dns_response_records(&response),
        vec![(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 88)),
            "app.example.test".to_owned(),
            60
        )]
    );
}

#[test]
fn dns_error_response_preserves_question() {
    let query = dns_query("bad.test", 1);
    let response = build_dns_error_response(&query, DnsResponseCode::ServFail);
    assert_eq!(&response[0..2], &query[0..2]);
    assert_eq!(response[3] & 0x0f, DnsResponseCode::ServFail as u8);
    assert_eq!(&response[12..], &query[12..]);
}

#[test]
fn dns_overflow_servfail_udp_packet_returns_to_client() {
    let query = dns_query("overflow.test", 1);
    let src_ip = u32::from_ne_bytes([172, 19, 0, 2]);
    let dst_ip = u32::from_ne_bytes([172, 19, 0, 1]);

    let packet =
        build_dns_servfail_udp_packet(src_ip, 40123, dst_ip, 53, &query).expect("servfail packet");
    let parsed = parse_udp_packet(&packet).expect("servfail UDP packet");

    assert_eq!(parsed.0, dst_ip);
    assert_eq!(parsed.1, 53);
    assert_eq!(parsed.2, src_ip);
    assert_eq!(parsed.3, 40123);
    assert_eq!(&parsed.4[0..2], &query[0..2]);
    assert_eq!(parsed.4[3] & 0x0f, DnsResponseCode::ServFail as u8);
    assert_eq!(&parsed.4[12..], &query[12..]);
}

#[test]
fn dns_overflow_servfail_udp_packet_ignores_invalid_queries() {
    assert!(build_dns_servfail_udp_packet(1, 53, 2, 40123, b"short dns").is_none());
}

#[test]
fn dns_table_restores_host() {
    let table = DnsTable::default();
    table.insert(
        IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
        "example.test".to_owned(),
        60,
    );
    assert_eq!(
        table.lookup(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
        Some("example.test".to_owned())
    );
}

#[test]
fn tcp_metadata_restores_host_from_dns_table() {
    let table = DnsTable::default();
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
    let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 20));
    let remote = SocketAddr::new(remote_ip, 443);
    table.insert(remote_ip, "tcp.example.test".to_owned(), 60);

    let metadata = tcp_metadata_for_stream(local, remote, &table);

    assert_eq!(metadata.network, Network::Tcp);
    assert_eq!(metadata.src_ip, Some(local.ip()));
    assert_eq!(metadata.src_port, local.port());
    assert_eq!(metadata.dst_ip, None);
    assert_eq!(metadata.dst_port, remote.port());
    assert_eq!(metadata.host, "tcp.example.test");
    assert_eq!(metadata.in_name, "hmeta-vpn");
}

#[test]
fn tcp_metadata_keeps_destination_ip_without_dns_table_host() {
    let table = DnsTable::default();
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
    let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 21));
    let remote = SocketAddr::new(remote_ip, 443);

    let metadata = tcp_metadata_for_stream(local, remote, &table);

    assert_eq!(metadata.network, Network::Tcp);
    assert_eq!(metadata.dst_ip, Some(remote_ip));
    assert_eq!(metadata.dst_port, remote.port());
    assert!(metadata.host.is_empty());
}

#[test]
fn udp_metadata_restores_host_from_dns_table() {
    let table = DnsTable::default();
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
    let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
    let remote = SocketAddr::new(remote_ip, 443);
    table.insert(remote_ip, "udp.example.test".to_owned(), 60);

    let metadata = udp_metadata_for_datagram(local, remote, &table);

    assert_eq!(metadata.network, Network::Udp);
    assert_eq!(metadata.src_ip, Some(local.ip()));
    assert_eq!(metadata.src_port, local.port());
    assert_eq!(metadata.dst_ip, Some(remote_ip));
    assert_eq!(metadata.dst_port, remote.port());
    assert_eq!(metadata.host, "udp.example.test");
    assert_eq!(metadata.in_name, "hmeta-vpn");
}

#[test]
fn udp_session_map_retains_touched_sessions_and_evicts_idle_ones() {
    let sessions = UdpSessionMap::default();
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
    let active_remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), 443);
    let idle_remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 11)), 443);
    let active_key = UdpSessionKey {
        local,
        remote: active_remote,
    };
    let idle_key = UdpSessionKey {
        local,
        remote: idle_remote,
    };
    let active_session = UdpTunSession::new(Arc::new(NoopPacketConn));
    let idle_session = UdpTunSession::new(Arc::new(NoopPacketConn));
    sessions.insert(active_key, active_session.clone());
    sessions.insert(idle_key, idle_session.clone());

    std::thread::sleep(Duration::from_millis(20));
    active_session.touch();
    idle_session
        .last_activity_ms
        .store(monotonic_ms().saturating_sub(10), Ordering::Relaxed);
    sessions.retain_active(Duration::from_millis(5));

    assert!(sessions.get(&active_key).is_some());
    assert!(sessions.get(&idle_key).is_none());
}

#[test]
fn udp_response_reader_forwards_packets_and_removes_session_on_read_error() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let sessions = UdpSessionMap::default();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
        let remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), 443);
        let key = UdpSessionKey { local, remote };
        let conn = Arc::new(ScriptedPacketConn::new(vec![
            ScriptedPacketRead::Packet(b"udp-reply".to_vec(), remote),
            ScriptedPacketRead::Error("udp conn closed"),
        ]));
        sessions.insert(key, UdpTunSession::new(conn.clone()));
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();

        read_udp_responses(key, local, conn, sessions.clone(), reply_tx).await;

        let reply = reply_rx.try_recv().expect("forwarded udp reply");
        assert_eq!(reply.data, b"udp-reply");
        assert_eq!(reply.local, local);
        assert_eq!(reply.remote, remote);
        assert!(sessions.get(&key).is_none());
    });
}

#[test]
fn udp_direct_session_forwards_echo_payloads() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let echo = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_addr = echo.local_addr().unwrap();
        let echo_task = tokio::spawn(async move {
            let mut buffer = [0_u8; 1024];
            for _ in 0..2 {
                let (size, peer) = echo.recv_from(&mut buffer).await.unwrap();
                let mut response = b"echo:".to_vec();
                response.extend_from_slice(&buffer[..size]);
                echo.send_to(&response, peer).await.unwrap();
            }
        });
        let tunnel = direct_mode_tunnel();
        let dns_table = DnsTable::default();
        let sessions = UdpSessionMap::default();
        let flow_tasks: FlowTasks = Arc::new(Mutex::new(Vec::new()));
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 2)), 40123);
        let key = UdpSessionKey {
            local,
            remote: echo_addr,
        };

        handle_udp_datagram(
            tunnel.clone(),
            dns_table.clone(),
            sessions.clone(),
            reply_tx.clone(),
            flow_tasks.clone(),
            b"first".to_vec(),
            local,
            echo_addr,
        )
        .await;
        let first = tokio::time::timeout(Duration::from_secs(2), reply_rx.recv())
            .await
            .unwrap()
            .expect("first udp reply");
        assert_eq!(first.data, b"echo:first");
        assert_eq!(first.local, local);
        assert_eq!(first.remote, echo_addr);
        assert!(sessions.get(&key).is_some());

        handle_udp_datagram(
            tunnel,
            dns_table,
            sessions.clone(),
            reply_tx,
            flow_tasks.clone(),
            b"second".to_vec(),
            local,
            echo_addr,
        )
        .await;
        let second = tokio::time::timeout(Duration::from_secs(2), reply_rx.recv())
            .await
            .unwrap()
            .expect("second udp reply");
        assert_eq!(second.data, b"echo:second");
        assert_eq!(second.local, local);
        assert_eq!(second.remote, echo_addr);
        assert!(sessions.get(&key).is_some());

        echo_task.await.unwrap();
        abort_flow_tasks(&flow_tasks).await;
    });
}

#[test]
fn dns_table_expires_stale_records() {
    let table = DnsTable::default();
    let ip = IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4));
    table.insert(ip, "expired.test".to_owned(), 60);
    {
        let mut records = table.records.lock().unwrap();
        records.get_mut(&ip).unwrap().expires_at_ms = monotonic_ms().saturating_sub(1);
    }
    assert_eq!(table.lookup(ip), None);
    assert!(!table.records.lock().unwrap().contains_key(&ip));
}

#[test]
fn dns_table_evicts_earliest_record_when_full() {
    let table = DnsTable::default();
    let oldest_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    for index in 0..DNS_TABLE_MAX_RECORDS {
        let ip = IpAddr::V4(Ipv4Addr::new(
            10,
            ((index >> 16) & 0xff) as u8,
            ((index >> 8) & 0xff) as u8,
            (index & 0xff) as u8,
        ));
        table.insert(ip, format!("host-{index}.test"), 300);
    }
    {
        let mut records = table.records.lock().unwrap();
        records.get_mut(&oldest_ip).unwrap().expires_at_ms = 1;
    }

    table.insert(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55)),
        "new.test".to_owned(),
        300,
    );

    let records = table.records.lock().unwrap();
    assert_eq!(records.len(), DNS_TABLE_MAX_RECORDS);
    assert!(!records.contains_key(&oldest_ip));
}

#[test]
fn dns_response_cache_reuses_response_with_fresh_transaction_id() {
    let cache = DnsResponseCache::default();
    let query = dns_query("cache.example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)));
    let records = parse_dns_response_records(&response);
    cache.insert(&query, &response, &records);

    let mut second_query = query.clone();
    second_query[0] = 0xab;
    second_query[1] = 0xcd;
    let cached = cache.lookup(&second_query).expect("cached response");

    assert_eq!(&cached[0..2], &[0xab, 0xcd]);
    assert_eq!(&cached[2..], &response[2..]);
}

#[test]
fn dns_response_cache_matches_case_insensitive_question_and_rewrites_question() {
    let cache = DnsResponseCache::default();
    let query = dns_query("CaseCache.Example.Test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8)));
    let records = parse_dns_response_records(&response);
    cache.insert(&query, &response, &records);

    let mut second_query = dns_query("casecache.example.test", 1);
    second_query[0] = 0xab;
    second_query[1] = 0xcd;
    second_query[2] = 0x00;
    second_query[3] = 0x00;
    let cached = cache.lookup(&second_query).expect("cached response");
    let second_question_end = parse_dns_question_end(&second_query).unwrap();

    assert_eq!(&cached[0..2], &[0xab, 0xcd]);
    assert_eq!(
        &cached[12..second_question_end],
        &second_query[12..second_question_end]
    );
    assert_eq!(
        parse_dns_response_records(&cached),
        vec![(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8)),
            "casecache.example.test".to_owned(),
            60
        )]
    );
}

#[test]
fn dns_response_cache_rewrites_cached_ttl_to_remaining_lifetime() {
    let cache = DnsResponseCache::default();
    let query = dns_query("ttl-cache.example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)));
    let records = parse_dns_response_records(&response);
    cache.insert(&query, &response, &records);

    let key = dns_cache_key(&query).expect("cache key");
    {
        let mut records = cache.records.lock().unwrap();
        records.get_mut(&key).unwrap().expires_at_ms = monotonic_ms().saturating_add(42_000);
    }

    let cached = cache.lookup(&query).expect("cached response");
    let cached_records = parse_dns_response_records(&cached);

    assert_eq!(
        cached_records,
        vec![(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)),
            "ttl-cache.example.test".to_owned(),
            42
        )]
    );
}

#[test]
fn dns_response_cache_expires_stale_records() {
    let cache = DnsResponseCache::default();
    let query = dns_query("expired-cache.example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)));
    let records = parse_dns_response_records(&response);
    cache.insert(&query, &response, &records);

    let key = dns_cache_key(&query).expect("cache key");
    {
        let mut records = cache.records.lock().unwrap();
        records.get_mut(&key).unwrap().expires_at_ms = monotonic_ms().saturating_sub(1);
    }

    assert_eq!(cache.lookup(&query), None);
    assert!(!cache.records.lock().unwrap().contains_key(&key));
}

#[test]
fn dns_response_cache_evicts_earliest_record_when_full() {
    let cache = DnsResponseCache::default();
    let oldest_query = dns_query("cache-0.example.test", 1);
    for index in 0..DNS_CACHE_MAX_RECORDS {
        let query = dns_query(&format!("cache-{index}.example.test"), 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response = build_dns_response(
            &query,
            &parsed,
            IpAddr::V4(Ipv4Addr::new(
                203,
                0,
                ((index >> 8) & 0xff) as u8,
                index as u8,
            )),
        );
        let records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &records);
    }
    let oldest_key = dns_cache_key(&oldest_query).expect("cache key");
    {
        let mut records = cache.records.lock().unwrap();
        records.get_mut(&oldest_key).unwrap().expires_at_ms = 1;
    }

    let query = dns_query("cache-new.example.test", 1);
    let parsed = parse_dns_query(&query).expect("dns query");
    let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 99)));
    let response_records = parse_dns_response_records(&response);
    cache.insert(&query, &response, &response_records);

    let records = cache.records.lock().unwrap();
    assert_eq!(records.len(), DNS_CACHE_MAX_RECORDS);
    assert!(!records.contains_key(&oldest_key));
}

#[test]
fn shared_stats_tracks_recent_dns_queries() {
    let stats = SharedStats::default();
    stats.dns_cache_hits.store(2, Ordering::Relaxed);
    stats.dns_cache_misses.store(5, Ordering::Relaxed);
    stats.record_dns_query("first.example.test".to_owned(), "A".to_owned());
    stats.record_dns_query("second.example.test".to_owned(), "AAAA".to_owned());
    stats.record_dns_query("first.example.test".to_owned(), "A".to_owned());
    stats.record_dns_query("First.Example.Test".to_owned(), "A".to_owned());

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dns_cache_hits, 2);
    assert_eq!(snapshot.dns_cache_misses, 5);
    assert_eq!(snapshot.recent_dns_queries.len(), 2);
    assert_eq!(snapshot.recent_dns_queries[0].name, "First.Example.Test");
    assert_eq!(snapshot.recent_dns_queries[0].record_type, "A");
    assert_eq!(snapshot.recent_dns_queries[0].count, 3);
    assert_eq!(snapshot.recent_dns_queries[1].name, "second.example.test");
}

fn dns_query(name: &str, qtype: u16) -> Vec<u8> {
    let mut query = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    push_dns_name(&mut query, name);
    query.extend_from_slice(&qtype.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    query
}

fn build_dns_cname_response(
    query: &[u8],
    request: &DnsQuery,
    cname_target: &str,
    ip: IpAddr,
) -> Vec<u8> {
    let mut response = Vec::with_capacity(query.len() + 96);
    response.extend_from_slice(&query[..request.question_end]);
    response[2] = 0x81;
    response[3] = 0x80;
    response[6..8].copy_from_slice(&2_u16.to_be_bytes());
    response[8..10].copy_from_slice(&0_u16.to_be_bytes());
    response[10..12].copy_from_slice(&0_u16.to_be_bytes());

    response.extend_from_slice(&[0xc0, 0x0c]);
    response.extend_from_slice(&5_u16.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&60_u32.to_be_bytes());
    let mut cname_rdata = Vec::new();
    push_dns_name(&mut cname_rdata, cname_target);
    response.extend_from_slice(&(cname_rdata.len() as u16).to_be_bytes());
    response.extend_from_slice(&cname_rdata);

    push_dns_name(&mut response, cname_target);
    match ip {
        IpAddr::V4(ip) => {
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&4_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            response.extend_from_slice(&28_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&16_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
    }
    response
}

fn push_dns_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
}
