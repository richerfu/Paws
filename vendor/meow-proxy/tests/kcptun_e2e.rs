#![cfg(feature = "kcptun")]
//! Real-server interop for the built-in `kcptun` SIP003 transport.
//!
//!   meow-rs ShadowsocksAdapter ──KCP/crypt/FEC──> Go harness
//!   (kcp-go `ListenWithOptions` + snappy + xtaci/smux v1 +
//!    go-shadowsocks2 termination) ──echo──> back
//!
//! The Go side is `tests/support/kcptun-server` — real upstream libraries
//! doing the crypt envelope, FEC decode, snappy framing and smux stream
//! handling, so a green run proves wire compatibility with code we did
//! not write, not self-consistency. The UDP leg exercises the legacy
//! UDP-over-TCP relay (`sp.udp-over-tcp.arpa`) the same way.
//!
//! Gated on `$KCPTUN_SERVER_BIN`: the whole suite is real-peer, so an
//! unset binary fails — `MEOW_KCPTUN_E2E_ALLOW_SKIP=1` prints a loud skip
//! for local runs (same convention as `MEOW_SMUX_E2E_ALLOW_SKIP`). Build
//! with: `(cd tests/support/kcptun-server && go build -o kcptun-server .)`

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use meow_common::{Metadata, Network, ProxyAdapter};
use meow_proxy::dialer::DirectDialer;
use meow_proxy::ShadowsocksAdapter;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};

const SS_PASSWORD: &str = "kcptun-test-password";
const SS_CIPHER: &str = "aes-256-gcm";
const TIMEOUT: Duration = Duration::from_secs(15);

fn harness_binary() -> Option<PathBuf> {
    match std::env::var_os("KCPTUN_SERVER_BIN") {
        Some(p) => {
            let p = PathBuf::from(p);
            assert!(p.is_file(), "KCPTUN_SERVER_BIN is not a file: {p:?}");
            Some(p)
        }
        None => {
            // Fail-closed: every leg is real-peer, so a green run must
            // mean the wire interop actually ran (sing-box convention).
            assert_eq!(
                std::env::var("MEOW_KCPTUN_E2E_ALLOW_SKIP").ok().as_deref(),
                Some("1"),
                "kcptun_e2e requires KCPTUN_SERVER_BIN — build \
                 tests/support/kcptun-server (`go build -o kcptun-server .`) \
                 or set MEOW_KCPTUN_E2E_ALLOW_SKIP=1 to skip locally"
            );
            eprintln!("SKIP: kcptun_e2e — KCPTUN_SERVER_BIN unset (allowed)");
            None
        }
    }
}

/// Spawn the Go harness; returns (child, udp listen addr).
async fn spawn_server(extra: &[&str]) -> Option<(Child, SocketAddr)> {
    let bin = harness_binary()?;
    let mut child = Command::new(bin)
        .args([
            "-listen",
            "127.0.0.1:0",
            "-sspass",
            SS_PASSWORD,
            "-sscipher",
            SS_CIPHER,
        ])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn kcptun-server harness");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
    let mut line = String::new();
    timeout(TIMEOUT, stdout.read_line(&mut line))
        .await
        .expect("harness banner timed out")
        .expect("read harness banner");
    let addr = line
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("LISTEN="))
        .unwrap_or_else(|| panic!("no LISTEN= in harness banner: {line:?}"));
    Some((child, addr.parse().expect("listen addr parses")))
}

fn adapter(port: u16, opts: &str, udp: bool) -> ShadowsocksAdapter {
    ShadowsocksAdapter::new(
        "test-ss-kcptun",
        "127.0.0.1",
        port,
        SS_PASSWORD,
        SS_CIPHER,
        udp,
        Some("kcptun"),
        Some(opts),
        None,
        Arc::new(DirectDialer),
    )
    .expect("adapter with built-in kcptun must construct")
}

async fn tcp_echo_round_trips(adapter: &ShadowsocksAdapter) {
    // The target address the SS layer asks for — the harness ignores it
    // and echoes, so any syntactically valid destination works.
    let metadata = Metadata {
        network: Network::Tcp,
        dst_ip: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        dst_port: 80,
        ..Default::default()
    };
    let mut conn = timeout(TIMEOUT, adapter.dial_tcp(&metadata))
        .await
        .expect("dial_tcp timed out")
        .expect("dial_tcp failed against the kcptun server");

    for round in 0..3 {
        let payload = format!("kcptun echo round {round} — {}", "x".repeat(900));
        conn.write_all(payload.as_bytes()).await.expect("write");
        conn.flush().await.expect("flush");
        let mut buf = vec![0u8; payload.len()];
        conn.read_exact(&mut buf).await.expect("read echo");
        assert_eq!(&buf, payload.as_bytes(), "echo mismatch round {round}");
    }
}

#[tokio::test]
async fn kcptun_tcp_echo_aes_snappy_fec() {
    // Defaults minus conn: aes crypt, snappy on, FEC 10+3, smux v1.
    let Some((_child, addr)) = spawn_server(&["-crypt", "aes"]).await else {
        return;
    };
    tcp_echo_round_trips(&adapter(addr.port(), "conn=1", false)).await;
}

#[tokio::test]
async fn kcptun_tcp_echo_salsa20_nocomp_asymmetric_fec() {
    // `datashard=0`/`parityshard=0` is unexpressible upstream too —
    // `FillDefaults` maps 0 → 10/3 on both sides — so the client still
    // encodes FEC 10+3 while the server (`-ds 0 -ps 0`, straight into
    // kcp-go without the plugin's defaults) runs no encoder and lazily
    // builds a 1+1 decoder + autotune. The wire is *asymmetric* FEC:
    // client parity on, server parity off.
    let Some((_child, addr)) =
        spawn_server(&["-crypt", "salsa20", "-nocomp", "-ds", "0", "-ps", "0"]).await
    else {
        return;
    };
    tcp_echo_round_trips(&adapter(
        addr.port(),
        "crypt=salsa20;nocomp=true;datashard=0;parityshard=0",
        false,
    ))
    .await;
}

#[tokio::test]
async fn kcptun_tcp_echo_aes128gcm() {
    // The AEAD envelope variant — exercises the nonce‖GCM-seal wire shape.
    let Some((_child, addr)) = spawn_server(&["-crypt", "aes-128-gcm"]).await else {
        return;
    };
    tcp_echo_round_trips(&adapter(addr.port(), "crypt=aes-128-gcm", false)).await;
}

#[tokio::test]
async fn kcptun_udp_over_tcp_round_trip() {
    let Some((_child, addr)) = spawn_server(&["-crypt", "aes"]).await else {
        return;
    };
    let adapter = adapter(addr.port(), "conn=1", true);
    assert!(adapter.support_udp(), "kcptun must advertise UDP via UoT");

    let target: SocketAddr = "192.0.2.53:5353".parse().unwrap();
    let metadata = Metadata {
        network: Network::Udp,
        host: smol_str::SmolStr::from(target.ip().to_string()),
        dst_ip: Some(target.ip()),
        dst_port: target.port(),
        ..Default::default()
    };
    let conn = timeout(TIMEOUT, adapter.dial_udp(&metadata))
        .await
        .expect("dial_udp timed out")
        .expect("dial_udp failed — UoT stream must open");

    let payload = b"kcptun uot datagram \x00\xff payload";
    let sent = timeout(TIMEOUT, conn.write_packet(payload, &target))
        .await
        .expect("write_packet timed out")
        .expect("write_packet failed");
    assert_eq!(sent, payload.len());

    let mut buf = vec![0u8; 2048];
    let (n, from) = timeout(TIMEOUT, conn.read_packet(&mut buf))
        .await
        .expect("read_packet timed out")
        .expect("read_packet failed");
    assert_eq!(&buf[..n], payload, "uot echo mismatch");
    assert_eq!(from, target, "uot source address must echo back");
}

/// Layer-isolation probe: bare `KcpStream` against the harness's `-raw`
/// echo mode — no snappy, no smux, no SS. Green proves the crypt+FEC+KCP
/// wire layers alone.
#[tokio::test]
async fn kcptun_raw_kcp_stream_echo() {
    use meow_transport::kcptun::{KcpConfig, KcpStream};
    use tokio::net::UdpSocket;

    let Some((_child, addr)) = spawn_server(&["-crypt", "aes", "-raw"]).await else {
        return;
    };
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(addr).await.unwrap();
    let cfg = KcpConfig {
        key: "it's a secrect".into(),
        crypt: "aes".into(),
        ..Default::default()
    };
    let mut stream =
        KcpStream::connect(Box::new(socket), rand::random(), &cfg).expect("kcp connect");

    let payload = b"raw kcp wire probe".repeat(30);
    stream.write_all(&payload).await.expect("write");
    stream.flush().await.expect("flush");
    let mut buf = vec![0u8; payload.len()];
    timeout(TIMEOUT, stream.read_exact(&mut buf))
        .await
        .expect("raw echo timed out")
        .expect("raw echo read");
    assert_eq!(buf, payload);
}

/// Second layer-isolation probe: `CompStream` (snappy framing) over KCP
/// against the harness's `-rawsnappy` mode — isolates the snappy layer
/// from smux/SS.
#[tokio::test]
async fn kcptun_raw_snappy_stream_echo() {
    use meow_transport::kcptun::{comp_stream, KcpConfig, KcpStream};
    use tokio::net::UdpSocket;

    let Some((_child, addr)) = spawn_server(&["-crypt", "aes", "-rawsnappy"]).await else {
        return;
    };
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(addr).await.unwrap();
    let cfg = KcpConfig {
        key: "it's a secrect".into(),
        crypt: "aes".into(),
        ..Default::default()
    };
    let kcp = KcpStream::connect(Box::new(socket), rand::random(), &cfg).expect("kcp connect");
    let mut stream = comp_stream(kcp);

    // >64KB forces multiple snappy chunks both directions.
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    stream.write_all(&payload).await.expect("write");
    stream.flush().await.expect("flush");
    let mut buf = vec![0u8; payload.len()];
    timeout(TIMEOUT, stream.read_exact(&mut buf))
        .await
        .expect("snappy echo timed out")
        .expect("snappy echo read");
    assert_eq!(buf, payload);
}

/// Every supported crypt name exercised against real upstream code — the
/// self-roundtrip unit tests can't catch a wire divergence inside a
/// cipher (tea's halved round count was exactly that). `-raw` with FEC
/// off isolates the crypt envelope alone.
#[tokio::test]
async fn kcptun_raw_kcp_all_crypts_echo() {
    use meow_transport::kcptun::{KcpConfig, KcpStream};
    use tokio::net::UdpSocket;

    for crypt in [
        "aes",
        "aes-256",
        "aes-128",
        "aes-192",
        "aes-128-gcm",
        "salsa20",
        "blowfish",
        "twofish",
        "cast5",
        "3des",
        "xtea",
        "tea",
        "xor",
        "none",
        "null",
    ] {
        let Some((_child, addr)) =
            spawn_server(&["-crypt", crypt, "-raw", "-ds", "0", "-ps", "0"]).await
        else {
            return;
        };
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.connect(addr).await.unwrap();
        let cfg = KcpConfig {
            key: "it's a secrect".into(),
            crypt: crypt.into(),
            data_shard: 0,
            parity_shard: 0,
            ..Default::default()
        };
        let mut stream =
            KcpStream::connect(Box::new(socket), rand::random(), &cfg).expect("kcp connect");

        let payload = format!("crypt {crypt} probe ").into_bytes().repeat(20);
        stream.write_all(&payload).await.expect("write");
        stream.flush().await.expect("flush");
        let mut buf = vec![0u8; payload.len()];
        timeout(TIMEOUT, stream.read_exact(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("{crypt}: raw echo timed out"))
            .expect("raw echo read");
        assert_eq!(buf, payload, "{crypt} echo mismatch");
    }
}

/// Third layer-isolation probe: `smux::Session` over CompStream over
/// `KcpStream` against the harness's `-rawsmux` echo — isolates the smux
/// stream layer from the SS termination.
#[tokio::test]
async fn kcptun_raw_smux_stream_echo() {
    use meow_proxy::mux::smux;
    use meow_transport::kcptun::{comp_stream, KcpConfig, KcpStream};
    use tokio::net::UdpSocket;

    let Some((_child, addr)) = spawn_server(&["-crypt", "aes", "-rawsmux"]).await else {
        return;
    };
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(addr).await.unwrap();
    let cfg = KcpConfig {
        key: "it's a secrect".into(),
        crypt: "aes".into(),
        ..Default::default()
    };
    let kcp = KcpStream::connect(Box::new(socket), rand::random(), &cfg).expect("kcp connect");
    let session = Arc::new(
        smux::Session::client_kcptun(
            Box::new(comp_stream(kcp)),
            cfg.stream_buf as usize,
            Duration::from_secs(cfg.keep_alive as u64),
            Duration::from_secs(30),
            cfg.frame_size as usize,
            cfg.smux_buf as usize,
        )
        .expect("smux session"),
    );

    let mut stream = timeout(TIMEOUT, session.open_stream())
        .await
        .expect("open_stream timed out")
        .expect("open_stream failed");
    let payload = b"raw smux wire probe".repeat(20);
    timeout(TIMEOUT, stream.write_all(&payload))
        .await
        .expect("write timed out")
        .expect("write");
    timeout(TIMEOUT, stream.flush())
        .await
        .expect("flush timed out")
        .expect("flush");
    let mut buf = vec![0u8; payload.len()];
    timeout(TIMEOUT, stream.read_exact(&mut buf))
        .await
        .expect("smux echo timed out")
        .expect("smux echo read");
    assert_eq!(buf, payload);
}

/// `sndwnd=2` paces outbound data two segments per ACK — the UNA-slide
/// immediate flush (kcp-go `Input` semantics) is the only reason a 64KB
/// echo finishes well inside the timeout; classic ikcp paced every
/// window by the maintenance interval.
#[tokio::test]
async fn kcptun_tcp_echo_tiny_sndwnd() {
    let Some((_child, addr)) = spawn_server(&["-crypt", "aes"]).await else {
        return;
    };
    let adapter = adapter(addr.port(), "conn=1;sndwnd=2", false);
    let metadata = Metadata {
        network: Network::Tcp,
        dst_ip: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        dst_port: 80,
        ..Default::default()
    };
    let mut conn = timeout(TIMEOUT, adapter.dial_tcp(&metadata))
        .await
        .expect("dial_tcp timed out")
        .expect("dial_tcp failed");

    // Write in 4KB chunks — the shadowsocks crate's connect path packs
    // the whole first user write into one handshake buffer but only
    // encrypts ≤16KB of it (upstream quirk, debug-asserts on larger).
    let payload = vec![0xabu8; 64 * 1024];
    for chunk in payload.chunks(4096) {
        timeout(TIMEOUT, conn.write_all(chunk))
            .await
            .expect("write timed out")
            .expect("write");
    }
    timeout(TIMEOUT, conn.flush())
        .await
        .expect("flush timed out")
        .expect("flush");
    let mut buf = vec![0u8; payload.len()];
    timeout(TIMEOUT, conn.read_exact(&mut buf))
        .await
        .expect("tiny-window echo timed out")
        .expect("tiny-window echo read");
    assert_eq!(buf, payload);
}
