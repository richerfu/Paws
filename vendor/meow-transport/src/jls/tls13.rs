//! jls TLS 1.3 handshake — ClientHello/ServerHello random authentication
//! over the shared record-level driver (`crate::restls::tls13::drive_tls13`).
//!
//! Upstream `jls-tls` replaces `ClientHello.random` with a sealed blob
//! (`authData` = the serialized CH with `random` zeroed) and answers with
//! the same construction in `ServerHello.random`. We build the CH with a
//! zeroed random, compute `authData` over those exact bytes, then splice
//! the fake random in — the transcript hashes the patched bytes, matching
//! what the server sees.

use super::auth;
use super::conn::JlsStream;
use super::JlsConfig;
use crate::restls::tls13::{
    build_client_hello, drive_tls13, CertPolicy, KeyShare, SentClientHello, Tls13Config,
    GROUP_P256, GROUP_P384, GROUP_X25519, HELLO_RANDOM_LEN, HELLO_RANDOM_OFFSET,
    TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384,
};
use crate::{Result, Stream, TransportError};

/// Cipher suites the jls ClientHello offers — shared with the driver so
/// an unoffered suite in ServerHello is rejected. Upstream (crypto/tls
/// defaults) offers CHACHA20 too; the AES-only list keeps the camouflage
/// hello small.
const JLS_CIPHERS: [u16; 2] = [TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384];

/// Build the jls ClientHello: a standard TLS 1.3 hello whose `random` is
/// the sealed fake-random blob. Resumption is unimplemented (this client
/// can't recompute PSK binders over the patched random), so no
/// `session_ticket`/`pre_shared_key` extensions are offered —
/// `psk_key_exchange_modes` stays in the shared scaffold (wire-legal
/// without a PSK offer; upstream's `SessionTicketsDisabled` omits it, a
/// camouflage-shape divergence only).
fn build_jls_client_hello(cfg: &JlsConfig, shares: &[KeyShare]) -> Result<SentClientHello> {
    let session_id: [u8; 32] = rand::random();
    // authData = the serialized CH with `random` zeroed (and PSK binders
    // zeroed — we send none, so the bytes as built are the authData).
    // `alpn` empty → no ALPN extension (the parser substitutes upstream's
    // h2,http/1.1 default when the option is absent entirely).
    let alpn: Vec<&str> = cfg.alpn.iter().map(String::as_str).collect();
    let mut sent = build_client_hello(
        &cfg.server_name,
        &[0u8; HELLO_RANDOM_LEN],
        &session_id,
        &JLS_CIPHERS,
        &alpn,
        false,
        shares,
    )?;
    let fake = auth::build_fake_random(&cfg.username, &cfg.password, &sent.hello)?;
    sent.hello[HELLO_RANDOM_OFFSET..HELLO_RANDOM_OFFSET + HELLO_RANDOM_LEN].copy_from_slice(&fake);
    Ok(sent)
}

/// Run the jls handshake and return the post-handshake stream.
///
/// A real TLS 1.3 handshake always completes: when the ServerHello random
/// fails authentication the flight still runs (full certificate and
/// CertificateVerify checks — the peer may be a real cover behind the
/// server's fallback relay), then the connection is rejected, matching
/// upstream `ErrJLSAuthFailed`. One divergence: upstream on HRR marks the
/// session unauthenticated and completes the cover handshake; the shared
/// driver cannot process HRR at all and fails mid-flight — the connection
/// is refused either way (JLS v3 forbids HRR for authenticated sessions),
/// only the fallback cover sees an abandoned rather than completed
/// handshake.
pub(crate) async fn dial<S>(inner: S, cfg: &JlsConfig) -> Result<JlsStream<S>>
where
    S: Stream,
{
    let shares = [
        KeyShare::generate(GROUP_X25519)?,
        KeyShare::generate(GROUP_P256)?,
        KeyShare::generate(GROUP_P384)?,
    ];
    let sent = build_jls_client_hello(cfg, &shares)?;

    let tls_cfg = Tls13Config {
        server_name: cfg.server_name.clone(),
        cert: CertPolicy {
            skip_cert_verify: false,
            verify_name: None,
            cert_pin: None,
            additional_roots: cfg.additional_roots.clone(),
        },
    };
    // The ServerHello check is non-fatal upstream — the handshake completes
    // either way, and authentication failure surfaces only after it.
    let out = drive_tls13(inner, &tls_cfg, sent, &shares, None, |parsed, raw| {
        let auth_data = auth::server_hello_auth_data(raw)?;
        Ok(auth::check_fake_random(
            &cfg.username,
            &cfg.password,
            &auth_data,
            &parsed.random,
        ))
    })
    .await?;

    if !out.random_authed {
        return Err(TransportError::Tls("jls: authentication failed".into()));
    }
    JlsStream::new(
        out.inner,
        out.cipher,
        &out.client_ap_secret,
        &out.server_ap_secret,
        &out.leftover_handshake,
    )
}
