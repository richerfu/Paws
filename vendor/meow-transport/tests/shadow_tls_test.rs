//! shadow-tls end-to-end tests — a fake shadow-tls server (fused
//! cover-TLS terminator + relay) on loopback exercises the real wire
//! protocol for v1/v2/v3 against a BoringSSL client.

mod support;

use hmac::{Hmac, Mac};
use meow_transport::tls::{TlsConfig, TlsLayer, TlsVersion};
use rustls::ServerConnection;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::io;
use std::sync::Arc;
use support::loopback::{gen_cert, install_crypto_provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type HmacSha1 = Hmac<Sha1>;

const PASSWORD: &[u8] = b"e2e-psk";
const RECORD_HDR: usize = 5;
const SERVER_RANDOM_INDEX: usize = RECORD_HDR + 4 + 2; // hdr + hs-hdr + version

fn sha1_hmac(key: &[u8]) -> HmacSha1 {
    HmacSha1::new_from_slice(key).unwrap()
}

fn kdf(password: &[u8], server_random: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(password);
    h.update(server_random);
    h.finalize().into()
}

fn xor_in_place(data: &mut [u8], key: &[u8; 32]) {
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[i % 32];
    }
}

/// Read exactly one TLS record from the wire.
async fn read_record(tcp: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; RECORD_HDR];
    tcp.read_exact(&mut hdr).await?;
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    let mut rec = hdr.to_vec();
    rec.resize(RECORD_HDR + len, 0);
    tcp.read_exact(&mut rec[RECORD_HDR..]).await?;
    Ok(rec)
}

/// rustls `write_tls` output → wire bytes, applying the v3 relay swizzle
/// to `application_data` records once `server_random` has been captured.
struct OutAssembler {
    rec: Vec<u8>,
    want: usize,
    server_random: Option<[u8; 32]>,
    write_hmac: Option<HmacSha1>,
    xor_key: Option<[u8; 32]>,
    password: Vec<u8>,
    version: u8,
}

impl OutAssembler {
    fn new(version: u8, password: &[u8]) -> Self {
        Self {
            rec: Vec::new(),
            want: 0,
            server_random: None,
            write_hmac: None,
            xor_key: None,
            password: password.to_vec(),
            version,
        }
    }

    /// Feed produced TLS bytes; returns wire bytes (records may be
    /// withheld until complete).
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        self.rec.extend_from_slice(bytes);
        loop {
            if self.want == 0 {
                if self.rec.len() < RECORD_HDR {
                    break;
                }
                self.want = RECORD_HDR + u16::from_be_bytes([self.rec[3], self.rec[4]]) as usize;
            }
            if self.rec.len() < self.want {
                break;
            }
            let record: Vec<u8> = self.rec.drain(..self.want).collect();
            self.want = 0;
            out.extend(self.emit(record));
        }
        out
    }

    fn emit(&mut self, mut record: Vec<u8>) -> Vec<u8> {
        // Capture the server random out of the ServerHello record.
        if record[0] == 22 && record.len() >= SERVER_RANDOM_INDEX + 32 && record[RECORD_HDR] == 2 {
            let mut rand = [0u8; 32];
            rand.copy_from_slice(&record[SERVER_RANDOM_INDEX..SERVER_RANDOM_INDEX + 32]);
            self.server_random = Some(rand);
            if self.version == 3 {
                let mut chain = sha1_hmac(&self.password);
                chain.update(&rand);
                self.write_hmac = Some(chain);
                self.xor_key = Some(kdf(&self.password, &rand));
            }
            return record;
        }
        // v3: swizzle cover appdata — tag(4) over the xor'd payload under
        // the handshake chain, then xor the payload.
        if self.version == 3 && record[0] == 23 && record.len() > 9 {
            if let (Some(chain), Some(key)) = (&mut self.write_hmac, &self.xor_key) {
                xor_in_place(&mut record[RECORD_HDR..], key);
                chain.update(&record[RECORD_HDR..]);
                let tag = chain.clone().finalize().into_bytes();
                let mut out = record[..RECORD_HDR].to_vec();
                out[3..5].copy_from_slice(&((record.len() - RECORD_HDR + 4) as u16).to_be_bytes());
                out.extend(&tag[..4]);
                out.extend(&record[RECORD_HDR..]);
                return out;
            }
        }
        record
    }
}

/// Group ids for the hybrid-PQ key shares the vendored BoringSSL offers by
/// default via boring-sys's boring-pq.patch (`X25519MLKEM768`,
/// `P256Kyber768Draft00`).  The v2 pin must strip both; the ML-KEM one is
/// the load-bearing assert (upstream's strip targets it by name).
const GROUP_X25519_MLKEM768: u16 = 0x11ec;
const GROUP_P256_KYBER768_DRAFT00: u16 = 0xfe32;

/// Whether the ClientHello's `supported_groups` extension offers `group`.
/// v2 asserts `false` for the hybrid-PQ groups (upstream strips them —
/// `BuildRemovedX25519MLKEM768HandshakeState`; a hybrid-PQ share breaks
/// v2 servers, and the production pin in `tls_config_for` is what keeps
/// this green), while v3 asserts `true` for ML-KEM to keep the v2 check
/// non-vacuous: a boring default flip shows up in the *positive* v3
/// assertion as well.
fn ch_offers_group(record: &[u8], group: u16) -> bool {
    assert!(record.len() >= 6, "record too short for a handshake header");
    assert_eq!(record[0], 22, "expected a TLS handshake record");
    assert_eq!(record[5], 1, "expected a ClientHello handshake message");
    let sid_len_index = SERVER_RANDOM_INDEX + 32;
    assert!(
        record.len() > sid_len_index,
        "record too short for session-id length"
    );
    let mut i = sid_len_index + 1 + record[sid_len_index] as usize;
    assert!(record.len() >= i + 2, "record too short for cipher-suites");
    let cs_len = u16::from_be_bytes([record[i], record[i + 1]]) as usize;
    i += 2 + cs_len;
    assert!(record.len() > i, "record too short for compression methods");
    i += 1 + record[i] as usize; // compression methods
    assert!(record.len() >= i + 2, "record too short for extensions");
    let ext_len = u16::from_be_bytes([record[i], record[i + 1]]) as usize;
    let ext_end = i + 2 + ext_len;
    i += 2;
    while i + 4 <= ext_end && i + 4 <= record.len() {
        let ty = u16::from_be_bytes([record[i], record[i + 1]]);
        let el = u16::from_be_bytes([record[i + 2], record[i + 3]]) as usize;
        if ty == 0x000a {
            // supported_groups
            let end = (i + 4 + el).min(record.len());
            let [hi, lo] = group.to_be_bytes();
            return record[i + 4..end].as_chunks::<2>().0.contains(&[hi, lo]);
        }
        i += 4 + el;
    }
    false
}

/// Verify the v3 ClientHello's embedded session-id tag
/// (upstream `verifyClientHello`).
fn verify_ch_tag(record: &[u8], password: &[u8]) {
    assert_eq!(record[0], 22, "first client record must be handshake");
    assert_eq!(record[RECORD_HDR], 1, "must be a ClientHello");
    let sid_len_index = SERVER_RANDOM_INDEX + 32;
    assert_eq!(record[sid_len_index], 32, "compat session id must be 32B");
    let sid_start = sid_len_index + 1;
    let sid_end = sid_start + 32;
    let mut chain = sha1_hmac(password);
    chain.update(&record[RECORD_HDR..sid_end - 4]);
    chain.update(&[0u8; 4]);
    chain.update(&record[sid_end..]);
    let tag = chain.finalize().into_bytes();
    assert_eq!(
        &record[sid_end - 4..sid_end],
        &tag[..4],
        "v3 ClientHello session-id tag"
    );
}

/// Run the cover handshake over `tcp` (rustls terminator + relay
/// mutations), returning the raw stream, the full inbound transcript,
/// and the captured server random.
async fn cover_handshake(
    mut tcp: TcpStream,
    conn: &mut ServerConnection,
    version: u8,
    password: &[u8],
) -> io::Result<(TcpStream, Vec<u8>, [u8; 32])> {
    let mut asm = OutAssembler::new(version, password);
    let mut inbound = Vec::new();
    let mut first_checked = false;
    loop {
        while conn.wants_write() {
            let mut produced = Vec::new();
            conn.write_tls(&mut produced)?;
            let wire = asm.feed(&produced);
            tcp.write_all(&wire).await?;
        }
        if !conn.is_handshaking() && !conn.wants_write() {
            break;
        }
        let rec = read_record(&mut tcp).await?;
        if version == 3 && !first_checked {
            verify_ch_tag(&rec, password);
            first_checked = true;
        }
        inbound.extend_from_slice(&rec);
        conn.read_tls(&mut &rec[..])?;
        conn.process_new_packets()
            .map_err(|e| io::Error::other(format!("cover TLS: {e}")))?;
    }
    // Flush any trailing writes.
    while conn.wants_write() {
        let mut produced = Vec::new();
        conn.write_tls(&mut produced)?;
        let wire = asm.feed(&produced);
        tcp.write_all(&wire).await?;
    }
    Ok((tcp, inbound, asm.server_random.unwrap_or([0u8; 32])))
}

fn server_conn() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    // No session tickets: a ticket that races in after Finished is hashed
    // by the server but may never be pulled by the client's TLS stack —
    // a nondeterministic transcript for the v2 handshake tag.
    cfg.send_tls13_tickets = 0;
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

fn client_layer(version: u8, cert_der: &rustls::pki_types::CertificateDer<'static>) -> TlsLayer {
    let mut cfg = TlsConfig::new("cover.example.com");
    cfg.additional_roots = vec![cert_der.as_ref().to_vec()];
    cfg.alpn = vec!["h2".to_string(), "http/1.1".to_string()];
    // Upstream floors at TLS 1.2 for every version; v1 caps at 1.2.
    cfg.min_version = Some(TlsVersion::Tls12);
    if version == 1 {
        cfg.max_version = Some(TlsVersion::Tls12);
    }
    // Mirror the production `tls_config_for` pin: v2 never offers
    // X25519MLKEM768 (boring ≥5.x would offer it by default).
    if version == 2 {
        cfg.curves = Some("X25519:P-256:P-384".to_string());
    }
    TlsLayer::new(&cfg).expect("tls layer")
}

/// A rustls cover pinned to TLS 1.2 — the plaintext-cover path where
/// certificate verification genuinely runs before the doomed Finished.
fn server_conn_tls12() -> (ServerConnection, rustls::pki_types::CertificateDer<'static>) {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let cfg = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    (
        ServerConnection::new(Arc::new(cfg)).expect("server conn"),
        cert_der,
    )
}

/// The v3 relay half of the mock server: verify the tagged ClientHello,
/// pump the doomed cover flight (swizzling post-SH appdata), then switch
/// on the first client record whose "C"-chain tag verifies and echo one
/// payload back under the "S" chain.
async fn v3_relay_server(mut tcp: TcpStream, conn: &mut ServerConnection, password: &[u8]) {
    let mut asm = OutAssembler::new(3, password);
    let mut verify: Option<HmacSha1> = None;
    let mut add: Option<HmacSha1> = None;

    // First client record must be the tagged ClientHello.
    let ch = read_record(&mut tcp).await.unwrap();
    // v3 carries no curves pin — the default BoringSSL hello must still
    // offer ML-KEM, which keeps the v2 `!ch_offers_group` assertion
    // meaningful (it would also pass vacuously if the default stopped
    // offering it).
    assert!(
        ch_offers_group(&ch, GROUP_X25519_MLKEM768),
        "unpinned v3 ClientHello should offer X25519MLKEM768 by default"
    );
    verify_ch_tag(&ch, password);
    conn.read_tls(&mut &ch[..]).unwrap();
    conn.process_new_packets().expect("cover TLS");

    let payload = 'switch: loop {
        // Pump whatever the cover wants to say, swizzling post-SH
        // appdata (SH random seeds the chains on first sight).
        while conn.wants_write() {
            let mut produced = Vec::new();
            conn.write_tls(&mut produced).unwrap();
            let wire = asm.feed(&produced);
            if verify.is_none() {
                if let Some(sr) = asm.server_random {
                    let mut v = sha1_hmac(password);
                    v.update(&sr);
                    v.update(b"C");
                    verify = Some(v);
                    let mut a = sha1_hmac(password);
                    a.update(&sr);
                    a.update(b"S");
                    add = Some(a);
                }
            }
            tcp.write_all(&wire).await.unwrap();
        }
        // A rejected dial drops the conn mid-flight — die quietly.
        let Ok(rec) = read_record(&mut tcp).await else {
            return;
        };
        if let Some(v) = &verify {
            if rec[0] == 23 && rec.len() > RECORD_HDR + 4 {
                // Upstream `copyByFrameUntilHMACMatches` — a record
                // verifying under the "C" chain is the data switch.
                let mut probe = v.clone();
                probe.update(&rec[RECORD_HDR + 4..]);
                let tag = probe.finalize().into_bytes();
                if rec[RECORD_HDR..RECORD_HDR + 4] == tag[..4] {
                    break 'switch rec[RECORD_HDR + 4..].to_vec();
                }
            }
        }
        // Untagged record — relay into the (doomed) cover conn;
        // the client Finished never verifies server-side either.
        let _ = conn.read_tls(&mut &rec[..]);
        let _ = conn.process_new_packets();
    };

    // Echo the payload back under the "S" chain.
    let add = add.as_mut().expect("S chain seeded");
    add.update(&payload);
    let etag = add.clone().finalize().into_bytes();
    add.update(&etag[..4]);
    let mut out = vec![23u8, 3, 3];
    out.extend(((payload.len() + 4) as u16).to_be_bytes());
    out.extend(&etag[..4]);
    out.extend(&payload);
    tcp.write_all(&out).await.unwrap();
}

// ─── v1 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn v1_end_to_end() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (mut tcp, _in, _rand) = cover_handshake(tcp, &mut conn, 1, PASSWORD)
            .await
            .expect("cover handshake");
        drop(conn);
        // v1 data path: raw echo.
        let mut buf = [0u8; 1024];
        let n = tcp.read(&mut buf).await.unwrap();
        tcp.write_all(&buf[..n]).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let layer = client_layer(1, &cert);
    let mut s = meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 1, PASSWORD, false)
        .await
        .expect("v1 dial");
    s.write_all(b"v1-payload").await.unwrap();
    let mut out = [0u8; 32];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"v1-payload");
    server.await.unwrap();
}

// ─── v2 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn v2_end_to_end() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let password = PASSWORD.to_vec();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // The server hashes everything it *sent* during the handshake —
        // the mirror of the client hashing everything it received.
        let mut out_chain = sha1_hmac(&password);
        let mut asm = OutAssembler::new(2, &password);
        let mut first_rec = Some(());
        let mut tcp = tcp;
        loop {
            while conn.wants_write() {
                let mut produced = Vec::new();
                conn.write_tls(&mut produced).unwrap();
                let wire = asm.feed(&produced);
                out_chain.update(&wire);
                tcp.write_all(&wire).await.unwrap();
            }
            if !conn.is_handshaking() && !conn.wants_write() {
                break;
            }
            let rec = read_record(&mut tcp).await.unwrap();
            if first_rec.take().is_some() {
                assert!(
                    !ch_offers_group(&rec, GROUP_X25519_MLKEM768),
                    "v2 ClientHello must not offer X25519MLKEM768"
                );
                assert!(
                    !ch_offers_group(&rec, GROUP_P256_KYBER768_DRAFT00),
                    "v2 ClientHello must not offer P256Kyber768Draft00"
                );
            }
            conn.read_tls(&mut &rec[..]).unwrap();
            conn.process_new_packets().expect("cover TLS");
        }
        drop(conn);
        // First framed record must carry the 8-byte transcript tag.
        let first = read_record(&mut tcp).await.unwrap();
        assert_eq!(first[0], 23, "data must be framed as appdata");
        let payload = &first[RECORD_HDR..];
        let expect = out_chain.finalize().into_bytes();
        assert_eq!(&payload[..8], &expect[..8], "v2 first-write transcript tag");
        // Echo the payload back framed.
        let body = &payload[8..];
        let mut rec = vec![23u8, 3, 3];
        rec.extend((body.len() as u16).to_be_bytes());
        rec.extend(body);
        tcp.write_all(&rec).await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let layer = client_layer(2, &cert);
    let mut s = meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 2, PASSWORD, false)
        .await
        .expect("v2 dial");
    s.write_all(b"v2-payload").await.unwrap();
    let mut out = [0u8; 32];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"v2-payload");
    server.await.unwrap();
}

// ─── v3 ─────────────────────────────────────────────────────────────────────

/// The v3 cover handshake never completes on the client side — the
/// wire-patched ClientHello diverges the transcript, so the cover's
/// Finished always fails verification.  The relay is therefore modelled
/// faithfully: the patched CH and the client's CCS are relayed into
/// rustls (the cover), cover output is swizzled once the ServerHello's
/// random is captured, and the first client record carrying a valid
/// "C"-chain tag switches the mock into echo mode.
#[tokio::test]
async fn v3_end_to_end() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();
    let password = PASSWORD.to_vec();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        v3_relay_server(tcp, &mut conn, &password).await;
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let layer = client_layer(3, &cert);
    let mut s = meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 3, PASSWORD, false)
        .await
        .expect("v3 dial — cover Finished fails but the relay authorized");
    s.write_all(b"v3-payload").await.unwrap();
    let mut out = [0u8; 32];
    let n = s.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"v3-payload");
    server.await.unwrap();
}

/// TLS 1.2 cover: the whole server flight rides plaintext handshake
/// records, so certificate verification genuinely runs before the
/// doomed Finished — making `cert_ok` the operative accept gate
/// (`(authorized, !is_tls13)` arm).  Pin all four combinations.
#[tokio::test]
async fn v3_tls12_cover_cert_gate() {
    install_crypto_provider();

    // (client trust tweak, expect dial ok, err substring on failure)
    let cases: &[(&str, bool)] = &[
        ("trusted", true),
        ("untrusted", false),
        ("pin-mismatch", false),
        ("skip-verify", true),
    ];
    for (kind, expect_ok) in cases {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (mut conn, cert) = server_conn_tls12();
        let password = PASSWORD.to_vec();
        let kind = kind.to_string();
        let expect_ok = *expect_ok;

        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            // Rejected dials may never reach the switch — let the relay
            // mock die on the dropped socket instead of hanging the test.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                v3_relay_server(tcp, &mut conn, &password),
            )
            .await;
        });

        let mut cfg = TlsConfig::new("cover.example.com");
        cfg.alpn = vec!["h2".to_string(), "http/1.1".to_string()];
        // Client still offers 1.3 (that is what emits the 32-byte compat
        // session id v3 patches); the SERVER is pinned to 1.2, so the
        // cover negotiates 1.2 and its post-SH flight stays plaintext.
        cfg.min_version = Some(TlsVersion::Tls12);
        match kind.as_str() {
            "trusted" => cfg.additional_roots = vec![cert.as_ref().to_vec()],
            "untrusted" => {
                // A different self-signed CA — the cover cert won't verify.
                let (rogue, _, _, _) = gen_cert(&["rogue.example.com"]);
                cfg.additional_roots = vec![rogue.as_ref().to_vec()];
            }
            "pin-mismatch" => {
                cfg.cert_pin = Some([0xAA; 32]); // wrong pin → custom-verify fails
            }
            "skip-verify" => cfg.skip_cert_verify = true,
            _ => unreachable!(),
        }
        let layer = TlsLayer::new(&cfg).expect("tls layer");
        let tcp = TcpStream::connect(addr).await.unwrap();
        let dial =
            meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 3, PASSWORD, false).await;
        match (dial, expect_ok) {
            (Ok(mut s), true) => {
                s.write_all(b"v3-tls12").await.unwrap();
                let mut out = [0u8; 32];
                let n = s.read(&mut out).await.unwrap();
                assert_eq!(&out[..n], b"v3-tls12", "{kind}");
            }
            (Err(e), false) => {
                assert!(
                    e.to_string().contains("shadow-tls"),
                    "{kind}: expected shadow-tls failure, got {e}"
                );
            }
            (Ok(_), false) => panic!("{kind}: dial must fail closed"),
            (Err(e), true) => panic!("{kind}: dial must succeed, got {e}"),
        }
        server.await.unwrap();
    }
}

/// `strict-mode` refuses a cover that negotiates below TLS 1.3 — even
/// when the TLS 1.2 cover would otherwise authorize (upstream
/// `ClientConfig.StrictMode`, checked before `authorized`).
#[tokio::test]
async fn v3_strict_mode_rejects_tls12_cover() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn_tls12();
    let password = PASSWORD.to_vec();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            v3_relay_server(tcp, &mut conn, &password),
        )
        .await;
    });

    let mut cfg = TlsConfig::new("cover.example.com");
    cfg.alpn = vec!["h2".to_string(), "http/1.1".to_string()];
    cfg.min_version = Some(TlsVersion::Tls12);
    cfg.additional_roots = vec![cert.as_ref().to_vec()]; // trusted 1.2 cover
    let layer = TlsLayer::new(&cfg).expect("tls layer");
    let tcp = TcpStream::connect(addr).await.unwrap();
    let err = meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 3, PASSWORD, true)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        err.contains("strict-mode"),
        "strict-mode must refuse the 1.2 cover, got {err}"
    );
    server.await.unwrap();
}

/// A plain-TLS peer (no shadow-tls relay) must fail closed: its unswizzled
/// appdata records fail the embedded-tag check, so the cover handshake
/// errors before any authorization — and even the transcript-divergent
/// Finished would never let it complete.
#[tokio::test]
async fn v3_hijacked_cover_fails() {
    install_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (mut conn, cert) = server_conn();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // Plain TLS termination — no swizzle, no tag check.  The client
        // aborts on the first unswizzled record; the server just relays
        // until the connection dies.
        let _ = cover_handshake(tcp, &mut conn, 1, PASSWORD).await;
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let layer = client_layer(3, &cert);
    let err = meow_transport::shadow_tls::dial(Box::new(tcp), &layer, 3, PASSWORD, false)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        err.contains("shadow-tls") || err.contains("handshake") || err.contains("hmac"),
        "expected a shadow-tls handshake failure, got: {err}"
    );
    let _ = server.await;
}
