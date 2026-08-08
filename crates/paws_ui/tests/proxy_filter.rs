#[path = "../src/proxy_filter.rs"]
mod proxy_filter;

use paws_model::ProxyItem;
use proxy_filter::matches_proxy_query;

fn proxy(name: &str, proxy_type: &str, delay_ms: Option<u32>, selected: bool) -> ProxyItem {
    ProxyItem {
        name: name.to_owned(),
        proxy_type: proxy_type.to_owned(),
        delay_ms,
        selected,
    }
}

#[test]
fn matches_proxy_query_across_proxy_fields() {
    let item = proxy("HK Premium 01", "vless", Some(128), false);

    assert!(matches_proxy_query(&item, ""));
    assert!(matches_proxy_query(&item, "hk"));
    assert!(matches_proxy_query(&item, "VLESS"));
    assert!(matches_proxy_query(&item, "128"));
    assert!(!matches_proxy_query(&item, "trojan"));
}

#[test]
fn matches_selected_proxy_aliases() {
    let selected = proxy("DIRECT", "direct", None, true);
    let unselected = proxy("DIRECT", "direct", None, false);

    assert!(matches_proxy_query(&selected, "selected"));
    assert!(matches_proxy_query(&selected, "已选"));
    assert!(matches_proxy_query(&selected, "当前"));
    assert!(!matches_proxy_query(&unselected, "selected"));
}
