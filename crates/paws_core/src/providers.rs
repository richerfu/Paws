use super::*;

pub(super) fn mark_provider_refresh(
    state: &mut CoreState,
    provider_type: &str,
    provider_name: &str,
    refreshed_at: String,
    error: Option<String>,
) {
    let key = provider_refresh_key(provider_type, provider_name);
    let refresh = ProviderRefreshState {
        refreshed_at,
        error,
    };
    state.provider_refresh.insert(key, refresh);
    apply_provider_refresh_states(&mut state.providers, &state.provider_refresh);
}

pub(super) fn apply_provider_refresh_states(
    providers: &mut [ProviderSummary],
    refresh: &HashMap<String, ProviderRefreshState>,
) {
    for provider in providers.iter_mut() {
        if let Some(record) = refresh.get(&provider_refresh_key(
            &provider.provider_type,
            &provider.name,
        )) {
            provider.last_refresh_at = Some(record.refreshed_at.clone());
            provider.last_refresh_error = record.error.clone();
        }
    }
    refresh_provider_cache_metadata(providers);
}

pub(super) fn provider_refresh_key(provider_type: &str, provider_name: &str) -> String {
    format!("{provider_type}:{provider_name}")
}

pub(super) fn provider_is_inline(provider: &ProviderSummary) -> bool {
    provider
        .vehicle_type
        .as_deref()
        .is_some_and(|vehicle_type| vehicle_type.eq_ignore_ascii_case("inline"))
}

pub(super) fn provider_stale_cache_available(
    state: &CoreState,
    provider_type: &str,
    provider_name: &str,
) -> bool {
    state
        .providers
        .iter()
        .find(|provider| provider.provider_type == provider_type && provider.name == provider_name)
        .is_some_and(|provider| provider.stale_cache_available)
}

pub(super) fn provider_refresh_failure_log_message(
    message: &str,
    stale_cache_available: bool,
) -> String {
    if stale_cache_available {
        format!("{message}; stale provider cache retained")
    } else {
        message.to_owned()
    }
}

pub(super) fn refresh_provider_cache_metadata(providers: &mut [ProviderSummary]) {
    for provider in providers {
        let metadata = provider
            .path
            .as_deref()
            .and_then(|path| Path::new(path).metadata().ok())
            .filter(std::fs::Metadata::is_file);
        provider.cache_exists = metadata.is_some();
        provider.cache_bytes = metadata.as_ref().map(std::fs::Metadata::len);
        provider.cache_updated_at = metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(system_time_secs);
        provider.stale_cache_available =
            provider.last_refresh_error.is_some() && provider.cache_exists;
    }
}

pub(super) fn enrich_proxy_provider_members(
    providers: &mut [ProviderSummary],
    registry: &dashmap::DashMap<String, Arc<ProxyProvider>>,
) {
    for provider in providers
        .iter_mut()
        .filter(|provider| provider.provider_type == "proxy")
    {
        let Some(runtime) = registry.get(&provider.name) else {
            provider.members.clear();
            continue;
        };
        if let Some(health_check) = runtime.health_check.as_ref() {
            provider.health_check_enabled = true;
            provider.health_check_url = Some(health_check.url.clone());
            provider.health_check_interval_seconds = Some(health_check.interval);
            provider.expected_status = Some(health_check.expected_status.clone());
        }
        let members: Vec<_> = runtime
            .proxies()
            .into_iter()
            .map(|proxy| ProviderProxySummary {
                name: proxy.name().to_owned(),
                proxy_type: proxy.adapter_type().to_string(),
                alive: proxy.alive(),
                delay_ms: (!proxy.delay_history().is_empty())
                    .then(|| u32::from(proxy.last_delay())),
            })
            .collect();
        provider.members = members;
    }
}
