#[path = "../src/activity_filter.rs"]
mod activity_filter;

use activity_filter::{
    matches_connection_query, matches_request_filter, request_connection_query, RequestStatusFilter,
};
use hmeta_model::{ConnectionSummary, RequestSummary};

fn connection(host: &str, rule: &str, proxy: &str) -> ConnectionSummary {
    ConnectionSummary {
        id: "conn-1".to_owned(),
        host: host.to_owned(),
        network: "tcp".to_owned(),
        rule: rule.to_owned(),
        rule_payload: "example.com".to_owned(),
        proxy: proxy.to_owned(),
        chains: vec!["Auto".to_owned(), proxy.to_owned()],
        started_at: "1893456000".to_owned(),
        upload_bytes: 10,
        download_bytes: 20,
    }
}

fn request(active: bool, host: &str, rule: &str, proxy: &str) -> RequestSummary {
    RequestSummary {
        id: "req-1".to_owned(),
        host: host.to_owned(),
        network: "udp".to_owned(),
        rule: rule.to_owned(),
        proxy: proxy.to_owned(),
        upload_bytes: 1,
        download_bytes: 2,
        active,
        updated_at: "1893456000".to_owned(),
    }
}

#[test]
fn matches_connection_query_across_connection_fields() {
    let item = connection("api.example.com:443", "DOMAIN(example.com)", "Proxy A");

    assert!(matches_connection_query(&item, ""));
    assert!(matches_connection_query(&item, "API.EXAMPLE"));
    assert!(matches_connection_query(&item, "domain"));
    assert!(matches_connection_query(&item, "example.com"));
    assert!(matches_connection_query(&item, "auto"));
    assert!(matches_connection_query(&item, "proxy a"));
    assert!(matches_connection_query(&item, "tcp"));
    assert!(matches_connection_query(&item, "189345"));
    assert!(!matches_connection_query(&item, "DIRECT"));
}

#[test]
fn matches_request_status_and_query_filters() {
    let active = request(
        true,
        "api.example.com:443",
        "DOMAIN(example.com)",
        "Proxy A",
    );
    let ended = request(false, "cdn.example.com:443", "MATCH", "DIRECT");

    assert!(matches_request_filter(
        &active,
        RequestStatusFilter::Active,
        "proxy"
    ));
    assert!(!matches_request_filter(
        &active,
        RequestStatusFilter::Ended,
        "proxy"
    ));
    assert!(matches_request_filter(
        &ended,
        RequestStatusFilter::Ended,
        "direct"
    ));
    assert!(matches_request_filter(
        &ended,
        RequestStatusFilter::All,
        "189345"
    ));
    assert!(!matches_request_filter(
        &ended,
        RequestStatusFilter::All,
        "proxy"
    ));
}

#[test]
fn exposes_request_status_filters_for_ui_segments() {
    assert_eq!(
        RequestStatusFilter::ALL,
        [
            RequestStatusFilter::All,
            RequestStatusFilter::Active,
            RequestStatusFilter::Ended
        ]
    );
}

#[test]
fn request_connection_query_targets_the_connection_host() {
    let item = request(
        true,
        " api.example.com:443 ",
        "DOMAIN(example.com)",
        "Proxy A",
    );

    assert_eq!(request_connection_query(&item), "api.example.com:443");
}
