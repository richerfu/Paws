#[path = "../src/route_status.rs"]
mod route_status;

use paws_model::ConnectionSummary;
use route_status::latest_active_rule_node;

fn connection(started_at: &str, chains: &[&str]) -> ConnectionSummary {
    ConnectionSummary {
        id: started_at.to_owned(),
        host: "example.com:443".to_owned(),
        domain: "example.com".to_owned(),
        destination_ip: "203.0.113.1".to_owned(),
        destination_port: 443,
        network: "tcp".to_owned(),
        rule: "DOMAIN(example.com)".to_owned(),
        rule_payload: "example.com".to_owned(),
        proxy: chains.join(" > "),
        chains: chains.iter().map(|chain| (*chain).to_owned()).collect(),
        started_at: started_at.to_owned(),
        upload_bytes: 0,
        download_bytes: 0,
    }
}

#[test]
fn rule_mode_uses_the_final_node_from_the_newest_active_connection() {
    let connections = vec![
        connection("2026-07-31T10:00:00Z", &["Proxy", "Hong Kong 01"]),
        connection("2026-07-31T10:01:00Z", &["Streaming", "United States 02"]),
    ];

    assert_eq!(
        latest_active_rule_node(&connections).as_deref(),
        Some("United States 02")
    );
}

#[test]
fn direct_active_rule_matches_are_reported_as_direct() {
    assert_eq!(
        latest_active_rule_node(&[connection("2026-07-31T10:00:00Z", &["DIRECT"])]).as_deref(),
        Some("DIRECT")
    );
}

#[test]
fn rule_mode_has_no_active_node_before_the_first_connection() {
    assert_eq!(latest_active_rule_node(&[]), None);
}
