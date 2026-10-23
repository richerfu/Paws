#[path = "../src/subscription_scan.rs"]
mod subscription_scan;

use subscription_scan::{parse_scanned_subscription, ScannedSubscriptionError};

#[test]
fn accepts_plain_http_subscription_urls() {
    let parsed =
        parse_scanned_subscription(" \nhttps://example.com/sub.yaml?token=abc \n").unwrap();

    assert_eq!(parsed.url, "https://example.com/sub.yaml?token=abc");
    assert_eq!(parsed.name, None);
}

#[test]
fn unwraps_clash_and_mihomo_install_links() {
    for scheme in ["clash", "mihomo"] {
        let parsed = parse_scanned_subscription(&format!(
            "{scheme}://install-config?url=https%3A%2F%2Fexample.com%2Fsub.yaml%3Ftoken%3Dabc&name=Paws"
        ))
        .unwrap();

        assert_eq!(parsed.url, "https://example.com/sub.yaml?token=abc");
        assert_eq!(parsed.name.as_deref(), Some("Paws"));
    }
}

#[test]
fn accepts_common_json_subscription_payloads() {
    let parsed = parse_scanned_subscription(
        r#"{"subscriptionUrl":"https://example.com/sub.yaml","title":"Office"}"#,
    )
    .unwrap();

    assert_eq!(parsed.url, "https://example.com/sub.yaml");
    assert_eq!(parsed.name.as_deref(), Some("Office"));
}

#[test]
fn rejects_empty_non_http_and_unrelated_qr_payloads() {
    assert_eq!(
        parse_scanned_subscription("  "),
        Err(ScannedSubscriptionError::Empty)
    );
    for payload in [
        "hello world",
        "file:///data/sub.yaml",
        "ss://example",
        "clash://install-config?url=file%3A%2F%2F%2Fdata%2Fsub.yaml",
    ] {
        assert_eq!(
            parse_scanned_subscription(payload),
            Err(ScannedSubscriptionError::Unsupported),
            "{payload}"
        );
    }
}
