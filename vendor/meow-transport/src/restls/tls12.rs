//! restls over TLS 1.2 — the `version-hint=tls12` path: eager ECDHE keys for
//! X25519/P-256/P-384 committed into the session_id (`restls12ClientAuthLayout3`),
//! a real ECDHE+AES-GCM cover handshake, then tagged data records.
//!
//! Upstream never resumes (`sessionTicket` is empty), so only layout3 is
//! implemented. The record protection is RFC 5288 AES-GCM with explicit
//! nonces; the restls tagged-record layer rides the nonce slot
//! (`restls12WithGCM` / `restls12GCMServerDisableCtr`).

use std::collections::VecDeque;

use tokio::io::AsyncWriteExt;

use crate::restls::conn::{CoverCipher, RestlsUpgraded};
use crate::restls::wire;
use crate::{Result, Stream, TransportError};

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce, Tag};

use super::tls13::{self, CertPolicy, HashAlg};

const HS_SERVER_HELLO: u8 = 2;
const HS_CERTIFICATE: u8 = 11;
const HS_SERVER_KEY_EXCHANGE: u8 = 12;
const HS_CERTIFICATE_REQUEST: u8 = 13;
const HS_SERVER_HELLO_DONE: u8 = 14;
const HS_CLIENT_KEY_EXCHANGE: u8 = 16;
const HS_NEW_SESSION_TICKET: u8 = 4;
const HS_CERTIFICATE_STATUS: u8 = 22;
const HS_FINISHED: u8 = 20;

const TLS_ECDHE_RSA_AES128_GCM: u16 = 0xc02f;
const TLS_ECDHE_ECDSA_AES128_GCM: u16 = 0xc02b;
const TLS_ECDHE_RSA_AES256_GCM: u16 = 0xc030;
const TLS_ECDHE_ECDSA_AES256_GCM: u16 = 0xc02c;

const GROUP_X25519: u16 = 0x001d;
const GROUP_P256: u16 = 0x0017;
const GROUP_P384: u16 = 0x0018;

/// `restls12ClientAuthLayout3` — 32-byte session_id split across three eager
/// ECDHE public keys: tag spans `[0..11]`, `[11..22]`, `[22..32]`.
const AUTH_LAYOUT3: [usize; 4] = [0, 11, 22, 32];

/// Bound on the server flight before ServerHelloDone.
const MAX_SERVER_FLIGHT: usize = 1 << 20;

/// restls TLS 1.2 configuration — resolved options.
pub(crate) struct Tls12Config {
    /// Cover SNI (`host`).
    pub(crate) server_name: String,
    /// Certificate verification policy.
    pub(crate) cert: CertPolicy,
}

/// TLS 1.2 cipher suite (ECDHE + AES-GCM only — covers without GCM cannot
/// carry the restls nonce-slot protocol anyway).
#[derive(Clone, Copy)]
enum Cipher12 {
    RsaAes128,
    EcdsaAes128,
    RsaAes256,
    EcdsaAes256,
}

impl Cipher12 {
    fn try_from(value: u16) -> Result<Self> {
        match value {
            TLS_ECDHE_RSA_AES128_GCM => Ok(Self::RsaAes128),
            TLS_ECDHE_ECDSA_AES128_GCM => Ok(Self::EcdsaAes128),
            TLS_ECDHE_RSA_AES256_GCM => Ok(Self::RsaAes256),
            TLS_ECDHE_ECDSA_AES256_GCM => Ok(Self::EcdsaAes256),
            other => Err(TransportError::Tls(format!(
                "restls12: unsupported cipher 0x{other:04x}"
            ))),
        }
    }

    /// PRF hash — TLS 1.2 GCM suites bind it to the cipher.
    fn hash(self) -> HashAlg {
        match self {
            Self::RsaAes128 | Self::EcdsaAes128 => HashAlg::Sha256,
            Self::RsaAes256 | Self::EcdsaAes256 => HashAlg::Sha384,
        }
    }

    fn key_len(self) -> usize {
        match self {
            Self::RsaAes128 | Self::EcdsaAes128 => 16,
            Self::RsaAes256 | Self::EcdsaAes256 => 32,
        }
    }
}

/// TLS 1.2 PRF — P_hash(secret, label || seed).
fn prf(hash: HashAlg, secret: &[u8], label: &[u8], seed: &[u8], out_len: usize) -> Vec<u8> {
    let mut full_seed = Vec::with_capacity(label.len() + seed.len());
    full_seed.extend_from_slice(label);
    full_seed.extend_from_slice(seed);
    let mut out = Vec::with_capacity(out_len);
    let mut a = hash.hmac(secret, &full_seed);
    while out.len() < out_len {
        let mut input = a.clone();
        input.extend_from_slice(&full_seed);
        out.extend_from_slice(&hash.hmac(secret, &input));
        a = hash.hmac(secret, &a);
    }
    out.truncate(out_len);
    out
}

/// TLS 1.2 AEAD record key — RFC 5288 AES-GCM with an 8-byte explicit nonce.
pub(crate) struct GcmAead {
    cipher: Cipher12,
    key: Vec<u8>,
    /// 4-byte salt from the key block (prepended to the explicit nonce).
    salt: [u8; 4],
    /// Record sequence number (drives the AAD).
    seq: u64,
    /// Explicit nonce counter for sealing.
    write_ctr: u64,
}

impl GcmAead {
    fn new(cipher: Cipher12, key: &[u8], salt: &[u8]) -> Self {
        let mut salt4 = [0u8; 4];
        salt4.copy_from_slice(salt);
        Self {
            cipher,
            key: key.to_vec(),
            salt: salt4,
            seq: 0,
            write_ctr: 0,
        }
    }

    fn nonce(&self, explicit: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.salt);
        nonce[4..].copy_from_slice(&explicit.to_be_bytes());
        nonce
    }

    /// TLS 1.2 AEAD additional data — upstream uses the record's own
    /// version bytes (`record[1..3]`), not a hardcoded 0x0303.
    fn aad(&self, seq: u64, typ: u8, version: [u8; 2], len: usize) -> [u8; 13] {
        let mut aad = [0u8; 13];
        aad[..8].copy_from_slice(&seq.to_be_bytes());
        aad[8] = typ;
        aad[9] = version[0];
        aad[10] = version[1];
        aad[11..].copy_from_slice(&(len as u16).to_be_bytes());
        aad
    }

    /// Seal one plaintext as a TLS 1.2 record of type `typ`.
    fn seal(&mut self, typ: u8, plaintext: &[u8]) -> Result<Vec<u8>> {
        let explicit = self.write_ctr;
        self.write_ctr += 1;
        let nonce = self.nonce(explicit);
        let mut body = plaintext.to_vec();
        let aad = self.aad(self.seq, typ, [0x03, 0x03], plaintext.len());
        let tag = match self.cipher {
            Cipher12::RsaAes128 | Cipher12::EcdsaAes128 => Aes128Gcm::new_from_slice(&self.key)
                .expect("key")
                .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut body),
            Cipher12::RsaAes256 | Cipher12::EcdsaAes256 => Aes256Gcm::new_from_slice(&self.key)
                .expect("key")
                .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut body),
        }
        .map_err(|e| TransportError::Tls(format!("restls12 seal: {e}")))?;
        self.seq += 1;
        let mut out = Vec::with_capacity(5 + 8 + body.len() + 16);
        out.push(typ);
        out.extend_from_slice(&[0x03, 0x03]);
        let len = 8 + body.len() + 16;
        out.extend_from_slice(&(len as u16).to_be_bytes());
        out.extend_from_slice(&explicit.to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    /// Open a record — `record` includes the 5-byte header. The explicit
    /// nonce may be rewritten by the restls layer before this call.
    fn open(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if record.len() < 5 + 8 + 16 {
            return Err(TransportError::Tls("restls12: short record".into()));
        }
        let typ = record[0];
        let len = u16::from_be_bytes([record[3], record[4]]) as usize;
        if record.len() != 5 + len {
            return Err(TransportError::Tls(
                "restls12: record length mismatch".into(),
            ));
        }
        let explicit = u64::from_be_bytes(record[5..13].try_into().unwrap());
        let body_len = len - 8 - 16;
        let mut body = record[13..13 + body_len].to_vec();
        let tag = record[13 + body_len..].to_vec();
        let nonce = self.nonce(explicit);
        let version = [record[1], record[2]];
        let aad = self.aad(self.seq, typ, version, body_len);
        let tagref = Tag::from_slice(&tag);
        match self.cipher {
            Cipher12::RsaAes128 | Cipher12::EcdsaAes128 => Aes128Gcm::new_from_slice(&self.key)
                .expect("key")
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut body, tagref),
            Cipher12::RsaAes256 | Cipher12::EcdsaAes256 => Aes256Gcm::new_from_slice(&self.key)
                .expect("key")
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut body, tagref),
        }
        .map_err(|e| TransportError::Tls(format!("restls12 open: {e}")))?;
        self.seq += 1;
        Ok(body)
    }

    /// Post-handshake sequence — the next nonce a seq-derived cover will
    /// emit. The restls layer seeds its real-record counter from this so
    /// covers that sent NewSessionTicket before Finished rewrite correctly.
    fn seq(&self) -> u64 {
        self.seq
    }
}

impl CoverCipher for GcmAead {
    /// Post-handshake cover records: the record's nonce slot already carries
    /// the running counter rewritten by `accept_cover_record` — decrypt as-is.
    fn open(&mut self, record: &mut [u8]) -> Option<(u8, Vec<u8>)> {
        let typ = record.first().copied()?;
        self.open(record).ok().map(|body| (typ, body))
    }

    fn seal(&mut self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in body.chunks(wire::MAX_PLAINTEXT) {
            match self.seal(wire::TLS_RECORD_APPLICATION_DATA, chunk) {
                Ok(rec) => out.extend_from_slice(&rec),
                Err(_) => break,
            }
        }
        out
    }

    fn seal_close_notify(&mut self) -> Vec<u8> {
        // [warning, close_notify] — a real alert-typed TLS 1.2 record.
        self.seal(wire::TLS_RECORD_ALERT, &[0x01, 0x00])
            .unwrap_or_default()
    }
}

/// Eager ECDHE keys — generated before the ClientHello so their public keys
/// commit into the session_id (`generateSessionIDForTLS12`). One per group.
fn generate_eager_keys() -> Result<Vec<tls13::KeyShare>> {
    [GROUP_X25519, GROUP_P256, GROUP_P384]
        .into_iter()
        .map(tls13::KeyShare::generate)
        .collect()
}

/// TLS 1.2 ClientHello — `supported_versions` is absent (this is a real 1.2
/// hello); the session_id carries the layout3 eager-key tags.
fn build_client_hello(
    cfg: &Tls12Config,
    random: &[u8; 32],
    keys: &[tls13::KeyShare],
    secret: &[u8; 32],
) -> Result<Vec<u8>> {
    let mut session_id = [0u8; 32];
    for (i, key) in keys.iter().enumerate() {
        let mut h = wire::restls_hasher(secret);
        h.update(&key.public);
        let tag = h.finalize();
        let (lo, hi) = (AUTH_LAYOUT3[i], AUTH_LAYOUT3[i + 1]);
        session_id[lo..hi].copy_from_slice(&tag.as_bytes()[..hi - lo]);
    }

    let ciphers = [
        TLS_ECDHE_RSA_AES128_GCM,
        TLS_ECDHE_ECDSA_AES128_GCM,
        TLS_ECDHE_RSA_AES256_GCM,
        TLS_ECDHE_ECDSA_AES256_GCM,
    ];

    let mut body = Vec::with_capacity(256);
    body.extend_from_slice(&[0x03, 0x03]); // client_version
    body.extend_from_slice(random);
    body.push(session_id.len() as u8);
    body.extend_from_slice(&session_id);
    put_u16((ciphers.len() * 2) as u16, &mut body);
    for c in ciphers {
        put_u16(c, &mut body);
    }
    body.extend_from_slice(&[1, 0]); // null compression

    // Extension set + order mirrors Go's clientHelloMsg.marshal for a
    // TLS 1.2 hello (upstream's native path): SNI, status_request,
    // supported_groups, ec_point_formats, signature_algorithms,
    // renegotiation_info, ALPN, SCT, session_ticket, EMS.
    let mut exts = Vec::new();
    tls13::push_ext(&mut exts, 0, &tls13::server_name_ext(&cfg.server_name)?);
    // status_request — OCSP stapling, empty responder/extensions lists.
    tls13::push_ext(&mut exts, 5, &[1, 0, 0, 0, 0]);
    tls13::push_ext(
        &mut exts,
        10,
        &tls13::u16_list_ext(&[GROUP_X25519, GROUP_P256, GROUP_P384]),
    );
    tls13::push_ext(&mut exts, 11, &[1, 0]); // uncompressed points
    tls13::push_ext(&mut exts, 13, &tls13::u16_list_ext(&tls13::SIG_ALGS));
    tls13::push_ext(&mut exts, 0xff01, &[0]); // secure renegotiation (empty)
    tls13::push_ext(&mut exts, 16, &tls13::alpn_ext(&["h2", "http/1.1"])?);
    tls13::push_ext(&mut exts, 18, &[]); // SCT
    tls13::push_ext(&mut exts, 35, &[]); // session_ticket capability
    tls13::push_ext(&mut exts, 23, &[]); // extended_master_secret
    put_u16(exts.len() as u16, &mut body);
    body.extend_from_slice(&exts);

    let mut hello = Vec::with_capacity(4 + body.len());
    hello.push(1);
    put_u24(body.len(), &mut hello);
    hello.extend_from_slice(&body);
    Ok(hello)
}

struct ParsedServerHello12 {
    random: [u8; 32],
    cipher: u16,
    extended_master_secret: bool,
}

fn parse_server_hello(raw: &[u8]) -> Result<ParsedServerHello12> {
    if raw.len() < 42 || raw[0] != HS_SERVER_HELLO {
        return Err(TransportError::Tls("restls12: invalid ServerHello".into()));
    }
    let body_len = read_u24(&raw[1..4]);
    if raw.len() != 4 + body_len {
        return Err(TransportError::Tls(
            "restls12: truncated ServerHello".into(),
        ));
    }
    let body = &raw[4..];
    if body[0] != 0x03 || body[1] != 0x03 {
        return Err(TransportError::Tls("restls12: not a TLS 1.2 server".into()));
    }
    let mut random = [0u8; 32];
    random.copy_from_slice(&body[2..34]);
    let mut pos = 34;
    let sid_len = take_u8(body, &mut pos)? as usize;
    take(body, &mut pos, sid_len)?; // session_id echo is not checked upstream
    let cipher = take_u16(body, &mut pos)?;
    let compression = take_u8(body, &mut pos)?;
    if compression != 0 {
        return Err(TransportError::Tls(
            "restls12: compression negotiated".into(),
        ));
    }
    let mut ems = false;
    if pos < body.len() {
        let ext_len = take_u16(body, &mut pos)? as usize;
        let exts = take(body, &mut pos, ext_len)?;
        let mut ep = 0;
        while ep < exts.len() {
            let typ = take_u16(exts, &mut ep)?;
            let len = take_u16(exts, &mut ep)? as usize;
            take(exts, &mut ep, len)?;
            if typ == 23 {
                ems = true;
            }
        }
    }
    Ok(ParsedServerHello12 {
        random,
        cipher,
        extended_master_secret: ems,
    })
}

/// ServerKeyExchange for ECDHE: `curve_type(1) group(2) pubkey(1+len)
/// sig_scheme(2) sig_len(2) sig`.
struct ParsedSke {
    group: u16,
    public: Vec<u8>,
    /// `client_random || server_random || params` — the signed content.
    signed_params: Vec<u8>,
    scheme: u16,
    signature: Vec<u8>,
}

fn parse_server_key_exchange(
    body: &[u8],
    client_random: &[u8],
    server_random: &[u8],
) -> Result<ParsedSke> {
    let mut pos = 0;
    let curve_type = take_u8(body, &mut pos)?;
    if curve_type != 3 {
        return Err(TransportError::Tls(
            "restls12: non-ECDHE key exchange".into(),
        ));
    }
    let group = take_u16(body, &mut pos)?;
    let pk_len = take_u8(body, &mut pos)? as usize;
    let public = take(body, &mut pos, pk_len)?.to_vec();
    let signed_end = pos;
    let scheme = take_u16(body, &mut pos)?;
    let sig_len = take_u16(body, &mut pos)? as usize;
    let signature = take(body, &mut pos, sig_len)?.to_vec();
    let mut signed_params = Vec::with_capacity(64 + signed_end);
    signed_params.extend_from_slice(client_random);
    signed_params.extend_from_slice(server_random);
    signed_params.extend_from_slice(&body[..signed_end]);
    Ok(ParsedSke {
        group,
        public,
        signed_params,
        scheme,
        signature,
    })
}

/// TLS 1.2 `Certificate` body → DER list (`u24` list length, `u24` per
/// cert, no per-cert extensions — unlike TLS 1.3).
fn parse_certificate_list12(body: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut pos = 0;
    let list_len = take_u24(body, &mut pos)?;
    let list = take(body, &mut pos, list_len)?;
    let mut certs = Vec::new();
    let mut lp = 0;
    while lp < list.len() {
        let clen = take_u24(list, &mut lp)?;
        certs.push(take(list, &mut lp, clen)?.to_vec());
    }
    if certs.is_empty() {
        return Err(TransportError::Tls(
            "restls12: empty certificate list".into(),
        ));
    }
    Ok(certs)
}

fn take_u24(input: &[u8], pos: &mut usize) -> Result<usize> {
    Ok(read_u24(take(input, pos, 3)?))
}

/// Run the restls TLS 1.2 handshake. Returns the post-handshake state.
pub(crate) async fn dial<S>(
    mut inner: S,
    cfg: &Tls12Config,
    secret: &[u8; 32],
) -> Result<RestlsUpgraded<S>>
where
    S: Stream,
{
    let keys = generate_eager_keys()?;
    let client_random: [u8; 32] = rand::random();
    let hello = build_client_hello(cfg, &client_random, &keys, secret)?;
    inner
        .write_all(&wrap_record(wire::TLS_RECORD_HANDSHAKE, &hello)?)
        .await?;
    inner.flush().await?;

    // Handshake transcript (plain handshake messages, headers included).
    let mut transcript = Vec::with_capacity(4096);
    transcript.extend_from_slice(&hello);

    // ── server flight (plaintext records) ────────────────────────────
    let mut server_hello: Option<ParsedServerHello12> = None;
    let mut certs: Vec<Vec<u8>> = Vec::new();
    let mut ske: Option<ParsedSke> = None;
    let mut cert_request = false;
    let mut cert_status = false;
    let mut hs_buf: VecDeque<u8> = VecDeque::new();
    let mut got_done = false;
    while !got_done {
        let record = tls13::read_record(&mut inner)
            .await?
            .ok_or_else(|| TransportError::Tls("restls12: EOF in server flight".into()))?;
        if record.typ == wire::TLS_RECORD_CHANGE_CIPHER_SPEC {
            return Err(TransportError::Tls("restls12: early CCS".into()));
        }
        if record.typ != wire::TLS_RECORD_HANDSHAKE {
            return Err(TransportError::Tls(format!(
                "restls12: unexpected record type {}",
                record.typ
            )));
        }
        hs_buf.extend(record.payload.iter().copied());
        if hs_buf.len() > MAX_SERVER_FLIGHT {
            return Err(TransportError::Tls(
                "restls12: oversized server flight".into(),
            ));
        }
        while let Some(msg) = tls13::pop_handshake_message(&mut hs_buf) {
            match msg.typ {
                HS_SERVER_HELLO => {
                    if server_hello.is_some() {
                        return Err(TransportError::Tls(
                            "restls12: duplicate ServerHello".into(),
                        ));
                    }
                    server_hello = Some(parse_server_hello(&msg.raw)?);
                }
                HS_CERTIFICATE => {
                    if server_hello.is_none()
                        || !certs.is_empty()
                        || ske.is_some()
                        || cert_request
                        || got_done
                    {
                        return Err(TransportError::Tls(
                            "restls12: unexpected Certificate".into(),
                        ));
                    }
                    certs = parse_certificate_list12(&msg.body)?;
                }
                HS_CERTIFICATE_STATUS => {
                    // RFC 6066: the staple rides between Certificate and
                    // SKE. We offered `status_request` (mirroring the
                    // upstream parrot), so a stapling cover legitimately
                    // sends this — the staple itself carries nothing we
                    // need, but it counts toward the transcript.
                    if certs.is_empty() || ske.is_some() || cert_status || got_done {
                        return Err(TransportError::Tls(
                            "restls12: unexpected CertificateStatus".into(),
                        ));
                    }
                    cert_status = true;
                }
                HS_SERVER_KEY_EXCHANGE => {
                    if server_hello.is_none()
                        || certs.is_empty()
                        || ske.is_some()
                        || cert_request
                        || got_done
                    {
                        return Err(TransportError::Tls(
                            "restls12: unexpected ServerKeyExchange".into(),
                        ));
                    }
                    let sh = server_hello.as_ref().expect("checked");
                    ske = Some(parse_server_key_exchange(
                        &msg.body,
                        &client_random,
                        &sh.random,
                    )?);
                }
                HS_CERTIFICATE_REQUEST => {
                    // TLS 1.2 flight order is SH → Cert → SKE → [CR] →
                    // SHD — a CR before SKE or after SHD is malformed.
                    if cert_request || ske.is_none() || got_done {
                        return Err(TransportError::Tls("restls12: unexpected CR".into()));
                    }
                    cert_request = true;
                }
                HS_SERVER_HELLO_DONE => {
                    if server_hello.is_none() || ske.is_none() || certs.is_empty() {
                        return Err(TransportError::Tls(
                            "restls12: early ServerHelloDone".into(),
                        ));
                    }
                    got_done = true;
                }
                // A ticket-issuing cover may coalesce NST into the same
                // record as ServerHelloDone (RFC 5077 order: NST follows
                // SHD and precedes CCS). It counts toward the
                // server-Finished transcript — but only after SHD.
                HS_NEW_SESSION_TICKET => {
                    if !got_done {
                        return Err(TransportError::Tls(
                            "restls12: NST before ServerHelloDone".into(),
                        ));
                    }
                }
                _ => {
                    return Err(TransportError::Tls(format!(
                        "restls12: unexpected message {}",
                        msg.typ
                    )))
                }
            }
            // NST spam keeps `hs_buf` small (each message pops complete)
            // while the transcript would grow without bound — cap it at
            // the same bar tls13 applies (`MAX_PRE_AUTH_TRANSCRIPT_LEN`).
            if transcript.len() + msg.raw.len() > MAX_SERVER_FLIGHT {
                return Err(TransportError::Tls(
                    "restls12: oversized server transcript".into(),
                ));
            }
            transcript.extend_from_slice(&msg.raw);
        }
    }

    let sh = server_hello.expect("flight checked");
    let ske = ske.expect("flight checked");
    let cipher = Cipher12::try_from(sh.cipher)?;
    let hash = cipher.hash();

    // Certificate verification — same policy as TLS 1.3.
    let name = cfg.cert.verify_name.as_deref().unwrap_or(&cfg.server_name);
    tls13::verify_certificate_chain(&cfg.cert, name, &certs)?;

    // ServerKeyExchange signature over client_random || server_random || params.
    // Only schemes we offered in `signature_algorithms` are acceptable —
    // upstream's `isSupportedSignatureAlgorithm` gate (v1.5 stays legal
    // here: TLS 1.2 RSA SKE needs it, unlike TLS 1.3 CV).
    if !tls13::SIG_ALGS.contains(&ske.scheme) {
        return Err(TransportError::Tls(format!(
            "restls12: unoffered SKE scheme 0x{:04x}",
            ske.scheme
        )));
    }
    tls13::verify_signature(ske.scheme, &ske.signature, &ske.signed_params, &certs[0])?;

    // ── client flight ────────────────────────────────────────────────
    // The eager key matching the server's chosen curve becomes the
    // ClientKeyExchange public key — this is what the session_id tag
    // committed to.
    let eager = keys
        .iter()
        .find(|k| k.group == ske.group)
        .ok_or_else(|| TransportError::Tls("restls12: server chose unoffered curve".into()))?;
    if cert_request {
        // No client cert — TLS 1.2 sends an empty Certificate message.
        let empty = [HS_CERTIFICATE, 0, 0, 3, 0, 0, 0];
        inner
            .write_all(&wrap_record_with_version(
                wire::TLS_RECORD_HANDSHAKE,
                &empty,
                [0x03, 0x03],
            )?)
            .await?;
        transcript.extend_from_slice(&empty);
    }
    let mut cke = Vec::with_capacity(5 + eager.public.len());
    cke.push(HS_CLIENT_KEY_EXCHANGE);
    put_u24(1 + eager.public.len(), &mut cke);
    cke.push(eager.public.len() as u8);
    cke.extend_from_slice(&eager.public);
    inner
        .write_all(&wrap_record_with_version(
            wire::TLS_RECORD_HANDSHAKE,
            &cke,
            [0x03, 0x03],
        )?)
        .await?;
    transcript.extend_from_slice(&cke);

    // Key schedule.
    let pre_master = eager.agree(&ske.public)?;
    let master_secret = if sh.extended_master_secret {
        let session_hash = hash.digest(&transcript);
        prf(
            hash,
            &pre_master,
            b"extended master secret",
            &session_hash,
            48,
        )
    } else {
        let mut seed = Vec::with_capacity(64);
        seed.extend_from_slice(&client_random);
        seed.extend_from_slice(&sh.random);
        prf(hash, &pre_master, b"master secret", &seed, 48)
    };
    let mut seed = Vec::with_capacity(64);
    seed.extend_from_slice(&sh.random);
    seed.extend_from_slice(&client_random);
    let key_block_len = cipher.key_len() * 2 + 8;
    let key_block = prf(hash, &master_secret, b"key expansion", &seed, key_block_len);
    let kl = cipher.key_len();
    let client_key = &key_block[..kl];
    let server_key = &key_block[kl..2 * kl];
    let client_salt = &key_block[2 * kl..2 * kl + 4];
    let server_salt = &key_block[2 * kl + 4..2 * kl + 8];
    let mut client_write = GcmAead::new(cipher, client_key, client_salt);
    let mut server_read = GcmAead::new(cipher, server_key, server_salt);

    // CCS + encrypted client Finished.
    inner
        .write_all(&[
            wire::TLS_RECORD_CHANGE_CIPHER_SPEC,
            0x03,
            0x03,
            0x00,
            0x01,
            0x01,
        ])
        .await?;
    let verify = prf(
        hash,
        &master_secret,
        b"client finished",
        &hash.digest(&transcript),
        12,
    );
    let mut fin_msg = Vec::with_capacity(4 + verify.len());
    fin_msg.push(HS_FINISHED);
    put_u24(verify.len(), &mut fin_msg);
    fin_msg.extend_from_slice(&verify);
    transcript.extend_from_slice(&fin_msg);
    let client_fin_record = client_write.seal(wire::TLS_RECORD_HANDSHAKE, &fin_msg)?;
    inner.write_all(&client_fin_record).await?;
    inner.flush().await?;

    // ── server [NST] + CCS + Finished ────────────────────────────────
    // The server-auth mask applies to the first *encrypted* record — the
    // Finished. A ticket-issuing cover's NewSessionTicket is a plaintext
    // handshake record preceding the CCS (RFC 5077 order).
    let mut saw_ccs = false;
    let mut authed: Option<bool> = None;
    let mut gcm_ctr_disabled = false;
    // Persistent across records — a handshake message may straddle a
    // record boundary (RFC 5246 §6.2.1).
    let mut msgs: VecDeque<u8> = VecDeque::new();
    loop {
        let record = tls13::read_record(&mut inner)
            .await?
            .ok_or_else(|| TransportError::Tls("restls12: EOF in server finish".into()))?;
        if record.typ == wire::TLS_RECORD_CHANGE_CIPHER_SPEC {
            if saw_ccs {
                return Err(TransportError::Tls("restls12: duplicate CCS".into()));
            }
            saw_ccs = true;
            continue;
        }
        if !saw_ccs {
            // RFC 5077 order: a ticket-issuing cover sends NewSessionTicket
            // as a *plaintext* handshake record before its CCS.
            if record.typ != wire::TLS_RECORD_HANDSHAKE {
                return Err(TransportError::Tls("restls12: record before CCS".into()));
            }
            msgs.extend(record.payload.iter().copied());
            if msgs.len() > MAX_SERVER_FLIGHT {
                return Err(TransportError::Tls(
                    "restls12: oversized pre-CCS message".into(),
                ));
            }
            while let Some(msg) = tls13::pop_handshake_message(&mut msgs) {
                if msg.typ != HS_NEW_SESSION_TICKET {
                    return Err(TransportError::Tls(format!(
                        "restls12: unexpected pre-CCS message {}",
                        msg.typ
                    )));
                }
                // NST precedes the server Finished in the transcript.
                if transcript.len() + msg.raw.len() > MAX_SERVER_FLIGHT {
                    return Err(TransportError::Tls(
                        "restls12: oversized server transcript".into(),
                    ));
                }
                transcript.extend_from_slice(&msg.raw);
            }
            continue;
        }
        if record.typ == wire::TLS_RECORD_ALERT {
            return Err(TransportError::Tls(
                "restls12: alert in server finish".into(),
            ));
        }
        // TLS 1.2 keeps real content types under encryption — Finished/NST
        // arrive as `handshake` (22) records, not opaque `application_data`.
        let mut full = record.header.to_vec();
        full.extend_from_slice(&record.payload);
        let post_buf;
        // First encrypted record: try the server-auth mask, then raw.
        // `open` consumes the sequence only on success, so a failed masked
        // probe leaves the raw retry at the correct nonce — and a masked
        // success (e.g. an NST preceding Finished) counts the record.
        if authed.is_none() {
            let (unmasked, disable) = wire::unmask_server_auth(&full, secret, &sh.random, true);
            gcm_ctr_disabled = disable;
            match server_read.open(&unmasked) {
                Ok(body) => {
                    authed = Some(true);
                    post_buf = body;
                }
                Err(_) => {
                    authed = Some(false);
                    post_buf = server_read.open(&full)?;
                }
            }
        } else {
            post_buf = server_read.open(&full)?;
        }
        // The decrypted record may carry NewSessionTicket(4) and/or Finished.
        msgs.extend(post_buf.iter().copied());
        if msgs.len() > MAX_SERVER_FLIGHT {
            return Err(TransportError::Tls(
                "restls12: oversized post-CCS message".into(),
            ));
        }
        while let Some(msg) = tls13::pop_handshake_message(&mut msgs) {
            match msg.typ {
                HS_NEW_SESSION_TICKET => {
                    // Transcript: NST counts toward the *server* Finished
                    // hash — and it precedes it.
                    if transcript.len() + msg.raw.len() > MAX_SERVER_FLIGHT {
                        return Err(TransportError::Tls(
                            "restls12: oversized server transcript".into(),
                        ));
                    }
                    transcript.extend_from_slice(&msg.raw);
                }
                HS_FINISHED => {
                    let expect = prf(
                        hash,
                        &master_secret,
                        b"server finished",
                        &hash.digest(&transcript),
                        12,
                    );
                    use subtle::ConstantTimeEq;
                    if !bool::from(msg.body.ct_eq(&expect)) {
                        return Err(TransportError::Tls(
                            "restls12: server Finished mismatch".into(),
                        ));
                    }
                    // Done — hand off to the tagged-record layer.
                    let gcm_next_seq = server_read.seq();
                    return Ok(RestlsUpgraded {
                        inner,
                        server_random: sh.random,
                        client_finished: None, // upstream: full tls12 has no fin binding
                        authed: authed.unwrap_or(false),
                        cover_read: Some(Box::new(server_read)),
                        cover_write: Some(Box::new(client_write)),
                        tls12_gcm: true,
                        gcm_ctr_disabled,
                        gcm_next_seq,
                        cover_hs_pending: msgs.iter().copied().collect(),
                    });
                }
                _ => {
                    return Err(TransportError::Tls(format!(
                        "restls12: unexpected post-CCS message {}",
                        msg.typ
                    )))
                }
            }
        }
    }
}

fn wrap_record(typ: u8, payload: &[u8]) -> Result<Vec<u8>> {
    wrap_record_with_version(typ, payload, [0x03, 0x01])
}

/// Upstream writes the CH at `0x0301` (compat) but post-ServerHello
/// records at `c.vers = 0x0303`.
fn wrap_record_with_version(typ: u8, payload: &[u8], version: [u8; 2]) -> Result<Vec<u8>> {
    if payload.len() > u16::MAX as usize {
        return Err(TransportError::Tls("restls12: record too large".into()));
    }
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(typ);
    out.extend_from_slice(&version);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn take<'a>(input: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| TransportError::Tls("restls12: parser overflow".into()))?;
    let out = input
        .get(*pos..end)
        .ok_or_else(|| TransportError::Tls("restls12: truncated".into()))?;
    *pos = end;
    Ok(out)
}

fn take_u8(input: &[u8], pos: &mut usize) -> Result<u8> {
    Ok(take(input, pos, 1)?[0])
}

fn take_u16(input: &[u8], pos: &mut usize) -> Result<u16> {
    let b = take(input, pos, 2)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u24(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
}

fn put_u16(value: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u24(value: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&[(value >> 16) as u8, (value >> 8) as u8, value as u8]);
}
