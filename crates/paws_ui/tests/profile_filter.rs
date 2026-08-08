#[path = "../src/profile_filter.rs"]
mod profile_filter;

use std::collections::BTreeMap;

use paws_model::{ProfileSummary, SubscriptionMetadata};
use profile_filter::matches_profile_query;

fn profile() -> ProfileSummary {
    ProfileSummary {
        id: "profile-1".to_owned(),
        name: "Work Remote".to_owned(),
        source: "url-import".to_owned(),
        raw_yaml_path: "/data/app/profiles/profile-1.yaml".to_owned(),
        runtime_yaml_path: "/data/app/runtime/profile-1.yaml".to_owned(),
        active: true,
        updated_at: Some("1700000000".to_owned()),
        last_refresh_at: Some("1700000100".to_owned()),
        last_refresh_error: None,
        subscription_url: Some("https://sub.example/test.yaml".to_owned()),
        rule_count: 42,
        selected_proxies: BTreeMap::new(),
        has_backup: false,
        upload_bytes: 0,
        download_bytes: 0,
        subscription_user_info: None,
        subscription_metadata: Some(SubscriptionMetadata {
            title: Some("Remote Title".to_owned()),
            update_interval_hours: Some(24),
            web_page_url: Some("https://portal.example".to_owned()),
            support_url: Some("https://help.example".to_owned()),
        }),
        next_refresh_at: Some("1700086500".to_owned()),
        refresh_due: false,
    }
}

#[test]
fn matches_profile_identity_source_metadata_and_state() {
    let item = profile();

    assert!(matches_profile_query(&item, ""));
    assert!(matches_profile_query(&item, "work"));
    assert!(matches_profile_query(&item, "url-import"));
    assert!(matches_profile_query(&item, "runtime/profile-1.yaml"));
    assert!(matches_profile_query(&item, "sub.example"));
    assert!(matches_profile_query(&item, "Remote Title"));
    assert!(matches_profile_query(&item, "portal.example"));
    assert!(matches_profile_query(&item, "使用中"));
    assert!(matches_profile_query(&item, "订阅"));
    assert!(!matches_profile_query(&item, "local-only"));
}

#[test]
fn matches_refresh_error_due_backup_and_local_aliases() {
    let mut item = profile();
    item.subscription_url = None;
    item.last_refresh_error = Some("timeout while fetching subscription".to_owned());
    item.refresh_due = true;
    item.has_backup = true;

    assert!(matches_profile_query(&item, "timeout"));
    assert!(matches_profile_query(&item, "失败"));
    assert!(matches_profile_query(&item, "待刷新"));
    assert!(matches_profile_query(&item, "备份"));
    assert!(matches_profile_query(&item, "本地"));
    assert!(!matches_profile_query(&item, "订阅"));
}
