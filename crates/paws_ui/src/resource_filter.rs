use paws_model::{GeodataFileSummary, ProviderSummary, RuleSummary};

pub(crate) fn matches_provider_query(provider: &ProviderSummary, query: &str) -> bool {
    let query = normalized_query(query);
    if query.is_empty() {
        return true;
    }
    contains(&provider.name, &query)
        || contains(&provider.provider_type, &query)
        || provider
            .vehicle_type
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .url
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .path
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .interval_seconds
            .is_some_and(|value| value.to_string().contains(&query))
        || provider
            .filter
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .exclude_filter
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .behavior
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .format
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .health_check_url
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || provider
            .health_check_interval_seconds
            .is_some_and(|value| value.to_string().contains(&query))
        || provider
            .last_refresh_error
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || (provider.cache_exists && matches!(query.as_str(), "cached" | "已缓存" | "缓存"))
        || (provider.stale_cache_available
            && matches!(query.as_str(), "stale" | "旧缓存" | "失败缓存"))
        || (provider.health_check_enabled
            && matches!(query.as_str(), "health" | "health-check" | "健康检查"))
}

pub(crate) fn matches_rule_query(rule: &RuleSummary, query: &str) -> bool {
    let query = normalized_query(query);
    if query.is_empty() {
        return true;
    }
    contains(&rule.line, &query)
        || contains(&rule.source, &query)
        || rule.order.to_string().contains(&query)
        || (rule.enabled && matches!(query.as_str(), "enabled" | "启用" | "on"))
        || (!rule.enabled && matches!(query.as_str(), "disabled" | "停用" | "关闭" | "off"))
}

pub(crate) fn matches_geodata_query(file: &GeodataFileSummary, query: &str) -> bool {
    let query = normalized_query(query);
    if query.is_empty() {
        return true;
    }
    contains(&file.name, &query)
        || contains(&file.path, &query)
        || (file.exists && matches!(query.as_str(), "available" | "可用" | "exists"))
        || (!file.exists && matches!(query.as_str(), "missing" | "缺失"))
}

fn normalized_query(query: &str) -> String {
    query.trim().to_ascii_lowercase()
}

fn contains(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}
