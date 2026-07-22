#![allow(dead_code)]

#[path = "../src/l10n.rs"]
mod l10n;

use l10n::{strings, UiLocale};

#[test]
fn locale_parser_prefers_english_for_en_tags() {
    assert_eq!(UiLocale::from_language_tag("en-US"), UiLocale::En);
    assert_eq!(UiLocale::from_language_tag("EN"), UiLocale::En);
}

#[test]
fn locale_parser_defaults_to_zh_cn() {
    assert_eq!(UiLocale::from_language_tag("zh-Hans-CN"), UiLocale::ZhCn);
    assert_eq!(UiLocale::from_language_tag("fr-FR"), UiLocale::ZhCn);
}

#[test]
fn localized_strings_cover_navigation_and_about_privacy() {
    let zh = strings(UiLocale::ZhCn);
    let en = strings(UiLocale::En);

    assert_eq!(zh.nav_about, "关于");
    assert_eq!(en.nav_about, "About");
    assert_eq!(zh.dashboard_speed_title, "网络速度");
    assert_eq!(en.dashboard_speed_title, "Network Speed");
    assert_eq!(zh.dashboard_start_vpn, "启动 VPN");
    assert_eq!(en.dashboard_start_vpn, "Start VPN");
    assert_eq!(zh.proxies_search_label, "搜索");
    assert_eq!(en.proxies_search_label, "Search");
    assert_eq!(zh.proxies_direct, "直连");
    assert_eq!(en.proxies_direct, "Direct");
    assert_eq!(zh.proxies_untested, "未测速");
    assert_eq!(en.proxies_untested, "Not tested");
    assert_eq!(zh.profiles_search_label, "搜索");
    assert_eq!(en.profiles_search_label, "Search");
    assert_eq!(zh.profiles_status_active, "使用中");
    assert_eq!(en.profiles_status_active, "Active");
    assert_eq!(zh.profiles_refresh_status_due, "待刷新");
    assert_eq!(en.profiles_refresh_status_due, "Due");
    assert_eq!(zh.profiles_source_subscription, "网络订阅 · 已保存到本地");
    assert_eq!(
        en.profiles_source_subscription,
        "Subscription · saved locally"
    );
    assert_eq!(zh.profiles_import_network, "网络导入");
    assert_eq!(en.profiles_import_network, "Import from URL");
    assert_eq!(zh.profiles_import_url_required, "请输入配置 URL");
    assert_eq!(en.profiles_import_url_required, "Enter a profile URL");
    assert_eq!(zh.profiles_yaml_editor_title, "编辑 YAML");
    assert_eq!(en.profiles_yaml_editor_title, "Edit YAML");
    assert_eq!(zh.profiles_yaml_parseable, "YAML 可解析");
    assert_eq!(en.profiles_yaml_parseable, "YAML parseable");
    assert_eq!(zh.profiles_yaml_valid, "YAML 校验通过");
    assert_eq!(en.profiles_yaml_valid, "YAML validation passed");
    assert_eq!(zh.connections_empty_title, "暂无连接");
    assert_eq!(en.connections_empty_title, "No connections");
    assert_eq!(zh.requests_status_all, "全部");
    assert_eq!(en.requests_status_all, "All");
    assert_eq!(zh.requests_view_connection, "查看连接");
    assert_eq!(en.requests_view_connection, "View Connection");
    assert_eq!(zh.traffic_realtime_title, "实时速度");
    assert_eq!(en.traffic_realtime_title, "Realtime Speed");
    assert_eq!(zh.traffic_dns_title, "DNS 诊断");
    assert_eq!(en.traffic_dns_title, "DNS Diagnostics");
    assert_eq!(zh.traffic_dns_cache, "缓存");
    assert_eq!(en.traffic_dns_cache, "Cache");
    assert_eq!(zh.dns_cache_hits, "命中");
    assert_eq!(en.dns_cache_hits, "hits");
    assert_eq!(zh.dns_cache_misses, "回源");
    assert_eq!(en.dns_cache_misses, "misses");
    assert_eq!(zh.dns_model_hijack, "TUN UDP/53 拦截");
    assert_eq!(en.dns_model_hijack, "TUN UDP/53 hijack");
    assert_eq!(zh.logs_empty_title, "暂无日志");
    assert_eq!(en.logs_empty_title, "No logs");
    assert_eq!(zh.logs_level_all, "全部");
    assert_eq!(en.logs_level_all, "All");
    assert_eq!(zh.resources_search_label, "搜索");
    assert_eq!(en.resources_search_label, "Search");
    assert_eq!(zh.resources_empty_title, "暂无资源");
    assert_eq!(en.resources_empty_title, "No resources");
    assert_eq!(zh.resources_refresh_status_failed_stale, "失败，使用旧缓存");
    assert_eq!(
        en.resources_refresh_status_failed_stale,
        "Failed, using old cache"
    );
    assert_eq!(zh.resources_geodata_ready, "离线资源就绪");
    assert_eq!(en.resources_geodata_ready, "Offline resources ready");
    assert_eq!(zh.settings_save_vpn, "保存 VPN 设置");
    assert_eq!(en.settings_save_vpn, "Save VPN Settings");
    assert_eq!(zh.settings_dns_hijack, "DNS 劫持");
    assert_eq!(en.settings_dns_hijack, "DNS Hijack");
    assert_eq!(zh.settings_cache, "缓存");
    assert_eq!(en.settings_cache, "Cache");
    assert_eq!(zh.feedback_proxy_delay_empty, "当前配置没有可测速节点");
    assert_eq!(
        en.feedback_proxy_delay_empty,
        "No testable nodes in the current profile"
    );
    assert_eq!(zh.feedback_connection_closed, "连接已断开");
    assert_eq!(en.feedback_connection_closed, "Connection closed");
    assert_eq!(zh.feedback_request_history_cleared, "请求历史已清空");
    assert_eq!(
        en.feedback_request_history_cleared,
        "Request history cleared"
    );
    assert_eq!(zh.feedback_logs_cleared, "日志已清空");
    assert_eq!(en.feedback_logs_cleared, "Logs cleared");
    assert_eq!(
        zh.feedback_subscription_refresh_failed_prefix,
        "刷新订阅失败："
    );
    assert_eq!(
        en.feedback_subscription_refresh_failed_prefix,
        "Refresh subscriptions failed: "
    );
    assert_eq!(zh.feedback_resource_refresh_failed_prefix, "资源刷新失败：");
    assert_eq!(
        en.feedback_resource_refresh_failed_prefix,
        "Resource refresh failed: "
    );
    assert_eq!(zh.feedback_active_profile_required, "请先启用一个配置");
    assert_eq!(
        en.feedback_active_profile_required,
        "Activate a profile first"
    );
    assert_eq!(zh.feedback_dns_upstream_required, "请至少填写一个 DNS 上游");
    assert_eq!(
        en.feedback_dns_upstream_required,
        "Enter at least one DNS upstream"
    );
    assert_eq!(
        zh.feedback_vpn_start_options_failed_prefix,
        "VPN 启动失败：读取 VPN 参数失败："
    );
    assert_eq!(
        en.feedback_vpn_start_options_failed_prefix,
        "VPN start failed: read VPN options failed: "
    );
    assert_eq!(
        zh.feedback_vpn_stop_fallback_applied_prefix,
        "停止回调失败，已回退本地停止："
    );
    assert_eq!(
        en.feedback_vpn_stop_fallback_applied_prefix,
        "Stop callback failed; fell back to local stop: "
    );
    assert_eq!(
        zh.feedback_dns_policy_format_error,
        "DNS 分流规则请使用 matcher = dns1, dns2"
    );
    assert_eq!(
        en.feedback_dns_policy_format_error,
        "Use matcher = dns1, dns2 for DNS policy rules"
    );
    assert_eq!(zh.proxies_filter_count_prefix, "显示");
    assert_eq!(en.proxies_filter_count_prefix, "Showing");
    assert_eq!(zh.tools_network_title, "网络检测");
    assert_eq!(en.tools_network_title, "Network Check");
    assert_eq!(zh.tools_status_rules_unit, "条规则");
    assert_eq!(en.tools_status_rules_unit, "rules");
    assert!(zh.about_privacy_subtitle.contains("本机"));
    assert!(en.about_privacy_subtitle.contains("stay local"));
    assert_eq!(UiLocale::ZhCn.language_tag(), "zh-CN");
    assert_eq!(UiLocale::En.language_tag(), "en");
}
