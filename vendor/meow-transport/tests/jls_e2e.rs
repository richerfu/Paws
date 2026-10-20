//! jls end-to-end tests — a record-level jls server on loopback exercises
//! the real wire protocol: ClientHello.random sealed-credential auth, the
//! server's own sealed ServerHello.random, certificate-skip on the
//! authenticated path, and plain TLS 1.3 application records after the
//! handshake. A rustls cover exercises the unauthenticated path — the
//! handshake completes and the client then rejects it.

mod support;

use aes_gcm::aead::consts::{U12, U32};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::{Aes128Gcm, AesGcm, Nonce};
use hmac::{Hmac, Mac};
use meow_transport::jls::{self, JlsConfig};
use rustls::ServerConnection;
use sha2::{Digest, Sha256};
use std::io;
use std::sync::Arc;
use support::loopback::{gen_cert, install_crypto_provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const USERNAME: &str = "alice";
const PASSWORD: &str = "e2e-pwd";
const RECORD_HDR: usize = 5;

/// JLS seals with AES-256-GCM under a 32-byte nonce (upstream
/// `NewGCMWithNonceSize(sha256.Size)`).
type JlsCipher = AesGcm<Aes256, U32>;

// ---- jls auth primitives (independent impl — wire-format cross-check) ----

fn jls_key(password: &str, auth_data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(password.as_bytes());
    h.update(auth_data);
    h.finalize().into()
}

fn jls_nonce(username: &str, auth_data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(username.as_bytes());
    h.update(auth_data);
    h.finalize().into()
}

/// Open a fake random; `Some(seed)` when valid.
fn jls_open_random(user: &str, pass: &str, auth_data: &[u8], random: &[u8; 32]) -> Option<Vec<u8>> {
    let cipher = JlsCipher::new_from_slice(&jls_key(pass, auth_data)).unwrap();
    let nonce = jls_nonce(user, auth_data);
    cipher
        .decrypt(
            Nonce::<U32>::from_slice(&nonce),
            Payload {
                msg: random,
                aad: &[],
            },
        )
        .ok()
        .filter(|pt| pt.len() == 16)
}

/// Seal a 16-byte seed into a fake random (server side).
fn jls_seal_random(user: &str, pass: &str, auth_data: &[u8]) -> [u8; 32] {
    let cipher = JlsCipher::new_from_slice(&jls_key(pass, auth_data)).unwrap();
    let nonce = jls_nonce(user, auth_data);
    let seed: [u8; 16] = rand::random();
    let ct = cipher
        .encrypt(
            Nonce::<U32>::from_slice(&nonce),
            Payload {
                msg: &seed,
                aad: &[],
            },
        )
        .unwrap();
    let mut out = [0u8; 32];
    out.copy_from_slice(&ct);
    out
}

// ---- SHA-256 TLS 1.3 key schedule -----------------------------------------

fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(salt).unwrap();
    m.update(ikm);
    m.finalize().into_bytes().to_vec()
}

fn hkdf_expand(prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut t = Vec::new();
    let mut i = 1u8;
    while out.len() < len {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(prk).unwrap();
        m.update(&t);
        m.update(info);
        m.update(&[i]);
        t = m.finalize().into_bytes().to_vec();
        out.extend_from_slice(&t);
        i += 1;
    }
    out.truncate(len);
    out
}

fn expand_label(secret: &[u8], label: &str, ctx: &[u8], len: usize) -> Vec<u8> {
    let mut info = Vec::with_capacity(4 + 6 + label.len() + ctx.len());
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label.as_bytes());
    info.push(ctx.len() as u8);
    info.extend_from_slice(ctx);
    hkdf_expand(secret, &info, len)
}

fn derive_secret(secret: &[u8], label: &str, msgs: &[u8]) -> Vec<u8> {
    expand_label(secret, label, &Sha256::digest(msgs), 32)
}

fn put_u24(v: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
}

struct RecordCipher {
    key: Aes128Gcm,
    iv: [u8; 12],
    seq: u64,
}

impl RecordCipher {
    fn new(secret: &[u8]) -> Self {
        let key_bytes = expand_label(secret, "key", &[], 16);
        let iv_vec = expand_label(secret, "iv", &[], 12);
        let mut iv = [0u8; 12];
        iv.copy_from_slice(&iv_vec);
        Self {
            key: Aes128Gcm::new_from_slice(&key_bytes).unwrap(),
            iv,
            seq: 0,
        }
    }

    fn nonce(&mut self) -> [u8; 12] {
        let mut n = self.iv;
        for (b, s) in n[4..].iter_mut().zip(self.seq.to_be_bytes()) {
            *b ^= s;
        }
        self.seq += 1;
        n
    }

    /// Seal `plaintext` under inner type `inner_type` — one TLS 1.3 record.
    fn seal(&mut self, inner_type: u8, plaintext: &[u8]) -> Vec<u8> {
        let mut inner = plaintext.to_vec();
        inner.push(inner_type);
        let hdr_len = (inner.len() + 16) as u16;
        let aad = [0x17, 0x03, 0x03, (hdr_len >> 8) as u8, hdr_len as u8];
        let nonce = self.nonce();
        let ct = self
            .key
            .encrypt(
                Nonce::<U12>::from_slice(&nonce),
                Payload {
                    msg: &inner,
                    aad: &aad,
                },
            )
            .unwrap();
        let mut out = aad.to_vec();
        out.extend_from_slice(&ct);
        out
    }

    fn open(&mut self, record: &[u8]) -> Option<(u8, Vec<u8>)> {
        let nonce = self.nonce();
        let pt = self
            .key
            .decrypt(
                Nonce::<U12>::from_slice(&nonce),
                Payload {
                    msg: &record[RECORD_HDR..],
                    aad: &record[..RECORD_HDR],
                },
            )
            .ok()?;
        let (inner, typ) = pt.split_at(pt.len() - 1);
        Some((typ[0], inner.to_vec()))
    }
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

/// Parsed ClientHello fields the fixture needs.
struct ParsedCH {
    wire: Vec<u8>, // handshake message (type+len+body)
    random: [u8; 32],
    session_id: Vec<u8>,
    server_name: String,
    x25519_share: [u8; 32],
}

fn parse_client_hello(record: &[u8]) -> ParsedCH {
    assert_eq!(record[0], 22, "expected handshake record");
    let body = &record[RECORD_HDR..];
    assert_eq!(body[0], 1, "expected ClientHello");
    let mut random = [0u8; 32];
    random.copy_from_slice(&body[6..38]);
    let mut pos = 38;
    let sid_len = body[pos] as usize;
    pos += 1;
    let session_id = body[pos..pos + sid_len].to_vec();
    pos += sid_len;
    let cs_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2 + cs_len;
    let comp_len = body[pos] as usize;
    pos += 1 + comp_len;
    let ext_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2;
    let exts = &body[pos..pos + ext_len];
    let mut ep = 0;
    let mut server_name = String::new();
    let mut x25519_share = [0u8; 32];
    while ep + 4 <= exts.len() {
        let typ = u16::from_be_bytes([exts[ep], exts[ep + 1]]);
        let len = u16::from_be_bytes([exts[ep + 2], exts[ep + 3]]) as usize;
        let data = &exts[ep + 4..ep + 4 + len];
        match typ {
            0 => {
                // SNI: list_len(2) + type(0) + name_len(2) + name
                let name_len = u16::from_be_bytes([data[3], data[4]]) as usize;
                server_name = String::from_utf8_lossy(&data[5..5 + name_len]).into_owned();
            }
            51 => {
                let mut sp = 2;
                while sp + 4 <= data.len() {
                    let group = u16::from_be_bytes([data[sp], data[sp + 1]]);
                    let klen = u16::from_be_bytes([data[sp + 2], data[sp + 3]]) as usize;
                    if group == 0x001d && klen == 32 {
                        x25519_share.copy_from_slice(&data[sp + 4..sp + 4 + klen]);
                    }
                    sp += 4 + klen;
                }
            }
            _ => {}
        }
        ep += 4 + len;
    }
    ParsedCH {
        wire: body.to_vec(),
        random,
        session_id,
        server_name,
        x25519_share,
    }
}

/// Server side after a completed handshake — application traffic
/// secrets retained so fixtures can rotate epochs (KeyUpdate).
struct JlsServerSide {
    tcp: TcpStream,
    c_ap: RecordCipher,
    s_ap: RecordCipher,
    c_ap_secret: Vec<u8>,
    s_ap_secret: Vec<u8>,
}

/// jls server fixture: verify the client's sealed random, answer with our
/// own, run the real TLS 1.3 flight, return the post-handshake state.
/// `corrupt_sid` echoes a bogus session_id — exercises the client's
/// RFC 8446 §4.1.3 compat-mode echo check. `tail` appends extra handshake
/// bytes into the Finished record — coalesced post-handshake messages
/// (e.g. KeyUpdate) must reach the client via `leftover_handshake`.
async fn jls_handshake(
    tcp: TcpStream,
    cert_der: &[u8],
    corrupt_sid: bool,
    tail: &[u8],
) -> io::Result<JlsServerSide> {
    let mut tcp = tcp;
    // 1. ClientHello — verify the JLS auth blob in `random`.
    let rec = read_record(&mut tcp).await?;
    let ch = parse_client_hello(&rec);
    assert_eq!(ch.server_name, "cover.example.com");
    let mut auth_data = ch.wire.clone();
    auth_data[6..38].fill(0);
    assert!(
        jls_open_random(USERNAME, PASSWORD, &auth_data, &ch.random).is_some(),
        "client random failed jls auth"
    );

    // 2. Client compat CCS.
    let ccs = read_record(&mut tcp).await?;
    assert_eq!(ccs[0], 20);

    // 3. ServerHello with our own sealed random (X25519, AES128-SHA256).
    let secret = x25519_dalek::StaticSecret::from(rand::random::<[u8; 32]>());
    let public = x25519_dalek::PublicKey::from(&secret);
    let shared = secret
        .diffie_hellman(&x25519_dalek::PublicKey::from(ch.x25519_share))
        .to_bytes();

    let mut sh_body = Vec::new();
    sh_body.extend_from_slice(&[0x03, 0x03]); // legacy_version
    sh_body.extend_from_slice(&[0u8; 32]); // random placeholder
    let echo_sid = if corrupt_sid {
        vec![0xff; ch.session_id.len()]
    } else {
        ch.session_id.clone()
    };
    sh_body.push(echo_sid.len() as u8);
    sh_body.extend_from_slice(&echo_sid);
    sh_body.extend_from_slice(&[0x13, 0x01]); // TLS_AES_128_GCM_SHA256
    sh_body.push(0); // compression
    let mut sh_exts = Vec::new();
    sh_exts.extend_from_slice(&[0, 43, 0, 2, 0x03, 0x04]); // supported_versions
    sh_exts.extend_from_slice(&[0, 51, 0, 36, 0, 0x1d, 0, 32]); // key_share
    sh_exts.extend_from_slice(public.as_bytes());
    sh_body.extend_from_slice(&(sh_exts.len() as u16).to_be_bytes());
    sh_body.extend_from_slice(&sh_exts);
    let mut sh_wire = vec![0x02];
    put_u24(sh_body.len(), &mut sh_wire);
    sh_wire.extend_from_slice(&sh_body);
    // Server authData = SH wire bytes with random zeroed.
    let mut sh_auth = sh_wire.clone();
    sh_auth[6..38].fill(0);
    let fake = jls_seal_random(USERNAME, PASSWORD, &sh_auth);
    sh_wire[6..38].copy_from_slice(&fake);

    let mut transcript = ch.wire.clone();
    transcript.extend_from_slice(&sh_wire);

    // 4. Key schedule (SHA-256 / AES-128-GCM only).
    let zero = vec![0u8; 32];
    let empty_hash = Sha256::digest([]);
    let early = hkdf_extract(&zero, &zero);
    let derived = expand_label(&early, "derived", &empty_hash, 32);
    let hs_secret = hkdf_extract(&derived, &shared);
    let c_hs_secret = derive_secret(&hs_secret, "c hs traffic", &transcript);
    let s_hs_secret = derive_secret(&hs_secret, "s hs traffic", &transcript);
    let derived2 = expand_label(&hs_secret, "derived", &empty_hash, 32);
    let master = hkdf_extract(&derived2, &zero);
    let mut c_hs = RecordCipher::new(&c_hs_secret);
    let mut s_hs = RecordCipher::new(&s_hs_secret);

    // 5. SH record + compat CCS + encrypted EE/Cert/CV/Finished.
    let mut sh_rec = vec![0x16, 0x03, 0x01];
    sh_rec.extend_from_slice(&(sh_wire.len() as u16).to_be_bytes());
    sh_rec.extend_from_slice(&sh_wire);
    tcp.write_all(&sh_rec).await?;
    tcp.write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01]).await?;

    let ee = [0x08, 0x00, 0x00, 0x02, 0x00, 0x00];
    transcript.extend_from_slice(&ee);
    let mut cert_body = vec![0x00]; // empty request context
    cert_body.extend_from_slice(&[0, 0, 0]); // certificate_list len — patched
    let mut cert_entry = Vec::new();
    put_u24(cert_der.len(), &mut cert_entry);
    cert_entry.extend_from_slice(cert_der);
    cert_entry.extend_from_slice(&[0, 0]); // no extensions
    let list_len = cert_entry.len();
    cert_body.extend_from_slice(&cert_entry);
    cert_body[1..4].copy_from_slice(&[
        (list_len >> 16) as u8,
        (list_len >> 8) as u8,
        list_len as u8,
    ]);
    let mut cert_msg = vec![0x0b];
    put_u24(cert_body.len(), &mut cert_msg);
    cert_msg.extend_from_slice(&cert_body);
    transcript.extend_from_slice(&cert_msg);
    // Authenticated clients skip CV verification — the signature is a
    // placeholder under a plausible scheme.
    let mut cv_body = vec![0x04, 0x03]; // ecdsa_secp256r1_sha256
    cv_body.extend_from_slice(&64u16.to_be_bytes());
    cv_body.extend_from_slice(&[0x5a; 64]);
    let mut cv = vec![0x0f];
    put_u24(cv_body.len(), &mut cv);
    cv.extend_from_slice(&cv_body);
    transcript.extend_from_slice(&cv);
    let fin_key = expand_label(&s_hs_secret, "finished", &[], 32);
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&fin_key).unwrap();
    m.update(Sha256::digest(&transcript).as_slice());
    let vdata = m.finalize().into_bytes();
    let mut fin = vec![0x14];
    put_u24(vdata.len(), &mut fin);
    fin.extend_from_slice(&vdata);
    transcript.extend_from_slice(&fin);

    let mut fin_rec = fin.clone();
    fin_rec.extend_from_slice(tail);
    for msg in [&ee[..], &cert_msg[..], &cv[..], &fin_rec[..]] {
        let r = s_hs.seal(22, msg);
        tcp.write_all(&r).await?;
    }
    tcp.flush().await?;

    // 6. Client Finished — transcript through the server Finished is what
    // both the client verify_data and the app secrets bind to.
    let c_ap_secret = derive_secret(&master, "c ap traffic", &transcript);
    let s_ap_secret = derive_secret(&master, "s ap traffic", &transcript);
    let fin_key_c = expand_label(&c_hs_secret, "finished", &[], 32);
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&fin_key_c).unwrap();
    m.update(Sha256::digest(&transcript).as_slice());
    let expect_fin = m.finalize().into_bytes();
    loop {
        let rec = read_record(&mut tcp).await?;
        if rec[0] == 20 {
            continue;
        }
        let (typ, body) = c_hs.open(&rec).expect("client Finished decrypt");
        assert_eq!(typ, 22);
        assert_eq!(&body[..4], &[0x14, 0x00, 0x00, 0x20]);
        assert_eq!(
            &body[4..],
            expect_fin.as_slice(),
            "client Finished mismatch"
        );
        break;
    }

    Ok(JlsServerSide {
        tcp,
        c_ap: RecordCipher::new(&c_ap_secret),
        s_ap: RecordCipher::new(&s_ap_secret),
        c_ap_secret,
        s_ap_secret,
    })
}

/// Plain echo under the application traffic keys until EOF.
async fn jls_server(tcp: TcpStream, cert_der: &[u8]) -> io::Result<()> {
    let mut s = jls_handshake(tcp, cert_der, false, &[]).await?;
    loop {
        let Ok(rec) = read_record(&mut s.tcp).await else {
            return Ok(());
        };
        if rec[0] == 20 {
            continue;
        }
        let Some((typ, body)) = s.c_ap.open(&rec) else {
            continue;
        };
        if typ != 23 {
            continue;
        }
        let r = s.s_ap.seal(23, &body);
        s.tcp.write_all(&r).await?;
        s.tcp.flush().await?;
    }
}

/// Rotate an application traffic secret per RFC 8446 §4.6.3.
fn traffic_update(secret: &[u8]) -> Vec<u8> {
    expand_label(secret, "traffic upd", &[], 32)
}

fn client_cfg(cert_der: Option<&rustls::pki_types::CertificateDer<'static>>) -> JlsConfig {
    JlsConfig {
        server_name: "cover.example.com".to_string(),
        username: USERNAME.to_string(),
        password: PASSWORD.to_string(),
        // The production default (upstream h2,http/1.1) — `alpn` empty
        // means "no ALPN extension" since the parser owns the default.
        alpn: vec!["h2".to_string(), "http/1.1".to_string()],
        additional_roots: cert_der
            .map(|c| vec![c.as_ref().to_vec()])
            .unwrap_or_default(),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

macro_rules! timed_test {
    ($name:ident, $imp:ident) => {
        #[tokio::test]
        async fn $name() {
            install_crypto_provider();
            tokio::time::timeout(TEST_TIMEOUT, $imp())
                .await
                .expect("jls e2e timed out");
        }
    };
}

/// Authed path: mini jls server → real handshake → echo. `additional_roots`
/// is deliberately empty: the authed path must not PKI-verify the
/// camouflage cert.
async fn e2e_authed_data_path_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        jls_server(tcp, cert_der.as_ref()).await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    stream.write_all(b"jls-ping").await.unwrap();
    let mut buf = [0u8; 8];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"jls-ping");
    drop(stream);
    server.await.unwrap();
}

/// Unauthed path: real rustls cover — the handshake completes, then the
/// client rejects it (upstream `ErrJLSAuthFailed`).
async fn e2e_unauthed_fails_impl() {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    let mut conn = ServerConnection::new(Arc::new(cfg)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        loop {
            while conn.wants_write() {
                let mut out = Vec::new();
                conn.write_tls(&mut out).unwrap();
                tcp.write_all(&out).await.unwrap();
            }
            if !conn.is_handshaking() {
                return;
            }
            let mut buf = [0u8; 8192];
            let n = tcp.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            conn.read_tls(&mut &buf[..n]).unwrap();
            conn.process_new_packets().unwrap();
        }
    });
    let err = jls::dial(
        TcpStream::connect(addr).await.unwrap(),
        &client_cfg(Some(&cert_der)),
    )
    .await
    .err()
    .expect("unauthed jls dial must fail");
    assert!(
        err.to_string().contains("authentication failed"),
        "unexpected error: {err}"
    );
    server.await.unwrap();
}

/// KeyUpdate(update_requested): the server rotates and then goes quiet —
/// the client must flush its KeyUpdate(0) response while parked on read,
/// and both directions must keep working under the rotated epochs.
async fn e2e_keyupdate_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        // KU(update_requested) sealed under the *current* write epoch —
        // the write key rotates only after the KU itself is sent.
        let rec = s.s_ap.seal(22, &[0x18, 0x00, 0x00, 0x01, 0x01]);
        s.tcp.write_all(&rec).await.unwrap();
        s.tcp.flush().await.unwrap();
        s.s_ap_secret = traffic_update(&s.s_ap_secret);
        s.s_ap = RecordCipher::new(&s.s_ap_secret);
        // Go quiet: the client's KeyUpdate(0) must arrive without more
        // server writes (it would be stuck in outbox if poll_read parked
        // without flushing).
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("client KeyUpdate(0) open");
        assert_eq!(typ, 22);
        assert_eq!(&body[..], &[0x18, 0x00, 0x00, 0x01, 0x00]);
        s.c_ap_secret = traffic_update(&s.c_ap_secret);
        s.c_ap = RecordCipher::new(&s.c_ap_secret);
        // Both directions under the new epochs.
        let r = s.s_ap.seal(23, b"rotated");
        s.tcp.write_all(&r).await.unwrap();
        s.tcp.flush().await.unwrap();
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("client ping open");
        assert_eq!(typ, 23);
        assert_eq!(&body[..], b"epoch2-ping");
        let r = s.s_ap.seal(23, &body);
        s.tcp.write_all(&r).await.unwrap();
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    // Parked on this read when the KU arrives — its response must flush.
    let mut buf = [0u8; 7];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"rotated");
    stream.write_all(b"epoch2-ping").await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 11];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"epoch2-ping");
    drop(stream);
    server.await.unwrap();
}

/// A TCP close mid-record surfaces as an error, not a silent EOF.
async fn e2e_mid_record_eof_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        let rec = s.s_ap.seal(23, b"cut-me-off");
        let half = rec.len() / 2;
        s.tcp.write_all(&rec[..half]).await.unwrap();
        s.tcp.flush().await.unwrap();
        // Abrupt close mid-record.
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 16];
    let err = stream
        .read(&mut buf)
        .await
        .expect_err("mid-record EOF must error");
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    server.await.unwrap();
}

/// close_notify maps to a clean EOF; writes still work (half-close).
async fn e2e_close_notify_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        let rec = s.s_ap.seal(21, &[0x01, 0x00]); // warning + close_notify
        s.tcp.write_all(&rec).await.unwrap();
        s.tcp.flush().await.unwrap();
        // Hold the socket open so the read resolves via the alert, not a
        // bare TCP EOF — then read one more client write to prove the
        // write direction survives.
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("post-close ping open");
        assert_eq!(typ, 23);
        assert_eq!(&body[..], b"still-works");
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 8];
    let n = stream.read(&mut buf).await.unwrap();
    assert_eq!(n, 0, "close_notify must read as EOF");
    stream.write_all(b"still-works").await.unwrap();
    stream.flush().await.unwrap();
    drop(stream);
    server.await.unwrap();
}

/// Wrong password vs a plain rustls cover: upstream behavior on failed
/// credentials — the server cannot open the sealed random so it answers
/// as an ordinary TLS server (unauthed SH.random), the client completes
/// the *full* PKI-verified handshake, then rejects with the jls auth
/// error. This exercises the client's SH-auth rejection with bad
/// credentials — `e2e_unauthed_fails` covers only "no credentials".
async fn e2e_wrong_password_fails_impl() {
    let (cert_der, key_der, _, _) = gen_cert(&["cover.example.com"]);
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");
    let mut conn = ServerConnection::new(Arc::new(cfg)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        loop {
            while conn.wants_write() {
                let mut out = Vec::new();
                conn.write_tls(&mut out).unwrap();
                tcp.write_all(&out).await.unwrap();
            }
            if !conn.is_handshaking() {
                return;
            }
            let mut buf = [0u8; 8192];
            let n = tcp.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            conn.read_tls(&mut &buf[..n]).unwrap();
            conn.process_new_packets().unwrap();
        }
    });
    let mut cfg = client_cfg(Some(&cert_der));
    cfg.password = "wrong".to_string();
    let err = jls::dial(TcpStream::connect(addr).await.unwrap(), &cfg)
        .await
        .err()
        .expect("wrong-password jls dial must fail");
    assert!(
        err.to_string().contains("authentication failed"),
        "wrong credentials must surface the jls auth rejection, got: {err}"
    );
    server.await.unwrap();
}

/// A KeyUpdate coalesced into the server Finished record arrives via
/// `leftover_handshake` — `JlsStream::new` must seed it into the
/// post-handshake walker or the read epoch desyncs.
async fn e2e_coalesced_keyupdate_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // KU(update_requested) spliced into the Finished record — the
        // client's JlsStream::new must process it from the leftover.
        let mut s = jls_handshake(
            tcp,
            cert_der.as_ref(),
            false,
            &[0x18, 0x00, 0x00, 0x01, 0x01],
        )
        .await
        .unwrap();
        // Server rotated its send epoch on sending KU.
        s.s_ap_secret = traffic_update(&s.s_ap_secret);
        s.s_ap = RecordCipher::new(&s.s_ap_secret);
        // The client's staged KU(0) response flushes on its first poll.
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("client KeyUpdate(0) open");
        assert_eq!(typ, 22);
        assert_eq!(&body[..], &[0x18, 0x00, 0x00, 0x01, 0x00]);
        s.c_ap_secret = traffic_update(&s.c_ap_secret);
        s.c_ap = RecordCipher::new(&s.c_ap_secret);
        // Appdata under the new epoch must read cleanly client-side.
        let r = s.s_ap.seal(23, b"epoch1-data");
        s.tcp.write_all(&r).await.unwrap();
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 11];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"epoch1-data");
    server.await.unwrap();
}

/// KeyUpdate(update_not_requested): the server rotates only its send
/// epoch — the client's read key must rekey and its write key must NOT
/// (no response record is owed). Both directions then work under their
/// respective epochs.
async fn e2e_ku_update_not_requested_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        // KU(0) sealed under the current epoch; server's send key rotates.
        let rec = s.s_ap.seal(22, &[0x18, 0x00, 0x00, 0x01, 0x00]);
        s.tcp.write_all(&rec).await.unwrap();
        s.tcp.flush().await.unwrap();
        s.s_ap_secret = traffic_update(&s.s_ap_secret);
        s.s_ap = RecordCipher::new(&s.s_ap_secret);
        let r = s.s_ap.seal(23, b"rotated");
        s.tcp.write_all(&r).await.unwrap();
        s.tcp.flush().await.unwrap();
        // Client must NOT have sent a KeyUpdate — its next record is
        // appdata under the *unrotated* client epoch.
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("client reply opens old epoch");
        assert_eq!(typ, 23);
        assert_eq!(&body[..], b"still-epoch1");
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 7];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"rotated");
    stream.write_all(b"still-epoch1").await.unwrap();
    stream.flush().await.unwrap();
    server.await.unwrap();
}

/// A KeyUpdate with a body other than [0]/[1] is a fatal error, not a
/// skip (upstream `unexpected_message` parity — an unrecognized KU
/// request would desync the epoch).
async fn e2e_malformed_keyupdate_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        let rec = s.s_ap.seal(22, &[0x18, 0x00, 0x00, 0x01, 0x02]);
        s.tcp.write_all(&rec).await.unwrap();
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 8];
    let err = stream
        .read(&mut buf)
        .await
        .expect_err("malformed KU must fail");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    server.await.unwrap();
}

/// Non-KeyUpdate post-handshake handshake messages (NewSessionTicket and
/// friends) are consumed and skipped — the stream keeps delivering data.
async fn e2e_nst_skipped_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        // NewSessionTicket (typ 4) with a small plausible body.
        let nst = [
            0x04, 0x00, 0x00, 0x0a, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
        ];
        let rec = s.s_ap.seal(22, &nst);
        s.tcp.write_all(&rec).await.unwrap();
        let r = s.s_ap.seal(23, b"after-nst");
        s.tcp.write_all(&r).await.unwrap();
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 9];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"after-nst");
    server.await.unwrap();
}

/// A fatal alert maps to ConnectionAborted; a flood of non-advancing
/// records (stray CCS) trips the useless-record bound instead of
/// livelocking `poll_read` inside one poll.
async fn e2e_fatal_alert_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        let rec = s.s_ap.seal(21, &[0x02, 0x28]); // fatal + handshake_failure
        s.tcp.write_all(&rec).await.unwrap();
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 8];
    let err = stream
        .read(&mut buf)
        .await
        .expect_err("fatal alert must fail");
    assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted);
    server.await.unwrap();
}

async fn e2e_useless_records_bounded_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        // 40 plaintext CCS records — legal noise, but past the 32 bound.
        for _ in 0..40 {
            s.tcp
                .write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01])
                .await
                .unwrap();
        }
        s.tcp.flush().await.unwrap();
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    let mut buf = [0u8; 8];
    let err = stream
        .read(&mut buf)
        .await
        .expect_err("CCS flood must trip the useless bound");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    server.await.unwrap();
}

/// `shutdown()` emits a sealed close_notify ([1,0]); a write afterwards
/// is BrokenPipe, matching real TLS stack semantics.
async fn e2e_shutdown_close_notify_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut s = jls_handshake(tcp, cert_der.as_ref(), false, &[])
            .await
            .unwrap();
        let rec = read_record(&mut s.tcp).await.unwrap();
        let (typ, body) = s.c_ap.open(&rec).expect("client close_notify open");
        assert_eq!(typ, 21);
        assert_eq!(&body[..], &[0x01, 0x00]);
    });
    let mut stream = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .expect("jls dial");
    stream.shutdown().await.unwrap();
    let err = stream
        .write_all(b"after-close")
        .await
        .expect_err("write after close_notify must be BrokenPipe");
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    server.await.unwrap();
}

/// A ServerHello that does not echo our compat session_id is rejected by
/// the shared driver's RFC 8446 §4.1.3 check — before the jls auth check.
async fn e2e_session_id_mismatch_impl() {
    let (cert_der, _key, _, _) = gen_cert(&["cover.example.com"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // The dial errors on the bogus echo; the fixture's CH auth assert
        // already passed, so it just dies mid-flight.
        let _ = jls_handshake(tcp, cert_der.as_ref(), true, &[]).await;
    });
    let err = jls::dial(TcpStream::connect(addr).await.unwrap(), &client_cfg(None))
        .await
        .err()
        .expect("sid-mismatch jls dial must fail");
    assert!(
        err.to_string().contains("did not echo the session_id"),
        "unexpected error: {err}"
    );
    server.await.unwrap();
}

/// Real-peer interop against the upstream jls server
/// (`github.com/metacubex/jls-tls` — a `crypto/tls` fork with
/// `JLSConfig`). The Go side authenticates our sealed `ClientHello.random`,
/// answers with its own sealed `ServerHello.random`, and echoes plaintext —
/// a green round trip proves both directions against code we did not
/// write, not self-consistency.
///
/// Gated on `$JLS_SERVER_BIN`; fails when unset so a green run always
/// exercised real-peer interop (`MEOW_JLS_E2E_ALLOW_SKIP=1` prints a loud
/// explicit skip for local runs only — CI must never set it). Build the
/// harness from `tests/support/jls-server/`:
/// `(cd tests/support/jls-server && go build -o jls-server .)`
async fn e2e_upstream_interop_impl() {
    use std::process::Stdio;
    let Some(bin) = std::env::var_os("JLS_SERVER_BIN") else {
        if std::env::var_os("MEOW_JLS_E2E_ALLOW_SKIP").is_some() {
            eprintln!(
                "SKIP: JLS_SERVER_BIN unset and MEOW_JLS_E2E_ALLOW_SKIP \
                 is set — upstream interop NOT exercised"
            );
            return;
        }
        panic!(
            "JLS_SERVER_BIN is required for the upstream-interop leg \
             (build tests/support/jls-server and point it at the binary). \
             A green run must exercise real-peer interop, so this test \
             refuses to silently skip — set MEOW_JLS_E2E_ALLOW_SKIP=1 \
             only for a loud local skip."
        );
    };
    let mut child = tokio::process::Command::new(&bin)
        .args([
            "-listen",
            "127.0.0.1:0",
            "-username",
            USERNAME,
            "-password",
            PASSWORD,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn jls server");
    let mut stdout = child.stdout.take().expect("stdout piped");
    // First line: "LISTEN=127.0.0.1:PORT"
    let mut line = String::new();
    tokio::io::AsyncBufReadExt::read_line(&mut tokio::io::BufReader::new(&mut stdout), &mut line)
        .await
        .expect("read banner");
    let listen = line
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("LISTEN="))
        .expect("LISTEN= in server banner")
        .to_string();

    let tcp = TcpStream::connect(&listen).await.unwrap();
    // No additional_roots: on the authenticated path the camouflage cert
    // is a throwaway and must not be PKI-verified.
    let mut s = jls::dial(tcp, &client_cfg(None))
        .await
        .unwrap_or_else(|e| panic!("jls dial vs upstream: {e}"));
    for round in 0..2 {
        let payload = format!("upstream-round{round}");
        s.write_all(payload.as_bytes()).await.unwrap();
        s.flush().await.unwrap();
        let mut buf = vec![0u8; payload.len()];
        s.read_exact(&mut buf)
            .await
            .unwrap_or_else(|e| panic!("jls echo round{round}: {e}"));
        assert_eq!(&buf, payload.as_bytes(), "echo mismatch");
    }
    drop(child);
}

timed_test!(e2e_authed_data_path, e2e_authed_data_path_impl);
timed_test!(e2e_unauthed_fails, e2e_unauthed_fails_impl);
timed_test!(e2e_wrong_password_fails, e2e_wrong_password_fails_impl);
timed_test!(e2e_keyupdate, e2e_keyupdate_impl);
timed_test!(e2e_mid_record_eof, e2e_mid_record_eof_impl);
timed_test!(e2e_close_notify, e2e_close_notify_impl);
timed_test!(e2e_session_id_mismatch, e2e_session_id_mismatch_impl);
timed_test!(e2e_coalesced_keyupdate, e2e_coalesced_keyupdate_impl);
timed_test!(
    e2e_ku_update_not_requested,
    e2e_ku_update_not_requested_impl
);
timed_test!(e2e_malformed_keyupdate, e2e_malformed_keyupdate_impl);
timed_test!(e2e_nst_skipped, e2e_nst_skipped_impl);
timed_test!(e2e_fatal_alert, e2e_fatal_alert_impl);
timed_test!(
    e2e_useless_records_bounded,
    e2e_useless_records_bounded_impl
);
timed_test!(e2e_shutdown_close_notify, e2e_shutdown_close_notify_impl);
timed_test!(e2e_upstream_interop, e2e_upstream_interop_impl);
