//! restls — a Shadowsocks SIP003 transport that hides inside a real TLS
//! handshake with a cover host (upstream `restls-client-go`, used by mihomo's
//! `restls` plugin).
//!
//! The client performs a real TLS handshake with the cover: the session_id
//! carries a keyed BLAKE3 tag over the offered key shares so a restls server
//! can authenticate the client without changing anything else on the wire.
//! After the handshake the cover's first encrypted record is masked with a
//! keyed value (`maskServerAuth`); when the mask uncovers a decryptable
//! record the peer is a restls relay and application data travels in tagged
//! `application_data` records shaped by the record script. When the mask
//! does not uncover (the peer is a plain relay) the stream transparently
//! continues as the negotiated cover TLS connection.
//!
//! Because restls commits the auth tag into `session_id` — part of the TLS
//! transcript — a generic TLS stack cannot implement it faithfully; the
//! handshake is driven at the record level here (see `tls13`), reusing the
//! pattern established by `reality_tls`.
//!
//! Known divergences from the upstream Go client (which runs a full uTLS
//! parrot + TLS stack): HelloRetryRequest is rejected rather than
//! processed — all three supported key shares are offered up front so a
//! group-mismatch HRR cannot occur, but a cover that demands a cookie HRR
//! (anti-DDoS) cannot complete; the ClientHello is a fixed Chrome-like
//! shape rather than a selectable uTLS parrot (`client-id`);
//! `version-hint=tls13` offers only TLS 1.3 in `supported_versions`, so a
//! TLS 1.2-only cover fails where upstream's `[1.3, 1.2]` offer would
//! degrade to transparent cover TLS; and an inbound `Respond(n)` that
//! releases `<`-held writes emits `n−1` fakes — upstream passes its
//! `sent` flag into `handleRestlsCommand` for exactly this, but its
//! `Write([]byte{})` release always reports `sent=false`, making the
//! decrement dead code upstream; we implement the evident intent
//! (observable only as one fewer traffic-shaping record); and the TLS
//! 1.3 CertificateVerify scheme gate is offer-membership — upstream
//! accepts any scheme in its *default* supported list (e.g. an
//! unoffered `0x0603`), we require the scheme to have been offered per
//! RFC 8446 §4.4.3.

pub(crate) mod conn;
pub(crate) mod script;
pub(crate) mod tls12;
pub(crate) mod tls13;
pub(crate) mod wire;

use crate::{Result, Stream};

/// Resolved restls configuration (SIP003 `plugin-opts` in `meow-proxy`).
#[derive(Debug, Clone)]
pub struct RestlsConfig {
    /// Cover SNI and certificate name (`host`).
    pub server_name: String,
    /// restls password — BLAKE3-derive-keyed into the traffic secret.
    pub password: String,
    /// Record script (`restls-script`); defaults to upstream's script.
    pub record_script: Option<String>,
    /// `version-hint` — `"tls13"` or `"tls12"` (required by the plugin
    /// layer; `dial` rejects anything else).
    pub version_hint: String,
    /// Skip cover certificate verification (`skip-cert-verify`).
    pub skip_cert_verify: bool,
    /// Override the DNS name checked in the cover cert (`name-cert-verify`).
    pub verify_name: Option<String>,
    /// SHA-256 fingerprint pin for the cover certificate (`fingerprint`).
    pub cert_pin: Option<[u8; 32]>,
    /// Extra CA roots (DER) — not an upstream opt; a test hook for the
    /// e2e suite's self-signed cover CA.
    pub additional_roots: Vec<Vec<u8>>,
}

/// Validate a `restls-script` string — config-time check so a malformed
/// script fails at startup rather than per-dial.
pub fn check_record_script(script: &str) -> Result<()> {
    script::parse_record_script(script).map(|_| ())
}

/// Dial `server` (`host:port` of the restls server) through `inner` and
/// return the post-handshake stream.
///
/// `inner` must already be connected to the restls server; the cover
/// handshake is performed in-process.
pub async fn dial<S>(inner: S, config: &RestlsConfig) -> Result<Box<dyn Stream>>
where
    S: Stream,
{
    let script = match &config.record_script {
        Some(s) => script::parse_record_script(s)?,
        None => script::parse_record_script(script::DEFAULT_SCRIPT)?,
    };
    let secret = wire::derive_secret(config.password.as_bytes());
    let cert = tls13::CertPolicy {
        skip_cert_verify: config.skip_cert_verify,
        verify_name: config.verify_name.clone(),
        cert_pin: config.cert_pin,
        additional_roots: config.additional_roots.clone(),
    };
    let upgraded = match config.version_hint.to_ascii_lowercase().as_str() {
        "tls12" => {
            let cfg = tls12::Tls12Config {
                server_name: config.server_name.clone(),
                cert,
            };
            tls12::dial(inner, &cfg, &secret).await?
        }
        "tls13" => {
            let cfg = tls13::Tls13Config {
                server_name: config.server_name.clone(),
                cert,
            };
            tls13::dial(inner, &cfg, &secret).await?
        }
        other => {
            return Err(crate::TransportError::Config(format!(
                "restls: invalid version hint '{other}' — expected tls12 or tls13"
            )));
        }
    };
    Ok(Box::new(conn::RestlsStream::new(upgraded, secret, script)?))
}
