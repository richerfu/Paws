#[path = "../src/resource_filter.rs"]
mod resource_filter;

use paws_model::{GeodataFileSummary, ProviderSummary, RuleSummary};
use resource_filter::{matches_geodata_query, matches_provider_query, matches_rule_query};

fn provider() -> ProviderSummary {
    ProviderSummary {
        name: "ProxyProviderHK".to_owned(),
        provider_type: "proxy".to_owned(),
        path: Some("/data/providers/proxy/hk.yaml".to_owned()),
        url: Some("https://example.test/hk.yaml".to_owned()),
        vehicle_type: Some("http".to_owned()),
        interval_seconds: Some(3600),
        filter: Some("香港|HK".to_owned()),
        exclude_filter: Some("Premium".to_owned()),
        behavior: Some("domain".to_owned()),
        format: Some("mrs".to_owned()),
        health_check_enabled: true,
        health_check_url: Some("https://cp.cloudflare.com/generate_204".to_owned()),
        health_check_interval_seconds: Some(600),
        expected_status: None,
        members: Vec::new(),
        cache_exists: true,
        cache_bytes: Some(128),
        cache_updated_at: Some("1700000000".to_owned()),
        stale_cache_available: false,
        last_refresh_at: Some("1700000000".to_owned()),
        last_refresh_error: None,
    }
}

fn rule() -> RuleSummary {
    RuleSummary {
        id: "rule-1".to_owned(),
        profile_id: "profile-1".to_owned(),
        line: "DOMAIN-SUFFIX,example.com,Proxy".to_owned(),
        enabled: true,
        order: 3,
        source: "custom".to_owned(),
    }
}

fn geodata(exists: bool) -> GeodataFileSummary {
    GeodataFileSummary {
        name: "geosite.dat".to_owned(),
        path: "/data/geodata/geosite.dat".to_owned(),
        exists,
        bytes: Some(1024),
        updated_at: Some("1700000000".to_owned()),
    }
}

#[test]
fn provider_filter_matches_resource_fields_and_state() {
    let mut item = provider();

    assert!(matches_provider_query(&item, ""));
    assert!(matches_provider_query(&item, "hk"));
    assert!(matches_provider_query(&item, "proxy"));
    assert!(matches_provider_query(&item, "example.test"));
    assert!(matches_provider_query(&item, "3600"));
    assert!(matches_provider_query(&item, "香港"));
    assert!(matches_provider_query(&item, "premium"));
    assert!(matches_provider_query(&item, "domain"));
    assert!(matches_provider_query(&item, "mrs"));
    assert!(matches_provider_query(&item, "cloudflare"));
    assert!(matches_provider_query(&item, "健康检查"));
    assert!(matches_provider_query(&item, "已缓存"));
    assert!(!matches_provider_query(&item, "rule-provider"));

    item.stale_cache_available = true;
    assert!(matches_provider_query(&item, "旧缓存"));
}

#[test]
fn rule_filter_matches_rule_line_source_order_and_state() {
    let mut item = rule();

    assert!(matches_rule_query(&item, "example.com"));
    assert!(matches_rule_query(&item, "custom"));
    assert!(matches_rule_query(&item, "3"));
    assert!(matches_rule_query(&item, "启用"));
    assert!(!matches_rule_query(&item, "停用"));

    item.enabled = false;
    assert!(matches_rule_query(&item, "停用"));
}

#[test]
fn geodata_filter_matches_name_path_and_availability() {
    let available = geodata(true);
    let missing = geodata(false);

    assert!(matches_geodata_query(&available, "geosite"));
    assert!(matches_geodata_query(&available, "geodata"));
    assert!(matches_geodata_query(&available, "可用"));
    assert!(matches_geodata_query(&missing, "缺失"));
    assert!(!matches_geodata_query(&available, "Country"));
}
