//! TLS layer tests — cases A1..A13 from `docs/specs/transport-layer-test-plan.md`.

mod support;

use meow_transport::{
    tls::{ClientCert, EchOpts, TlsConfig, TlsLayer},
    Transport,
};
use support::{
    log_capture::capture_logs,
    loopback::{gen_cert, install_crypto_provider, spawn_tls_server, ServerOptions},
};
use tokio::net::TcpStream;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Dial the loopback server and return the upgraded stream ready for I/O.
async fn tls_connect(
    addr: std::net::SocketAddr,
    config: &TlsConfig,
) -> meow_transport::Result<Box<dyn meow_transport::Stream>> {
    let tcp = TcpStream::connect(addr).await.expect("TCP connect");
    TlsLayer::new(config)?.connect(Box::new(tcp)).await
}

// ─── A1: tls_connect_cert_ok ─────────────────────────────────────────────────

#[tokio::test]
async fn tls_connect_cert_ok() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("localhost".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("localhost")
    };

    let result = tls_connect(addr, &config).await;
    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
}

// ─── A2: tls_connect_bad_cert_errs ───────────────────────────────────────────

#[tokio::test]
async fn tls_connect_bad_cert_errs() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der,
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    // Do NOT add the server cert to additional_roots → cert verification fails.
    let config = TlsConfig::new("localhost");

    let result = tls_connect(addr, &config).await;
    assert!(result.is_err(), "expected Err for untrusted cert");
    let err_str = result.err().unwrap().to_string();
    // The error message must contain a TLS-related marker so log greps stay useful.
    assert!(
        err_str.contains("tls") || err_str.contains("handshake") || err_str.contains("certificate"),
        "error message missing expected marker: {err_str}"
    );
}

// ─── A3: tls_skip_verify_connects ────────────────────────────────────────────

#[tokio::test]
async fn tls_skip_verify_connects() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der,
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        skip_cert_verify: true,
        ..TlsConfig::new("localhost")
    };

    // Log capture for warn assertion (sync part: TlsLayer::new emits the warn).
    let logs = capture_logs(|| {
        TlsLayer::new(&config).expect("TlsLayer::new with skip_cert_verify");
    });

    // Assert the warn was emitted.
    assert!(
        logs.contains_all(&["skip-cert-verify"]),
        "expected skip-cert-verify warn, got: {:?}",
        logs.lines()
    );

    // Connection itself must succeed.
    let result = tls_connect(addr, &config).await;
    assert!(
        result.is_ok(),
        "expected Ok with skip_cert_verify, got: {:?}",
        result.err()
    );
}

// ─── A4: tls_alpn_negotiated_h2 ──────────────────────────────────────────────

#[tokio::test]
async fn tls_alpn_negotiated_h2() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        alpn: vec!["h2".into(), "http/1.1".into()],
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("localhost")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    assert_eq!(
        info.alpn.as_deref(),
        Some(b"h2" as &[u8]),
        "expected negotiated ALPN=h2"
    );
}

// ─── A5: tls_alpn_fallback_http11 ────────────────────────────────────────────

#[tokio::test]
async fn tls_alpn_fallback_http11() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    // Server only offers http/1.1
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![b"http/1.1".to_vec()],
        require_client_cert_ca: None,
    })
    .await;

    // Client prefers h2 first, but server only offers http/1.1
    let config = TlsConfig {
        alpn: vec!["h2".into(), "http/1.1".into()],
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("localhost")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    assert_eq!(
        info.alpn.as_deref(),
        Some(b"http/1.1" as &[u8]),
        "expected fallback ALPN=http/1.1"
    );
}

// ─── A6: tls_alpn_empty_config ───────────────────────────────────────────────

#[tokio::test]
async fn tls_alpn_empty_config() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        require_client_cert_ca: None,
    })
    .await;

    // Client sends no ALPN (alpn = []).
    let config = TlsConfig {
        alpn: vec![],
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("localhost")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    // When client sends no ALPN extension, the server observes no negotiated ALPN.
    assert!(
        info.alpn.is_none(),
        "expected no negotiated ALPN when client sends none, got: {:?}",
        info.alpn
    );
}

// ─── A7: tls_sni_override ────────────────────────────────────────────────────

#[tokio::test]
async fn tls_sni_override() {
    install_crypto_provider();
    // Server cert for "cdn.example.com" to match the SNI override.
    let (cert_der, key_der, _, _) = gen_cert(&["cdn.example.com"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    // Dial to 127.0.0.1 but override SNI to "cdn.example.com".
    let config = TlsConfig {
        sni: Some("cdn.example.com".into()),
        skip_cert_verify: true, // cert CN doesn't match the dial IP
        ..TlsConfig::new("cdn.example.com")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    assert_eq!(
        info.server_name.as_deref(),
        Some("cdn.example.com"),
        "server should have received SNI=cdn.example.com"
    );
}

// ─── A8: tls_sni_fallback_to_host ────────────────────────────────────────────

#[tokio::test]
async fn tls_sni_fallback_to_host() {
    install_crypto_provider();
    // Config-layer simulation: sni=Some("localhost") because server is "localhost" hostname.
    // The "fallback" happened in config, not in TlsLayer.
    let (cert_der, key_der, _, _) = gen_cert(&["localhost"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("localhost")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    assert_eq!(
        info.server_name.as_deref(),
        Some("localhost"),
        "server should have received SNI=localhost"
    );
}

// ─── A9: tls_sni_is_ip_omitted ───────────────────────────────────────────────

#[tokio::test]
async fn tls_sni_is_ip_omitted() {
    install_crypto_provider();
    // When server is an IP (127.0.0.1), config resolves sni=Some("127.0.0.1").
    // rustls parses "127.0.0.1" as ServerName::IpAddress, which does NOT include
    // the SNI extension in the ClientHello (RFC 6066 §3 prohibits IP literals).
    let (cert_der, key_der, _, _) = gen_cert(&["127.0.0.1"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        // IP literal: rustls will use IpAddress ServerName → no SNI extension.
        sni: Some("127.0.0.1".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("127.0.0.1")
    };

    tls_connect(addr, &config).await.expect("connect");

    let info = conn_rx.await.expect("ConnInfo");
    // RFC 6066: IP literals MUST NOT appear in the SNI extension.
    assert!(
        info.server_name.is_none(),
        "IP-based connection must not include SNI extension, got: {:?}",
        info.server_name
    );
}

// ─── A10: tls_client_cert_accepted ───────────────────────────────────────────

#[tokio::test]
async fn tls_client_cert_accepted() {
    install_crypto_provider();

    // Server cert.
    let (server_cert_der, server_key_der, _, _) = gen_cert(&["localhost"]);

    // Client cert (self-signed, the CA is the cert itself for this simple test).
    let (client_cert_der, _client_key_der, client_cert_pem, client_key_pem) =
        gen_cert(&["client.example.com"]);

    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: server_cert_der.clone(),
        key_der: server_key_der,
        server_alpn: vec![],
        require_client_cert_ca: Some(client_cert_der.clone()),
    })
    .await;

    let config = TlsConfig {
        sni: Some("localhost".into()),
        skip_cert_verify: true, // server cert is self-signed, not in additional_roots
        client_cert: Some(ClientCert {
            cert_pem: client_cert_pem.into_bytes(),
            key_pem: client_key_pem.into_bytes(),
        }),
        ..TlsConfig::new("localhost")
    };

    tls_connect(addr, &config)
        .await
        .expect("connect with client cert");

    let info = conn_rx.await.expect("ConnInfo");
    assert!(
        !info.peer_certs.is_empty(),
        "server should have observed client cert"
    );
    // Verify the client cert DER matches what we sent.
    assert_eq!(
        info.peer_certs[0],
        client_cert_der.as_ref(),
        "client cert DER mismatch"
    );
}

// ─── A13: tls_fingerprint_none_no_warn ───────────────────────────────────────

#[test]
fn tls_fingerprint_none_no_warn() {
    install_crypto_provider();

    let config = TlsConfig::new("localhost");
    assert!(config.fingerprint.is_none());

    let logs = capture_logs(|| {
        TlsLayer::new(&config).expect("TlsLayer::new without fingerprint");
    });

    // No fingerprint-related warn must appear.
    let fp_warn_count = logs.count_containing(&["uTLS fingerprint"]);
    assert_eq!(
        fp_warn_count, 0,
        "expected 0 fingerprint warns with fingerprint=None, got {fp_warn_count}"
    );
}

// ─── verify_name (gost-plugin name-cert-verify) ──────────────────────────────

/// `verify_name` verifies the peer cert against a name different from the
/// connection SNI: SNI `sni.example.com`, cert for `real.example.com`,
/// `verify_name = "real.example.com"` → handshake succeeds and the server
/// sees the *unchanged* SNI.
#[tokio::test]
async fn tls_verify_name_overrides_cert_check_not_sni() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["real.example.com"]);
    let (addr, conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("sni.example.com".into()),
        verify_name: Some("real.example.com".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("sni.example.com")
    };

    tls_connect(addr, &config)
        .await
        .expect("connect with verify_name override");

    let info = conn_rx.await.expect("ConnInfo");
    assert_eq!(
        info.server_name.as_deref(),
        Some("sni.example.com"),
        "verify_name must not change the wire SNI"
    );
}

/// IP `server_name` + DNS `verify_name`: `into_ssl` would seed
/// `param->ip` from the literal and `check_id` enforces ip and hosts
/// independently — the override must replace that seed, not stack on it.
#[tokio::test]
async fn tls_verify_name_dns_overrides_ip_sni() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["real.example.com"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("127.0.0.1".into()),
        verify_name: Some("real.example.com".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("127.0.0.1")
    };

    tls_connect(addr, &config)
        .await
        .expect("IP host + DNS verify_name must not stack both checks");
}

/// DNS `server_name` + IP `verify_name`: the symmetric case — the seeded
/// `param->hosts` must not remain when the override installs an IP check.
#[tokio::test]
async fn tls_verify_name_ip_overrides_dns_sni() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["127.0.0.1"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("sni.example.com".into()),
        verify_name: Some("127.0.0.1".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("sni.example.com")
    };

    tls_connect(addr, &config)
        .await
        .expect("DNS SNI + IP verify_name must not stack both checks");
}

/// Without the override the same cert is verified against the SNI and the
/// handshake must fail.
#[tokio::test]
async fn tls_verify_name_absent_fails_on_sni_mismatch() {
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["real.example.com"]);
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der: cert_der.clone(),
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("sni.example.com".into()),
        additional_roots: vec![cert_der.as_ref().to_vec()],
        ..TlsConfig::new("sni.example.com")
    };

    let result = tls_connect(addr, &config).await;
    assert!(
        result.is_err(),
        "cert for real.example.com must fail against SNI sni.example.com"
    );
}

// ─── ECH config bound ─────────────────────────────────────────────────────────

/// `ECHConfigList` is u16-length-prefixed on the wire; a larger blob is
/// malformed and must be rejected at `TlsLayer::new`, not re-parsed on
/// every handshake.
#[test]
fn tls_ech_config_over_u16_bound_rejected() {
    let config = TlsConfig {
        ech: Some(EchOpts::Config(vec![0u8; u16::MAX as usize + 1])),
        ..TlsConfig::new("example.com")
    };
    let err = TlsLayer::new(&config)
        .err()
        .expect("oversized ECHConfigList");
    assert!(
        err.to_string().contains("ECHConfigList"),
        "unexpected error: {err}"
    );
}

// ─── cert_pin (gost-plugin fingerprint / SSL pinning) ─────────────────────────

/// A leaf SHA-256 pin accepts a cert even when it is *not* CA-trusted:
/// upstream `FingerprintVerifier` replaces CA verification entirely.
#[tokio::test]
async fn tls_cert_pin_accepts_untrusted_leaf() {
    use sha2::Digest;
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["pin.example.com"]);
    let pin: [u8; 32] = sha2::Sha256::digest(cert_der.as_ref()).into();
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der,
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    // Deliberately no `additional_roots`: the self-signed leaf is
    // untrusted and only the pin can accept it.
    let config = TlsConfig {
        sni: Some("pin.example.com".into()),
        cert_pin: Some(pin),
        ..TlsConfig::new("pin.example.com")
    };

    tls_connect(addr, &config)
        .await
        .expect("cert_pin must accept the pinned leaf without CA trust");
}

/// A pin that matches nothing in the presented chain must reject.
#[tokio::test]
async fn tls_cert_pin_mismatch_rejected() {
    use sha2::Digest;
    install_crypto_provider();
    let (cert_der, key_der, _, _) = gen_cert(&["pin.example.com"]);
    let (other_der, _, _, _) = gen_cert(&["other.example.com"]);
    let wrong_pin: [u8; 32] = sha2::Sha256::digest(other_der.as_ref()).into();
    let (addr, _conn_rx) = spawn_tls_server(ServerOptions {
        cert_der,
        key_der,
        server_alpn: vec![],
        require_client_cert_ca: None,
    })
    .await;

    let config = TlsConfig {
        sni: Some("pin.example.com".into()),
        cert_pin: Some(wrong_pin),
        ..TlsConfig::new("pin.example.com")
    };

    assert!(
        tls_connect(addr, &config).await.is_err(),
        "non-matching pin must reject the handshake"
    );
}
