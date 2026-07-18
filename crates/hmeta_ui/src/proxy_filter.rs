use hmeta_model::ProxyItem;

pub(crate) fn matches_proxy_query(proxy: &ProxyItem, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    contains(&proxy.name, &query)
        || contains(&proxy.proxy_type, &query)
        || (proxy.selected && matches!(query.as_str(), "selected" | "已选" | "当前"))
        || proxy
            .delay_ms
            .is_some_and(|delay| delay.to_string().contains(&query))
}

fn contains(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}
