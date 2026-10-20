//! In-process `restls` SIP003 plugin for Shadowsocks (mihomo
//! `transport/restls` parity — the record-level TLS client and tagged-record
//! transport live in [`meow_transport::restls`]).
//!
//! `plugin: restls` with `plugin-opts` keys:
//!
//! | opt | meaning | default |
//! |-----|---------|---------|
//! | `host` | cover server name — TLS SNI the handshake is relayed to | *(required)* |
//! | `password` | restls PSK (BLAKE3-derived traffic secret) | *(required)* |
//! | `version-hint` | `tls13` or `tls12` (case-insensitive) | *(required)* |
//! | `restls-script` | record-shaping script (`250?100<1,…`) | upstream default |
//! | `skip-cert-verify` | disable cover cert verification | `false` |
//! | `name-cert-verify` | cert-verify hostname when it differs from SNI | `host` |
//! | `fingerprint` | SHA-256 **certificate pin** (upstream
//!   `ca.NewFingerprintVerifier`), *not* a uTLS profile | — |
//! | `force-tls12` | upstream test knob — forces the TLS 1.2 code path | `false` |
//!
//! `host`/`password`/`version-hint` are non-`omitempty` upstream
//! (`restlsOption`), so all three are required here.
//!
//! Upstream `force-tls12` disables the TLS 1.3 path entirely
//! (`!supportTLS13`), so the session-id tag uses the TLS 1.2 layout
//! eager-key format — identical to `version-hint: tls12`. The knob maps
//! to that same code path here. It is an upstream debugging aid; normal
//! deployments use `version-hint` alone.
//!
//! The node-level `client-fingerprint` option and upstream's `client-id`
//! opt select a uTLS ClientHello parrot upstream; this client crafts its
//! own ClientHello (a fixed Chrome-like shape), so neither has an effect
//! here — `client-id` falls through to the unknown-opt warn.

use meow_common::error::{MeowError, Result};
use meow_transport::{restls::RestlsConfig, Stream};
use tracing::{debug, warn};

use crate::plugin_util::{parse_bool_strict, parse_cert_pin, sip003_opts};
use crate::transport_to_proxy_err;

const PLUGIN: &str = "restls";

/// Parsed `restls` client options.
#[derive(Debug, Clone)]
pub struct RestlsPluginConfig {
    /// Cover server name — the TLS handshake's SNI; the restls server
    /// relays the handshake to this host.
    pub host: String,
    /// restls PSK (BLAKE3-derive-keyed into the traffic secret).
    pub password: String,
    /// `tls13` or `tls12` (case-insensitive) — selects the record-level
    /// client and the session-id tag layout.
    pub version_hint: String,
    /// Record script (`restls-script`); `None` → upstream default.
    pub restls_script: Option<String>,
    pub skip_cert_verify: bool,
    /// Cert-verify hostname when it differs from `host`/SNI
    /// (`name-cert-verify` — upstream `ca.NewNameCertVerifier`).
    pub verify_name: Option<String>,
    /// SHA-256 certificate pin (`fingerprint` — SSL pinning per upstream
    /// `ca.NewFingerprintVerifier`, *not* a uTLS ClientHello profile).
    pub cert_pin: Option<[u8; 32]>,
}

/// Parse a flattened SIP003 opts string for `restls`
/// (`host=cover.example.com;password=…;version-hint=tls13;restls-script=…`).
///
/// `host`, `password` and `version-hint` are required (upstream marks all
/// three non-`omitempty`).  Unknown keys are logged at `warn` level and
/// ignored.
pub fn parse_opts(s: &str) -> Result<RestlsPluginConfig> {
    let mut cfg = RestlsPluginConfig {
        host: String::new(),
        password: String::new(),
        version_hint: String::new(),
        restls_script: None,
        skip_cert_verify: false,
        verify_name: None,
        cert_pin: None,
    };
    let mut force_tls12 = false;

    for (key, value) in sip003_opts(s) {
        match key.as_str() {
            "host" => cfg.host = value,
            "password" => cfg.password = value,
            "version-hint" => cfg.version_hint = value.to_ascii_lowercase(),
            "restls-script" if !value.is_empty() => {
                // Fail at parse time, not per-dial — `dial` parses the
                // script on every connection otherwise.
                meow_transport::restls::check_record_script(&value)
                    .map_err(transport_to_proxy_err)?;
                cfg.restls_script = Some(value);
            }
            "skip-cert-verify" => {
                cfg.skip_cert_verify = parse_bool_strict(&value, PLUGIN, "skip-cert-verify")?;
            }
            "force-tls12" => force_tls12 = parse_bool_strict(&value, PLUGIN, "force-tls12")?,
            "name-cert-verify" if !value.is_empty() => cfg.verify_name = Some(value),
            "fingerprint" if !value.is_empty() => {
                cfg.cert_pin = Some(parse_cert_pin(&value, PLUGIN)?);
            }
            // Explicit-empty value clears the opt — same as absent.
            "restls-script" | "name-cert-verify" | "fingerprint" => {}
            other => warn!("{PLUGIN}: ignoring unknown opt '{other}'"),
        }
    }

    // Required upstream (`obfs:"…"` non-omitempty fields).
    for (key, val) in [
        ("host", &cfg.host),
        ("password", &cfg.password),
        ("version-hint", &cfg.version_hint),
    ] {
        if val.is_empty() {
            return Err(MeowError::Config(format!(
                "{PLUGIN}: missing required '{key}' opt"
            )));
        }
    }
    match cfg.version_hint.as_str() {
        "tls12" | "tls13" => {}
        other => {
            return Err(MeowError::Config(format!(
                "{PLUGIN}: 'version-hint' must be tls12 or tls13, got '{other}'"
            )));
        }
    }
    if force_tls12 {
        if cfg.version_hint == "tls13" {
            warn!("{PLUGIN}: 'force-tls12' overrides version-hint=tls13");
        }
        cfg.version_hint = "tls12".to_string();
    }
    Ok(cfg)
}

/// Dial a TCP + restls connection to `server_host:server_port` and return
/// the tagged-record stream ready to be wrapped by the SS layer.
pub async fn dial(
    cfg: &RestlsPluginConfig,
    server_host: &str,
    server_port: u16,
    dialer: &dyn crate::dialer::TcpDialer,
    internal: bool,
) -> Result<Box<dyn Stream>> {
    debug!(
        "{PLUGIN}: dialing {}:{} via cover={} version-hint={}",
        server_host, server_port, cfg.host, cfg.version_hint
    );
    let tcp = dialer
        .dial(server_host, server_port, internal)
        .await
        .map_err(MeowError::Io)?;
    let config = RestlsConfig {
        server_name: cfg.host.clone(),
        password: cfg.password.clone(),
        record_script: cfg.restls_script.clone(),
        version_hint: cfg.version_hint.clone(),
        skip_cert_verify: cfg.skip_cert_verify,
        verify_name: cfg.verify_name.clone(),
        cert_pin: cfg.cert_pin,
        additional_roots: Vec::new(),
    };
    meow_transport::restls::dial(Box::new(tcp), &config)
        .await
        .map_err(transport_to_proxy_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_opts_cases() {
        let cases: &[(&str, bool)] = &[
            // minimal — host, password and version-hint all required
            ("host=cover.example.com;password=p;version-hint=tls13", true),
            ("host=cover.example.com;password=p;version-hint=tls12", true),
            ("host=cover.example.com;password=p;version-hint=TLS13", true),
            // every upstream option
            (
                "host=h;password=p;version-hint=tls13;restls-script=200?100<1",
                true,
            ),
            ("host=h;password=p;version-hint=tls13;skip-cert-verify=true", true),
            ("host=h;password=p;version-hint=tls13;name-cert-verify=v.example.com", true),
            (
                "host=h;password=p;version-hint=tls13;fingerprint=aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:\
                 88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99",
                true,
            ),
            ("host=h;password=p;version-hint=tls13;force-tls12=true", true),
            // missing required opts
            ("password=p;version-hint=tls13", false),           // no host
            ("host=h;version-hint=tls13", false),               // no password
            ("host=h;password=p", false),                       // no version-hint
            ("host=h;password=p;version-hint=", false),         // empty hint
            ("host=h;password=p;version-hint=tls11", false),    // bad hint
            ("host=h;password=p;version-hint=1.3", false),      // bad hint
            ("", false),
            ("host=h;password=p;version-hint=tls13;fingerprint=chrome", false),
            ("host=h;password=p;version-hint=tls13;fingerprint=zz", false),
            ("host=h;password=p;version-hint=tls13;restls-script=abc", false),
            ("host=h;password=p;version-hint=tls13;restls-script=20000<300", false),
            // keys are case-insensitive (mapstructure parity)
            ("Host=cover.example.com;Password=p;Version-Hint=tls13", true),
            // security-relevant bools reject junk — a typo must not
            // silently coerce to "verify on" / "tls12 path off"
            ("host=h;password=p;version-hint=tls13;skip-cert-verify=bogus", false),
            ("host=h;password=p;version-hint=tls13;force-tls12=maybe", false),
        ];
        for (opts, ok) in cases {
            assert_eq!(
                parse_opts(opts).is_ok(),
                *ok,
                "opts {opts:?} expected ok={ok}"
            );
        }
    }

    #[test]
    fn parse_opts_full() {
        let cfg = parse_opts(
            "host=cover.example.com;password=psk;version-hint=tls12;\
             restls-script=100?50<1;skip-cert-verify=true;\
             name-cert-verify=real.example.com;\
             fingerprint=AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899",
        )
        .unwrap();
        assert_eq!(cfg.version_hint, "tls12");
        assert_eq!(cfg.restls_script.as_deref(), Some("100?50<1"));
        assert!(cfg.skip_cert_verify);
        assert_eq!(cfg.verify_name.as_deref(), Some("real.example.com"));
        assert!(cfg.cert_pin.is_some());
    }

    #[test]
    fn force_tls12_overrides_hint() {
        let cfg = parse_opts("host=h;password=p;version-hint=tls13;force-tls12=true").unwrap();
        assert_eq!(cfg.version_hint, "tls12");
        // No-op on an already-tls12 hint.
        let cfg = parse_opts("host=h;password=p;version-hint=tls12;force-tls12=true").unwrap();
        assert_eq!(cfg.version_hint, "tls12");
    }
}
