//! jls — a Shadowsocks SIP003 transport authenticating inside a real TLS
//! 1.3 handshake (upstream `metacubex/jls-tls`, used by mihomo's `jls`
//! plugin and ShadowQUIC).
//!
//! The client seals a random seed into `ClientHello.random` under
//! credentials derived from the username/password and the serialized
//! hello; a jls server decrypts it, and answers with its own sealed
//! `ServerHello.random`. The handshake is a genuine TLS 1.3 exchange — the
//! camouflage certificate is only verified when authentication fails (the
//! server then behaves as a relay to a real TLS site). After the
//! handshake the connection is a plain TLS stream — no record tagging.
//!
//! Because the auth blob lives in `random` — part of the TLS transcript —
//! a generic TLS stack cannot produce it; the handshake is driven at the
//! record level via the shared `restls::tls13::drive_tls13` driver.

pub(crate) mod auth;
pub(crate) mod conn;
pub(crate) mod tls13;

use crate::{Result, Stream, TransportError};

/// Resolved jls configuration (SIP003 `plugin-opts` in `meow-proxy`).
#[derive(Debug, Clone)]
pub struct JlsConfig {
    /// Cover SNI — also the certificate DNS name on the unauthenticated
    /// (fallback) path (`host` option; upstream `ServerName`).
    pub server_name: String,
    /// jls username — feeds the auth nonce (`user_iv` in rustls-jls terms).
    pub username: String,
    /// jls password — feeds the auth key (`user_pwd`).
    pub password: String,
    /// ALPN protocols; upstream default `["h2", "http/1.1"]` applies when
    /// empty.
    pub alpn: Vec<String>,
    /// Extra CA roots (DER) for the unauthenticated cert check — not in
    /// upstream's option set; exists so e2e tests can pin a self-signed CA.
    pub additional_roots: Vec<Vec<u8>>,
}

/// Dial through `inner` and return the post-handshake stream.
///
/// `inner` must already be connected to the jls server; the TLS handshake
/// is performed in-process. Returns `Err` after a completed handshake when
/// the peer did not authenticate (upstream `ErrJLSAuthFailed`).
pub async fn dial<S>(inner: S, config: &JlsConfig) -> Result<Box<dyn Stream>>
where
    S: Stream,
{
    if config.server_name.is_empty() {
        return Err(TransportError::Config(
            "jls: server name is required".into(),
        ));
    }
    if config.username.is_empty() {
        return Err(TransportError::Config("jls: username is required".into()));
    }
    if config.password.is_empty() {
        return Err(TransportError::Config("jls: password is required".into()));
    }
    Ok(Box::new(tls13::dial(inner, config).await?))
}
