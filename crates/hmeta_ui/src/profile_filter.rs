use hmeta_model::ProfileSummary;

pub(crate) fn matches_profile_query(profile: &ProfileSummary, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    contains(&profile.name, &query)
        || contains(&profile.source, &query)
        || contains(&profile.raw_yaml_path, &query)
        || contains(&profile.runtime_yaml_path, &query)
        || profile
            .subscription_url
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || profile
            .updated_at
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || profile
            .last_refresh_at
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || profile
            .last_refresh_error
            .as_deref()
            .is_some_and(|value| contains(value, &query))
        || profile
            .subscription_metadata
            .as_ref()
            .is_some_and(|metadata| {
                metadata
                    .title
                    .as_deref()
                    .is_some_and(|value| contains(value, &query))
                    || metadata
                        .web_page_url
                        .as_deref()
                        .is_some_and(|value| contains(value, &query))
                    || metadata
                        .support_url
                        .as_deref()
                        .is_some_and(|value| contains(value, &query))
            })
        || (profile.active && matches!(query.as_str(), "active" | "使用中" | "当前"))
        || (profile.subscription_url.is_some()
            && matches!(query.as_str(), "remote" | "subscription" | "订阅" | "网络"))
        || (profile.subscription_url.is_none()
            && matches!(query.as_str(), "local" | "本地" | "yaml"))
        || (profile.refresh_due && matches!(query.as_str(), "due" | "待刷新" | "可刷新"))
        || (profile.last_refresh_error.is_some()
            && matches!(query.as_str(), "failed" | "失败" | "错误"))
        || (profile.has_backup && matches!(query.as_str(), "backup" | "备份" | "可回滚"))
}

fn contains(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}
