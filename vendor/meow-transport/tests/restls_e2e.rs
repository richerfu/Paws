//! restls end-to-end tests — a fused restls relay + cover-TLS terminator on
//! loopback exercises the real wire protocol: session-id auth tag,
//! masked server-auth record, tagged post-handshake data records, and the
//! transparent-fallback path when the "relay" is a plain TLS terminator.

mod support;

use meow_transport::restls::{self, RestlsConfig};
use rustls::ServerConnection;
use std::io;
use std::io::{Read, Write};
use std::sync::Arc;
use support::loopback::{gen_cert, install_crypto_provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PASSWORD: &str = "e2e-psk";
const RECORD_HDR: usize = 5;
const AUTH_HEADER_LEN: usize = 12;

fn secret(password: &str) -> [u8; 32] {
    blake3::derive_key("restls-traffic-key", password.as_bytes())
}

async fn read_record(tcp: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; RECORD_HDR];
    tcp.read_exact(&mut hdr).await?;
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    let mut rec = hdr.to_vec();
    rec.resize(RECORD_HDR + len, 0);
    tcp.read_exact(&mut rec[RECORD_HDR..]).await?;
    Ok(rec)
}

/// Parse a ClientHello record; returns `(session_id, key_shares)` where each
/// share is `(group, public bytes)`.
fn parse_client_hello(record: &[u8]) -> (Vec<u8>, Vec<(u16, Vec<u8>)>) {
    assert_eq!(record[0], 22, "expected handshake record");
    let body = &record[RECORD_HDR..];
    assert_eq!(body[0], 1, "expected ClientHello");
    let mut pos = 4 + 2 + 32; // hs header + version + random
    let sid_len = body[pos] as usize;
    pos += 1;
    let sid = body[pos..pos + sid_len].to_vec();
    pos += sid_len;
    let cs_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2 + cs_len;
    let comp_len = body[pos] as usize;
    pos += 1 + comp_len;
    let ext_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2;
    let exts = &body[pos..pos + ext_len];
    let mut shares = Vec::new();
    let mut ep = 0;
    while ep + 4 <= exts.len() {
        let typ = u16::from_be_bytes([exts[ep], exts[ep + 1]]);
        let len = u16::from_be_bytes([exts[ep + 2], exts[ep + 3]]) as usize;
        let data = &exts[ep + 4..ep + 4 + len];
        if typ == 51 {
            let mut sp = 2; // skip client_shares length
            while sp + 4 <= data.len() {
                let group = u16::from_be_bytes([data[sp], data[sp + 1]]);
                let klen = u16::from_be_bytes([data[sp + 2], data[sp + 3]]) as usize;
                shares.push((group, data[sp + 4..sp + 4 + klen].to_vec()));
                sp += 4 + klen;
            }
        }
        ep += 4 + len;
    }
    (sid, shares)
}

/// The restls session-id tag — `blake3(secret, Σ group‖share)[:16]`.
fn ch_tag(secret: &[u8; 32], shares: &[(u16, Vec<u8>)]) -> [u8; 16] {
    let mut h = blake3::Hasher::new_keyed(secret);
    for (group, share) in shares {
        h.update(&group.to_be_bytes());
        h.update(share);
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    out
}

/// `maskServerAuth` — XOR blake3(secret, server_random) into the first 16
/// bytes after the record header.
fn mask_server_auth(record: &mut [u8], secret: &[u8; 32], server_random: &[u8; 32]) {
    let mut h = blake3::Hasher::new_keyed(secret);
    h.update(server_random);
    let mask = h.finalize();
    for (i, b) in record[RECORD_HDR..RECORD_HDR + 16].iter_mut().enumerate() {
        *b ^= mask.as_bytes()[i];
    }
}

// ── server-side tagged-record helpers (mirror of wire.rs, dir-flipped) ─────

fn auth_hash(secret: &[u8; 32], sr: &[u8; 32], to_client: bool, ctr: u64) -> blake3::Hasher {
    let mut h = blake3::Hasher::new_keyed(secret);
    h.update(sr);
    h.update(if to_client {
        b"server-to-client".as_slice()
    } else {
        b"client-to-server".as_slice()
    });
    h.update(&ctr.to_be_bytes());
    h
}

/// Server-side extract — verifies a client→server tagged record, returns
/// `(data_len, command)`. `gcm` selects the 8-byte nonce prefix layout.
fn server_extract(
    record: &[u8],
    secret: &[u8; 32],
    sr: &[u8; 32],
    ctr: u64,
    client_fin: Option<&[u8]>,
    gcm: bool,
) -> Option<(usize, [u8; 2])> {
    let hdr = RECORD_HDR + if gcm { 8 } else { 0 };
    if record.len() < hdr + AUTH_HEADER_LEN || record[0] != 23 {
        return None;
    }
    if gcm && u64::from_be_bytes(record[RECORD_HDR..RECORD_HDR + 8].try_into().unwrap()) != ctr + 1
    {
        return None;
    }
    let payload = &record[hdr..];
    let mut hmac = auth_hash(secret, sr, false, ctr);
    if let Some(fin) = client_fin {
        hmac.update(fin);
    }
    hmac.update(&record[..hdr]);
    hmac.update(&payload[8..]);
    if hmac.finalize().as_bytes()[..8] != payload[..8] {
        return None;
    }
    let mut hmask = auth_hash(secret, sr, false, ctr);
    let data_region = &payload[AUTH_HEADER_LEN..];
    hmask.update(&data_region[..data_region.len().min(32)]);
    let mask = hmask.finalize();
    let mut field = [0u8; 4];
    for (i, b) in payload[8..12].iter().enumerate() {
        field[i] = b ^ mask.as_bytes()[i];
    }
    Some((
        u16::from_be_bytes([field[0], field[1]]) as usize,
        [field[2], field[3]],
    ))
}

/// Server-side tagged-record build — data travels server→client.
fn server_build(
    data: &[u8],
    secret: &[u8; 32],
    sr: &[u8; 32],
    ctr: u64,
    command: [u8; 2],
    gcm: bool,
) -> Vec<u8> {
    let hdr = RECORD_HDR + if gcm { 8 } else { 0 };
    // Record length covers the explicit nonce for TLS 1.2 GCM.
    let payload_len = data.len() + AUTH_HEADER_LEN + if gcm { 8 } else { 0 };
    let mut out = Vec::with_capacity(RECORD_HDR + payload_len);
    out.extend_from_slice(&[0x17, 0x03, 0x03]);
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    if gcm {
        out.extend_from_slice(&(ctr + 1).to_be_bytes());
    }
    out.resize(hdr + AUTH_HEADER_LEN, 0);
    out.extend_from_slice(data);
    // len||cmd masked with the data-prefix mask.
    let mut hmask = auth_hash(secret, sr, true, ctr);
    hmask.update(&out[hdr + AUTH_HEADER_LEN..][..data.len().min(32)]);
    let mask = hmask.finalize();
    let field = &mut out[hdr + 8..hdr + 12];
    field[..2].copy_from_slice(&(data.len() as u16).to_be_bytes());
    field[2..4].copy_from_slice(&command);
    for (b, m) in field.iter_mut().zip(mask.as_bytes().iter()) {
        *b ^= m;
    }
    let mut hmac = auth_hash(secret, sr, true, ctr);
    hmac.update(&out[..hdr]);
    hmac.update(&out[hdr + 8..]);
    out[hdr..hdr + 8].copy_from_slice(&hmac.finalize().as_bytes()[..8]);
    out
}

fn server_conn() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    server_conn_tickets(0)
}

fn server_conn_tickets(
    tickets: usize,
) -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    cfg.send_tls13_tickets = tickets;
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

/// A cover requesting client auth (optional): rustls then emits a
/// CertificateRequest inside the flight — the client must answer with
/// an (empty) Certificate or the cover aborts.
#[derive(Debug)]
struct OptionalClientAuth;

impl rustls::server::danger::ClientCertVerifier for OptionalClientAuth {
    fn offer_client_auth(&self) -> bool {
        true
    }
    fn client_auth_mandatory(&self) -> bool {
        false
    }
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

fn server_conn_client_auth() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut cfg = rustls::ServerConfig::builder()
        .with_client_cert_verifier(Arc::new(OptionalClientAuth))
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    cfg.send_tls13_tickets = 0;
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

fn client_cfg(cert_der: &rustls::pki_types::CertificateDer<'static>) -> RestlsConfig {
    RestlsConfig {
        server_name: "cover.example.com".to_string(),
        password: PASSWORD.to_string(),
        record_script: None,
        version_hint: "tls13".to_string(),
        skip_cert_verify: false,
        verify_name: None,
        cert_pin: None,
        additional_roots: vec![cert_der.as_ref().to_vec()],
    }
}

/// Drive the fused relay+cover for the handshake: verifies the client's
/// session-id tag, relays records to rustls, and masks the cover's first
/// encrypted record. Returns when the client's Finished has been relayed.
async fn relay_handshake(
    tcp: TcpStream,
    conn: &mut ServerConnection,
    secret: &[u8; 32],
    mask: bool,
) -> io::Result<(TcpStream, [u8; 32], Option<Vec<u8>>)> {
    relay_handshake_fin(tcp, conn, secret, mask, 0, 0).await
}

/// `fin_index` selects which post-CCS client record is the Finished —
/// a client-auth cover sends Certificate first, so Finished rides
/// record 1 instead of record 0. `extra_ccs` appends synthetic compat-CCS
/// records after the cover's own — a restls cover emitting ≥2 CCS must
/// not defeat the unconditional server-auth mask probe.
async fn relay_handshake_fin(
    mut tcp: TcpStream,
    conn: &mut ServerConnection,
    secret: &[u8; 32],
    mask: bool,
    fin_index: usize,
    extra_ccs: usize,
) -> io::Result<(TcpStream, [u8; 32], Option<Vec<u8>>)> {
    let mut server_random = [0u8; 32];
    let mut masked = false;
    // Cover-side CCS precedes the cover's encrypted flight.
    let mut cover_ccs = false;
    let mut saw_ccs = false;
    let mut first_ch = true;
    // Post-CCS client record `fin_index` is the sealed Finished — the
    // restls server mixes it into the first tagged record's MAC
    // (`clientFinRaw`).
    let mut client_fin = None;
    let mut post_ccs = 0usize;
    loop {
        while conn.wants_write() {
            let mut produced = Vec::new();
            conn.write_tls(&mut produced)?;
            // Split into records; mask the first post-CCS appdata record.
            let mut pos = 0;
            while pos + RECORD_HDR <= produced.len() {
                let len = u16::from_be_bytes([produced[pos + 3], produced[pos + 4]]) as usize;
                let mut rec = produced[pos..pos + RECORD_HDR + len].to_vec();
                if rec[0] == 22 && rec.len() > 43 && rec[RECORD_HDR] == 2 {
                    server_random.copy_from_slice(&rec[RECORD_HDR + 6..RECORD_HDR + 38]);
                }
                if rec[0] == 20 {
                    cover_ccs = true;
                }
                if rec[0] == 23 && cover_ccs && !masked && mask {
                    mask_server_auth(&mut rec, secret, &server_random);
                    masked = true;
                }
                tcp.write_all(&rec).await?;
                if rec[0] == 20 {
                    for _ in 0..extra_ccs {
                        tcp.write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01]).await?;
                    }
                }
                pos += RECORD_HDR + len;
            }
        }
        if !conn.is_handshaking() && !conn.wants_write() {
            break;
        }
        let rec = read_record(&mut tcp).await?;
        if first_ch {
            let (sid, shares) = parse_client_hello(&rec);
            assert_eq!(sid.len(), 32);
            let ok = ch_tag(secret, &shares) == sid[..16];
            assert!(ok, "session-id auth tag mismatch");
            first_ch = false;
        }
        if rec[0] == 20 {
            saw_ccs = true;
        } else if rec[0] == 23 && saw_ccs {
            if post_ccs == fin_index && client_fin.is_none() {
                client_fin = Some(rec.clone());
            }
            post_ccs += 1;
        }
        conn.read_tls(&mut &rec[..])?;
        conn.process_new_packets()
            .map_err(|e| io::Error::other(format!("cover TLS: {e}")))?;
    }
    // Flush trailing writes.
    while conn.wants_write() {
        let mut produced = Vec::new();
        conn.write_tls(&mut produced)?;
        tcp.write_all(&produced).await?;
    }
    Ok((tcp, server_random, client_fin))
}

// ── tests ─────────────────────────────────────────────────────────────────

/// Every e2e is wrapped in a timeout — a stalled handshake or data path
/// must fail, not hang the suite.
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

macro_rules! timed_test {
    ($name:ident, $imp:ident) => {
        #[tokio::test]
        async fn $name() {
            tokio::time::timeout(TEST_TIMEOUT, $imp())
                .await
                .expect(concat!(stringify!($imp), " timed out"));
        }
    };
}

timed_test!(e2e_tagged_data_path, e2e_tagged_data_path_impl);
timed_test!(e2e_server_respond_command, e2e_server_respond_command_impl);
timed_test!(e2e_verify_name_override, e2e_verify_name_override_impl);
timed_test!(e2e_transparent_fallback, e2e_transparent_fallback_impl);
timed_test!(
    e2e_transparent_fallback_nst,
    e2e_transparent_fallback_nst_impl
);
timed_test!(
    e2e_cert_pin_mismatch_fails,
    e2e_cert_pin_mismatch_fails_impl
);
timed_test!(e2e_tls12_tagged, e2e_tls12_tagged_impl);
timed_test!(e2e_tls12_p256_group, e2e_tls12_p256_group_impl);
timed_test!(e2e_tls13_p256_keyshare, e2e_tls13_p256_keyshare_impl);
timed_test!(
    e2e_tls13_client_cert_request,
    e2e_tls13_client_cert_request_impl
);
timed_test!(
    e2e_tls12_client_cert_request,
    e2e_tls12_client_cert_request_impl
);
timed_test!(e2e_stray_ccs_ignored, e2e_stray_ccs_ignored_impl);
timed_test!(e2e_tagged_extra_ccs, e2e_tagged_extra_ccs_impl);
timed_test!(e2e_respond_sent_decrement, e2e_respond_sent_decrement_impl);
timed_test!(e2e_upstream_interop, e2e_upstream_interop_impl);

/// Full restls path: tagged relay masks the cover's first encrypted record;
/// post-handshake data travels in tagged records shaped by the script.
async fn e2e_tagged_data_path_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        // Data path: tagged records in, echo back as tagged records.
        let mut to_server_ctr = 0u64;
        let mut to_client_ctr = 0u64;
        // Echo each data record once, with Respond(n) fake replies, until
        // the client drops.
        loop {
            let Ok(rec) = read_record(&mut tcp).await else {
                break;
            };
            let fin = if to_server_ctr == 0 {
                client_fin.as_deref()
            } else {
                None
            };
            let Some((data_len, cmd)) =
                server_extract(&rec, &secret, &server_random, to_server_ctr, fin, false)
            else {
                panic!(
                    "tagged record failed verification: ctr={} len={} fin={:?} rec={:?}",
                    to_server_ctr,
                    rec.len(),
                    client_fin.as_ref().map(Vec::len),
                    &rec[..rec.len().min(24)]
                );
            };
            to_server_ctr += 1;
            if cmd[0] == 0x01 {
                // Respond(n) → n fake all-padding records.
                for _ in 0..cmd[1] {
                    let fake =
                        server_build(&[], &secret, &server_random, to_client_ctr, [0, 0], false);
                    to_client_ctr += 1;
                    tcp.write_all(&fake).await.unwrap();
                }
            }
            if data_len > 0 {
                let data =
                    &rec[RECORD_HDR + AUTH_HEADER_LEN..RECORD_HDR + AUTH_HEADER_LEN + data_len];
                let reply =
                    server_build(data, &secret, &server_random, to_client_ctr, [0, 0], false);
                to_client_ctr += 1;
                tcp.write_all(&reply).await.unwrap();
            }
        }
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial");
    s.write_all(b"ping").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"ping");
    drop(s);
    server.await.unwrap();
}

/// TLS 1.3 cover restricted to P-256 — the client must derive the shared
/// secret from the *P-256* keyshare it offered (selecting the offered
/// keypair by the server's chosen group, not `keys[0]`).
async fn e2e_tls13_p256_keyshare_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut provider = rustls::crypto::ring::default_provider();
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::SECP256R1];
    let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13 versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    cfg.send_tls13_tickets = 0;
    let mut conn = ServerConnection::new(Arc::new(cfg)).expect("server conn");
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        let rec = read_record(&mut tcp).await.unwrap();
        let (data_len, _) = server_extract(
            &rec,
            &secret,
            &server_random,
            0,
            client_fin.as_deref(),
            false,
        )
        .expect("tagged record");
        let reply = server_build(
            &rec[RECORD_HDR + AUTH_HEADER_LEN..RECORD_HDR + AUTH_HEADER_LEN + data_len],
            &secret,
            &server_random,
            0,
            [0, 0],
            false,
        );
        tcp.write_all(&reply).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert_der))
        .await
        .expect("restls dial vs P-256 cover");
    s.write_all(b"p256").await.unwrap();
    let mut out = [0u8; 16];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"p256");
    drop(s);
    server.await.unwrap();
}

/// An inbound `Respond(n)` command asks the client for n fake all-padding
/// records — the server's way of requesting traffic shaping. The client must
/// emit exactly n verifiable tagged records and still deliver the data the
/// commanding record carried.
async fn e2e_server_respond_command_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        let mut to_client_ctr = 0u64;
        // First inbound record carries data + Respond(2).
        let probe = server_build(
            b"probe",
            &secret,
            &server_random,
            to_client_ctr,
            [0x01, 2],
            false,
        );
        to_client_ctr += 1;
        tcp.write_all(&probe).await.unwrap();
        // The client must emit exactly 2 fake records — verifiable tagged
        // records with data_len == 0.
        for i in 0..2u64 {
            let rec = read_record(&mut tcp).await.unwrap();
            // The first client record — fake or not — binds client_fin.
            let fin = if i == 0 { client_fin.as_deref() } else { None };
            let Some((data_len, _cmd)) =
                server_extract(&rec, &secret, &server_random, i, fin, false)
            else {
                panic!("fake record {i} failed verification");
            };
            assert_eq!(data_len, 0, "fake record {i} must be all padding");
        }
        // The data path still works afterwards.
        let tail = server_build(
            b"world",
            &secret,
            &server_random,
            to_client_ctr,
            [0, 0],
            false,
        );
        tcp.write_all(&tail).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial");
    let mut out = [0u8; 64];
    // First read delivers the commanding record's payload…
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"probe");
    // …the second read drains the staged fakes, then returns the tail.
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"world");
    drop(s);
    server.await.unwrap();
}

/// A stray post-handshake CCS record must not advance either counter —
/// upstream's `readRecordOrCCS` consumes CCS in a separate arm before the
/// tagged path. If we ticked, the tagged record after it would fail the
/// MAC check and kill the connection.
async fn e2e_stray_ccs_ignored_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        // Read the client's first tagged record so the handshake data is
        // drained, then answer with: tagged "one", a raw CCS, tagged "two".
        let rec = read_record(&mut tcp).await.unwrap();
        assert!(server_extract(
            &rec,
            &secret,
            &server_random,
            0,
            client_fin.as_deref(),
            false
        )
        .is_some());
        let r1 = server_build(b"one", &secret, &server_random, 0, [0, 0], false);
        tcp.write_all(&r1).await.unwrap();
        const CCS: [u8; 6] = [0x14, 0x03, 0x03, 0x00, 0x01, 0x01];
        tcp.write_all(&CCS).await.unwrap();
        let r2 = server_build(b"two", &secret, &server_random, 1, [0, 0], false);
        tcp.write_all(&r2).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial");
    s.write_all(b"x").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"one");
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"two", "record after stray CCS must verify");
    drop(s);
    server.await.unwrap();
}

/// Upstream `handleRestlsCommand(cmd, sent)`: when the inbound record that
/// carries `Respond(n)` also releases `<`-held writes that actually emit,
/// that emitted record counts as one of the n responses — n-1 fakes, not n.
/// Script `5<,50`: the first write emits 5 bytes and holds the rest.
async fn e2e_respond_sent_decrement_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        // Record 0: line1's 5 data bytes + Respond(0) interrupt command.
        let rec = read_record(&mut tcp).await.unwrap();
        let Some((data_len, _)) = server_extract(
            &rec,
            &secret,
            &server_random,
            0,
            client_fin.as_deref(),
            false,
        ) else {
            panic!("record 0 failed verification");
        };
        assert_eq!(data_len, 5);
        // Release the hold while commanding Respond(2) — the released data
        // record absorbs one response, so exactly 1 fake must follow.
        let probe = server_build(b"go", &secret, &server_random, 0, [0x01, 2], false);
        tcp.write_all(&probe).await.unwrap();
        let rec = read_record(&mut tcp).await.unwrap();
        let Some((data_len, _)) = server_extract(&rec, &secret, &server_random, 1, None, false)
        else {
            panic!("released data record failed verification");
        };
        assert_eq!(data_len, 35, "held bytes must flush on release");
        let rec = read_record(&mut tcp).await.unwrap();
        let Some((data_len, _)) = server_extract(&rec, &secret, &server_random, 2, None, false)
        else {
            panic!("fake record failed verification");
        };
        assert_eq!(data_len, 0);
        // No fourth record — a second fake would mean the decrement was
        // lost.
        match tokio::time::timeout(std::time::Duration::from_millis(500), read_record(&mut tcp))
            .await
        {
            // Timeout or clean close: no extra record was emitted.
            Err(_) | Ok(Err(_)) => {}
            Ok(Ok(rec)) => panic!(
                "unexpected extra client record — Respond decrement lost: len={} verify={:?}",
                rec.len(),
                server_extract(&rec, &secret, &server_random, 3, None, false)
            ),
        }
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut cfg = client_cfg(&cert);
    cfg.record_script = Some("5<,50".to_string());
    let mut s = restls::dial(tcp, &cfg).await.expect("restls dial");
    s.write_all(&[7u8; 40]).await.unwrap();
    // The probe's payload drives the read path that releases the hold.
    let mut out = [0u8; 16];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"go");
    drop(s);
    server.await.unwrap();
}

/// `name-cert-verify` checks the cover certificate against a name that
/// differs from the SNI the relay sees — the cert is minted for
/// `verify.example.com` while the ClientHello still advertises
/// `cover.example.com`.
async fn e2e_verify_name_override_impl() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["verify.example.com"]);
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    cfg.send_tls13_tickets = 0;
    let mut conn = ServerConnection::new(Arc::new(cfg)).expect("server conn");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, _fin) = relay_handshake(tcp, &mut conn, &secret, true)
            .await
            .expect("relay handshake");
        // Echo one tagged record so the client proves the data path too.
        let rec = read_record(&mut tcp).await.unwrap();
        let Some((data_len, _cmd)) =
            server_extract(&rec, &secret, &server_random, 0, _fin.as_deref(), false)
        else {
            panic!("tagged record failed verification");
        };
        let data = &rec[RECORD_HDR + AUTH_HEADER_LEN..RECORD_HDR + AUTH_HEADER_LEN + data_len];
        let reply = server_build(data, &secret, &server_random, 0, [0, 0], false);
        tcp.write_all(&reply).await.unwrap();
    });

    let mut cfg = client_cfg(&cert_der);
    cfg.verify_name = Some("verify.example.com".to_string());
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &cfg).await.expect("restls dial");
    s.write_all(b"named").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"named");
    drop(s);
    server.await.unwrap();
}

/// Wrong password → the relay treats the client as a plain cover connection
/// (upstream's fallback); the client's unmask fails and the stream degrades
/// to transparent cover TLS.
async fn e2e_transparent_fallback_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // Plain relay: no masking — the tag won't verify anyway.
        let (mut tcp, _, _) = relay_handshake(tcp, &mut conn, &secret, false)
            .await
            .expect("relay handshake");
        // Pull the client's first (cover-cipher) record so rustls yields the
        // plaintext, then echo it back through the cover.
        // Transparent TLS: client writes flow through the cover cipher;
        // pull plaintext from rustls and echo it back.
        let rec = read_record(&mut tcp).await.unwrap();
        conn.read_tls(&mut &rec[..]).unwrap();
        conn.process_new_packets().unwrap();
        let mut out = [0u8; 128];
        let n = conn.reader().read(&mut out).unwrap();
        conn.writer().write_all(&out[..n]).unwrap();
        let mut produced = Vec::new();
        conn.write_tls(&mut produced).unwrap();
        tcp.write_all(&produced).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial");
    s.write_all(b"transparent").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"transparent");
    server.await.unwrap();
}

/// Same fallback, but the cover issues post-handshake TLS 1.3 session
/// tickets — a real TLS conn consumes them internally, so the transparent
/// path must skip inner-handshake records instead of dying.
async fn e2e_transparent_fallback_nst_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn_tickets(2);
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, _, _) = relay_handshake(tcp, &mut conn, &secret, false)
            .await
            .expect("relay handshake");
        // The cover's two post-handshake NST records were flushed to the
        // client during the handshake drain — the client's transparent
        // read path must skip them before surfacing app data.
        let rec = read_record(&mut tcp).await.unwrap();
        conn.read_tls(&mut &rec[..]).unwrap();
        conn.process_new_packets().unwrap();
        let mut out = [0u8; 128];
        let n = conn.reader().read(&mut out).unwrap();
        conn.writer().write_all(&out[..n]).unwrap();
        let mut produced = Vec::new();
        conn.write_tls(&mut produced).unwrap();
        tcp.write_all(&produced).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial");
    s.write_all(b"nst-fallback").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"nst-fallback");
    server.await.unwrap();
}

/// A forged/unpinned cover certificate must fail verification.
async fn e2e_cert_pin_mismatch_fails_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, _cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // The pin mismatch surfaces client-side; keep the relay alive long
        // enough for the handshake to proceed, then let it drop.
        let _ = relay_handshake(tcp, &mut conn, &secret, true).await;
    });

    let mut cfg = client_cfg(&_cert);
    cfg.cert_pin = Some([0xAA; 32]); // wrong pin
    let tcp = TcpStream::connect(addr).await.unwrap();
    match restls::dial(tcp, &cfg).await {
        Ok(_) => panic!("pin must reject"),
        Err(e) => assert!(e.to_string().contains("fingerprint"), "{e}"),
    }
    server.await.unwrap();
}

// ── TLS 1.2 ───────────────────────────────────────────────────────────────

/// Extract the CKE pubkey from a plaintext client record and check the
/// layout3 session-id tag (server-side `restls` verification). Upstream
/// reads the segment for the *negotiated* curve
/// (`sessionId[layout[curveIndex]]`) — inferred here from the CKE pubkey
/// length: X25519 → 32 B → segment 0, P-256 → 65 B → segment 1,
/// P-384 → 97 B → segment 2.
fn verify_tls12_tag(ch: &[u8], cke_pub: &[u8], secret: &[u8; 32]) -> bool {
    let (sid, _shares) = parse_client_hello(ch);
    assert_eq!(sid.len(), 32);
    let layout3 = [0usize, 11, 22, 32];
    let seg = match cke_pub.len() {
        32 => 0,
        65 => 1,
        97 => 2,
        other => panic!("unexpected CKE pubkey length {other}"),
    };
    let mut h = blake3::Hasher::new_keyed(secret);
    h.update(cke_pub);
    let tag = h.finalize();
    tag.as_bytes()[..layout3[seg + 1] - layout3[seg]] == sid[layout3[seg]..layout3[seg + 1]]
}

fn server_conn_tls12() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    server_conn_tls12_kx(rustls::crypto::ring::default_provider().kx_groups)
}

/// A TLS 1.2 cover restricted to `kx` groups — a P-256-only cover forces
/// the client to select its P-256 eager key (a `keys[0]`-style bug would
/// send an X25519 pubkey and fail the ECDHE).
fn server_conn_tls12_kx(
    kx: Vec<&'static dyn rustls::crypto::SupportedKxGroup>,
) -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut provider = rustls::crypto::ring::default_provider();
    provider.kx_groups = kx;
    let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .expect("tls12 versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    // Enable ticket emission: the cover sends a plaintext NewSessionTicket
    // record before its CCS (RFC 5077 order) — the client must consume it
    // rather than erroring on a pre-CCS record.
    cfg.ticketer = rustls::crypto::ring::Ticketer::new().expect("ticketer");
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

/// A TLS 1.2 cover requesting client auth — CertificateRequest rides the
/// flight between SKE and ServerHelloDone; the client answers empty.
fn server_conn_tls12_client_auth() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>)
{
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12])
    .expect("tls12 versions")
    .with_client_cert_verifier(Arc::new(OptionalClientAuth))
    .with_single_cert(vec![cert_der.clone()], key_der)
    .expect("server config");
    cfg.ticketer = rustls::crypto::ring::Ticketer::new().expect("ticketer");
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

/// restls `version-hint=tls12` e2e: eager-key session-id tags, masked first
/// encrypted record, tagged records with the GCM nonce slot.
async fn e2e_tls12_tagged_impl() {
    let (conn, cert) = server_conn_tls12();
    e2e_tls12_tagged_inner(conn, cert).await;
}

/// Same tagged-path flow against a P-256-only cover — the CKE must carry
/// the P-256 eager key's pubkey (tag then verifies at layout3 segment 1).
async fn e2e_tls12_p256_group_impl() {
    let (conn, cert) = server_conn_tls12_kx(vec![rustls::crypto::ring::kx_group::SECP256R1]);
    e2e_tls12_tagged_inner(conn, cert).await;
}

async fn e2e_tls12_tagged_inner(
    mut conn: ServerConnection,
    cert: rustls::pki_types::CertificateDer<'static>,
) {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut server_random = [0u8; 32];
        let mut ch_record = Vec::new();
        let mut cke_pub = Vec::new();
        let mut masked = false;
        let mut cover_ccs = false;
        let mut client_ccs = false;
        let mut disable_ctr = false;
        let mut saw_nst = false;
        let mut handshake_done = false;
        // Relay the handshake; capture CH, server random, and the client's
        // plaintext ClientKeyExchange pubkey for tag verification.
        while !handshake_done {
            while conn.wants_write() {
                let mut produced = Vec::new();
                conn.write_tls(&mut produced).unwrap();
                let mut pos = 0;
                while pos + RECORD_HDR <= produced.len() {
                    let len = u16::from_be_bytes([produced[pos + 3], produced[pos + 4]]) as usize;
                    let mut rec = produced[pos..pos + RECORD_HDR + len].to_vec();
                    if rec[0] == 22 && rec.len() > 43 && rec[RECORD_HDR] == 2 {
                        server_random.copy_from_slice(&rec[RECORD_HDR + 6..RECORD_HDR + 38]);
                    }
                    if rec[0] == 20 {
                        cover_ccs = true;
                    } else if rec[0] == 22 && !cover_ccs {
                        // Scan the plaintext handshake record — messages may
                        // be coalesced; find a NewSessionTicket (type 4).
                        let mut p = RECORD_HDR;
                        while p + 4 <= rec.len() {
                            if rec[p] == 4 {
                                saw_nst = true;
                            }
                            let ml = ((rec[p + 1] as usize) << 16)
                                | ((rec[p + 2] as usize) << 8)
                                | rec[p + 3] as usize;
                            p += 4 + ml;
                        }
                    } else if cover_ccs && !masked {
                        // First encrypted record — GCM nonce slot is the
                        // explicit nonce; zero nonce → mask after it.
                        let zero_nonce = rec[RECORD_HDR..RECORD_HDR + 8] == [0u8; 8];
                        disable_ctr = !zero_nonce;
                        let off = if zero_nonce {
                            RECORD_HDR + 8
                        } else {
                            RECORD_HDR
                        };
                        let mut h = blake3::Hasher::new_keyed(&secret);
                        h.update(&server_random);
                        let mask = h.finalize();
                        for (i, b) in rec[off..off + 16].iter_mut().enumerate() {
                            *b ^= mask.as_bytes()[i];
                        }
                        masked = true;
                    }
                    tcp.write_all(&rec).await.unwrap();
                    pos += RECORD_HDR + len;
                }
            }
            if !conn.is_handshaking() && !conn.wants_write() {
                break;
            }
            let rec = read_record(&mut tcp).await.unwrap();
            if ch_record.is_empty() {
                ch_record = rec.clone();
            }
            if rec[0] == 20 {
                client_ccs = true;
            }
            if rec[0] == 22 && !client_ccs && rec[RECORD_HDR] == 16 {
                // ClientKeyExchange: [16 u24len u8len pubkey]
                let pk_len = rec[RECORD_HDR + 4] as usize;
                cke_pub = rec[RECORD_HDR + 5..RECORD_HDR + 5 + pk_len].to_vec();
            }
            conn.read_tls(&mut &rec[..]).unwrap();
            conn.process_new_packets().unwrap();
            if !conn.is_handshaking() && !conn.wants_write() {
                handshake_done = true;
            }
        }
        assert!(
            verify_tls12_tag(&ch_record, &cke_pub, &secret),
            "tls12 session-id tag mismatch"
        );
        assert!(saw_nst, "expected a plaintext NewSessionTicket record");
        // `disableCtr` is INBOUND-ONLY upstream (`restls_server.go`:
        // `parrotGCM` controls the server→client nonce slot; the client
        // always writes its `toServerCounter+1` nonce).
        let server_nonce_slot = !disable_ctr;
        // Client→server tagged records always carry the 8-byte nonce.
        let data_off = RECORD_HDR + 8 + AUTH_HEADER_LEN;
        let mut to_server_ctr = 0u64;
        let mut to_client_ctr = 0u64;
        loop {
            let Ok(rec) = read_record(&mut tcp).await else {
                break;
            };
            let Some((data_len, cmd)) =
                server_extract(&rec, &secret, &server_random, to_server_ctr, None, true)
            else {
                panic!("tls12 tagged record failed verification");
            };
            to_server_ctr += 1;
            if cmd[0] == 0x01 {
                for _ in 0..cmd[1] {
                    let fake = server_build(
                        &[],
                        &secret,
                        &server_random,
                        to_client_ctr,
                        [0, 0],
                        server_nonce_slot,
                    );
                    to_client_ctr += 1;
                    tcp.write_all(&fake).await.unwrap();
                }
            }
            if data_len > 0 {
                let data = &rec[data_off..data_off + data_len];
                let reply = server_build(
                    data,
                    &secret,
                    &server_random,
                    to_client_ctr,
                    [0, 0],
                    server_nonce_slot,
                );
                to_client_ctr += 1;
                tcp.write_all(&reply).await.unwrap();
            }
        }
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut cfg = client_cfg(&cert);
    cfg.version_hint = "tls12".to_string();
    let mut s = restls::dial(tcp, &cfg).await.expect("restls12 dial");
    s.write_all(b"pong12").await.unwrap();
    let mut out = [0u8; 64];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"pong12");
    drop(s);
    server.await.unwrap();
}

/// Real-peer interop against the upstream restls server
/// (`github.com/metacubex/restls-client-go`'s `RestlsServer` over a Go
/// `crypto/tls` cover). The Go side authenticates our session-id tag,
/// unmasks our records, and echoes plaintext — a green round trip proves
/// both directions against code we did not write, not self-consistency.
///
/// The suite **fails** when `$RESTLS_SERVER_BIN` is unset — a green CI run
/// must exercise the real-peer leg. Build the harness from
/// `tests/support/restls-server/`:
/// `(cd tests/support/restls-server && go build -o restls-server .)`
/// `MEOW_RESTLS_E2E_ALLOW_SKIP=1` prints a loud explicit skip for local
/// runs only — CI builds the harness via `actions/setup-go` and never
/// sets it.
async fn e2e_upstream_interop_impl() {
    use std::process::Stdio;
    let Some(bin) = std::env::var_os("RESTLS_SERVER_BIN") else {
        if std::env::var_os("MEOW_RESTLS_E2E_ALLOW_SKIP").is_some() {
            eprintln!(
                "SKIP: RESTLS_SERVER_BIN unset and MEOW_RESTLS_E2E_ALLOW_SKIP \
                 is set — upstream interop NOT exercised"
            );
            return;
        }
        panic!(
            "RESTLS_SERVER_BIN is required for the upstream-interop leg \
             (build tests/support/restls-server and point it at the binary). \
             A green run must exercise real-peer interop, so this test \
             refuses to silently skip — set MEOW_RESTLS_E2E_ALLOW_SKIP=1 \
             only for a loud local skip."
        );
    };
    let mut child = tokio::process::Command::new(&bin)
        .args(["-listen", "127.0.0.1:0", "-password", PASSWORD])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn restls server");
    let mut stdout = child.stdout.take().expect("stdout piped");
    // First line: "LISTEN=127.0.0.1:PORT COVER=…"
    let mut line = String::new();
    tokio::io::AsyncBufReadExt::read_line(&mut tokio::io::BufReader::new(&mut stdout), &mut line)
        .await
        .expect("read banner");
    let listen = line
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("LISTEN="))
        .expect("LISTEN= in server banner")
        .to_string();

    for version in ["tls13", "tls12"] {
        let tcp = TcpStream::connect(&listen).await.unwrap();
        let cfg = RestlsConfig {
            server_name: "cover.example.com".to_string(),
            password: PASSWORD.to_string(),
            record_script: None,
            version_hint: version.to_string(),
            // The Go cover presents a self-signed cert — the protocol
            // interop is under test, not the PKI path (covered elsewhere).
            skip_cert_verify: true,
            verify_name: None,
            cert_pin: None,
            additional_roots: vec![],
        };
        let mut s = restls::dial(tcp, &cfg)
            .await
            .unwrap_or_else(|e| panic!("restls {version} dial vs upstream: {e}"));
        for round in 0..2 {
            let payload = format!("upstream-{version}-round{round}");
            s.write_all(payload.as_bytes()).await.unwrap();
            s.flush().await.unwrap();
            let mut buf = vec![0u8; payload.len()];
            s.read_exact(&mut buf)
                .await
                .unwrap_or_else(|e| panic!("restls {version} echo round{round}: {e}"));
            assert_eq!(&buf, payload.as_bytes(), "{version} echo mismatch");
        }
    }
    drop(child);
}

/// A cover emitting two compat-CCS records must not defeat the
/// unconditional server-auth mask probe (regression: the removed
/// `server_ccs == 1` gate would skip the probe here and drop the client
/// into transparent mode — the tagged echo would then fail to parse).
async fn e2e_tagged_extra_ccs_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, server_random, client_fin) =
            relay_handshake_fin(tcp, &mut conn, &secret, true, 0, 1)
                .await
                .expect("relay handshake");
        let rec = read_record(&mut tcp).await.unwrap();
        let (data_len, _) = server_extract(
            &rec,
            &secret,
            &server_random,
            0,
            client_fin.as_deref(),
            false,
        )
        .expect("tagged record must parse — mask probe ran despite 2 CCS");
        let data = &rec[RECORD_HDR + AUTH_HEADER_LEN..RECORD_HDR + AUTH_HEADER_LEN + data_len];
        let out = server_build(data, &secret, &server_random, 0, [0, 0], false);
        tcp.write_all(&out).await.unwrap();
    });

    let mut stream = restls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(&cert))
        .await
        .expect("restls dial");
    stream.write_all(b"ping").await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    server.await.unwrap();
}

/// TLS 1.3 cover requesting client auth: the CertificateRequest lands
/// between EE and Certificate; the client must answer with an empty
/// Certificate (restls never carries client certs) or the cover aborts.
async fn e2e_tls13_client_cert_request_impl() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn_client_auth();
    let secret = secret(PASSWORD);

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // Certificate precedes Finished — the tag binds record 1.
        let (mut tcp, server_random, client_fin) =
            relay_handshake_fin(tcp, &mut conn, &secret, true, 1, 0)
                .await
                .expect("relay handshake");
        let rec = read_record(&mut tcp).await.unwrap();
        let (data_len, _) = server_extract(
            &rec,
            &secret,
            &server_random,
            0,
            client_fin.as_deref(),
            false,
        )
        .expect("tagged record");
        let reply = server_build(
            &rec[RECORD_HDR + AUTH_HEADER_LEN..RECORD_HDR + AUTH_HEADER_LEN + data_len],
            &secret,
            &server_random,
            0,
            [0, 0],
            false,
        );
        tcp.write_all(&reply).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut s = restls::dial(tcp, &client_cfg(&cert))
        .await
        .expect("restls dial vs client-auth cover");
    s.write_all(b"cr").await.unwrap();
    let mut out = [0u8; 16];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"cr");
    drop(s);
    server.await.unwrap();
}

/// TLS 1.2 cover requesting client auth: CertificateRequest is legal only
/// between SKE and ServerHelloDone — the ordering gate plus the empty
/// Certificate response get exercised end to end.
async fn e2e_tls12_client_cert_request_impl() {
    let (conn, cert) = server_conn_tls12_client_auth();
    e2e_tls12_tagged_inner(conn, cert).await;
}
