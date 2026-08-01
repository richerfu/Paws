use super::*;
use base64::Engine;

#[test]
fn new_store_starts_without_profiles() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("seed")));
    let store = ProfileStore::open(root).unwrap();
    assert_eq!(store.active_profile(), None);
    assert!(store.summaries().is_empty());
    assert!(store.active_raw_yaml().is_err());
}

#[test]
fn profile_summary_exposes_raw_and_runtime_yaml_paths() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("summary-yaml-paths")
    ));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    store.seed_default().unwrap();

    let summary = store.summaries().into_iter().next().expect("summary");

    assert_eq!(
        summary.raw_yaml_path,
        root.join("profiles/default.yaml").to_string_lossy()
    );
    assert_eq!(
        summary.runtime_yaml_path,
        root.join("runtime/default.yaml").to_string_lossy()
    );
}

#[test]
fn runtime_yaml_merges_rules_and_options() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("runtime")));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();
    let rule_ids = store
        .import_rules_for_profile("default", "manual", "DOMAIN-SUFFIX,example.com,DIRECT")
        .unwrap();
    let yaml = store
        .build_runtime_yaml("default", RuntimeMode::Global, &VpnOptions::default())
        .unwrap();
    assert!(yaml.contains("mode: global"));
    assert!(yaml.contains("DOMAIN-SUFFIX,example.com,DIRECT"));
    assert!(yaml.contains("tun:"));

    store
        .set_rule_enabled("default", &rule_ids[0], false)
        .unwrap();
    let yaml = store
        .build_runtime_yaml("default", RuntimeMode::Global, &VpnOptions::default())
        .unwrap();
    assert!(!yaml.contains("DOMAIN-SUFFIX,example.com,DIRECT"));
    assert!(store
        .rules_for_profile("default")
        .iter()
        .any(|rule| rule.id == rule_ids[0] && !rule.enabled));

    store.delete_rule(&rule_ids[0]).unwrap();
    assert!(store.rules_for_profile("default").is_empty());
}

#[test]
fn imports_custom_rules_from_text_and_clash_yaml() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("custom-rule-import")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();

    store
        .import_rules_for_profile(
            "default",
            "plain-text",
            "\u{feff}# local overrides\nDOMAIN-SUFFIX,example.com,DIRECT\n\nMATCH,Proxy\n",
        )
        .unwrap();
    store
        .import_rules_for_profile(
            "default",
            "clash-yaml",
            r#"
mixed-port: 7890
rules:
  - DOMAIN,api.example.com,Proxy
  - "IP-CIDR,192.0.2.0/24,DIRECT,no-resolve"
"#,
        )
        .unwrap();

    let rules = store.rules_for_profile("default");
    assert_eq!(rules.len(), 4);
    assert_eq!(rules[0].line, "DOMAIN-SUFFIX,example.com,DIRECT");
    assert_eq!(rules[1].line, "MATCH,Proxy");
    assert_eq!(rules[2].line, "DOMAIN,api.example.com,Proxy");
    assert_eq!(rules[3].line, "IP-CIDR,192.0.2.0/24,DIRECT,no-resolve");
}

#[test]
fn custom_rule_import_rejects_empty_or_unrelated_files() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("invalid-rule-import")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();

    assert!(store
        .import_rules_for_profile("default", "empty", "# comments only\n")
        .is_err());
    assert!(store
        .import_rules_for_profile(
            "default",
            "unrelated-yaml",
            "proxy-providers:\n  remote:\n    type: http\n",
        )
        .is_err());
    assert!(store
        .import_rules_for_profile("default", "invalid-rule", "not-a-rule")
        .is_err());
    assert!(store.rules_for_profile("default").is_empty());
}

#[test]
fn activity_rules_are_normalized_and_replace_conflicting_targets() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("activity-rule-upsert")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();
    store
        .import_rules_for_profile(
            "default",
            "manual-file",
            "DOMAIN,other.example,DIRECT\nDOMAIN-SUFFIX,Example.COM.,Proxy",
        )
        .unwrap();

    let mutation = store
        .stage_manual_rule(
            "default",
            &ManualRuleSpec {
                match_kind: ManualRuleMatchKind::DomainSuffix,
                value: ".EXAMPLE.com.".to_owned(),
                target: "DIRECT".to_owned(),
            },
        )
        .unwrap();

    assert_eq!(mutation.kind, ManualRuleMutationKind::Updated);
    assert_eq!(mutation.line, "DOMAIN-SUFFIX,example.com,DIRECT");
    assert_eq!(
        mutation.replaced_line.as_deref(),
        Some("DOMAIN-SUFFIX,Example.COM.,Proxy")
    );
    let rules = store.rules_for_profile("default");
    assert_eq!(rules[0].line, mutation.line);
    assert_eq!(rules[0].source, MANUAL_ACTIVITY_RULE_SOURCE);
    assert_eq!(rules[1].line, "DOMAIN,other.example,DIRECT");
}

#[test]
fn activity_ip_rules_use_canonical_host_prefixes_and_deduplicate() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("activity-ip-rule")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();
    store
        .import_rules_for_profile(
            "default",
            "manual-a",
            "IP-CIDR,192.0.2.7/32,DIRECT\nIP-CIDR,192.0.2.7/32,Proxy",
        )
        .unwrap();

    let mutation = store
        .stage_manual_rule(
            "default",
            &ManualRuleSpec {
                match_kind: ManualRuleMatchKind::IpCidr,
                value: "192.0.2.7".to_owned(),
                target: "Proxy".to_owned(),
            },
        )
        .unwrap();

    assert_eq!(mutation.line, "IP-CIDR,192.0.2.7/32,Proxy");
    assert_eq!(mutation.removed_duplicates, 1);
    assert_eq!(
        store
            .rules_for_profile("default")
            .iter()
            .filter(|rule| rule.line.contains("192.0.2.7/32"))
            .count(),
        1
    );
}

#[test]
fn activity_rules_reject_domain_ip_mixups_and_invalid_prefixes() {
    assert!(normalize_manual_rule_spec(&ManualRuleSpec {
        match_kind: ManualRuleMatchKind::Domain,
        value: "192.0.2.1".to_owned(),
        target: "DIRECT".to_owned(),
    })
    .is_err());
    assert!(normalize_manual_rule_spec(&ManualRuleSpec {
        match_kind: ManualRuleMatchKind::IpCidr,
        value: "192.0.2.1/64".to_owned(),
        target: "DIRECT".to_owned(),
    })
    .is_err());
}

#[test]
fn rule_reorder_updates_runtime_yaml_order() {
    let root =
        std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("reorder-rules")));
    let mut store = ProfileStore::open(root).unwrap();
    store.seed_default().unwrap();
    let rule_ids = store
        .import_rules_for_profile(
            "default",
            "manual",
            "DOMAIN-SUFFIX,alpha.example,DIRECT\nDOMAIN-SUFFIX,beta.example,Proxy",
        )
        .unwrap();

    store
        .reorder_rules("default", &[rule_ids[1].clone(), rule_ids[0].clone()])
        .unwrap();
    let rules = store.rules_for_profile("default");
    assert_eq!(rules[0].id, rule_ids[1]);
    assert_eq!(rules[1].id, rule_ids[0]);

    let yaml = store
        .build_runtime_yaml("default", RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let beta = yaml.find("DOMAIN-SUFFIX,beta.example,Proxy").unwrap();
    let alpha = yaml.find("DOMAIN-SUFFIX,alpha.example,DIRECT").unwrap();
    assert!(beta < alpha);
}

#[test]
fn runtime_yaml_sanitizes_app_managed_fields_and_injects_geox_url() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("sanitize")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Noisy",
            "local",
            r#"
port: 8080
socks-port: 1080
mixed-port: 9999
external-controller: 0.0.0.0:9990
external-ui: ui
external-ui-name: metacubexd
external-ui-url: https://example.invalid/ui.zip
external-controller-cors:
  allow-origins:
    - "*"
secret: unsafe
authentication:
  - user:pass
skip-auth-prefixes:
  - 127.0.0.1/8
allow-lan: true
bind-address: 0.0.0.0
lan-allowed-ips:
  - 0.0.0.0/0
lan-disallowed-ips:
  - 127.0.0.1/8
external-controller-tls: 0.0.0.0:9443
external-controller-unix: /tmp/meow.sock
external-controller-pipe: meow
routing-mark: 666
interface-name: eth0
tproxy-sni: true
subscriptions:
  - ignored
listeners:
  - name: ignored
geodata:
  mmdb-path: /tmp/user.mmdb
  asn-path: /tmp/user-asn.mmdb
  geosite-path: /tmp/user-geosite.mrs
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
    asn: https://example.invalid/GeoLite2-ASN.mmdb
    geosite: https://example.invalid/geosite.mrs
  geodata-mode: memconservative
  geodata-loader: standard
  geoip-matcher: trie
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
            None,
        )
        .unwrap();

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let runtime_root = value.as_mapping().unwrap();

    assert!(runtime_root.get(&value_key("port")).is_none());
    assert!(runtime_root.get(&value_key("socks-port")).is_none());
    assert!(runtime_root.get(&value_key("allow-lan")).is_none());
    assert!(runtime_root.get(&value_key("bind-address")).is_none());
    assert!(runtime_root.get(&value_key("lan-allowed-ips")).is_none());
    assert!(runtime_root.get(&value_key("lan-disallowed-ips")).is_none());
    assert!(runtime_root.get(&value_key("authentication")).is_none());
    assert!(runtime_root.get(&value_key("skip-auth-prefixes")).is_none());
    assert!(runtime_root.get(&value_key("subscriptions")).is_none());
    assert!(runtime_root.get(&value_key("listeners")).is_none());
    assert!(runtime_root
        .get(&value_key("external-controller-tls"))
        .is_none());
    assert!(runtime_root
        .get(&value_key("external-controller-unix"))
        .is_none());
    assert!(runtime_root
        .get(&value_key("external-controller-pipe"))
        .is_none());
    assert!(runtime_root.get(&value_key("external-ui")).is_none());
    assert!(runtime_root.get(&value_key("external-ui-name")).is_none());
    assert!(runtime_root.get(&value_key("external-ui-url")).is_none());
    assert!(runtime_root
        .get(&value_key("external-controller-cors"))
        .is_none());
    assert!(runtime_root.get(&value_key("secret")).is_none());
    assert!(runtime_root.get(&value_key("routing-mark")).is_none());
    assert!(runtime_root.get(&value_key("interface-name")).is_none());
    assert!(runtime_root.get(&value_key("tproxy-sni")).is_none());
    assert_eq!(
        get_string(runtime_root, "external-controller").as_deref(),
        Some("127.0.0.1:9090")
    );
    assert_eq!(
        runtime_root
            .get(&value_key("mixed-port"))
            .and_then(Value::as_i64),
        Some(7890)
    );
    assert!(matches!(
        runtime_root.get(&value_key("geox-url")),
        Some(Value::Mapping(geox)) if geox.get(&value_key("geoip")).is_some()
    ));
    assert!(root.join("geodata").exists());
    let geodata = runtime_root
        .get(&value_key("geodata"))
        .and_then(Value::as_mapping)
        .expect("geodata paths");
    assert_eq!(
        get_string(geodata, "mmdb-path").as_deref(),
        Some(
            root.join("geodata")
                .join("Country.mmdb")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        get_string(geodata, "asn-path").as_deref(),
        Some(
            root.join("geodata")
                .join("GeoLite2-ASN.mmdb")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        get_string(geodata, "geosite-path").as_deref(),
        Some(
            root.join("geodata")
                .join("geosite.dat")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(geodata.get(&value_key("auto-update")).is_none());
    assert!(geodata.get(&value_key("auto-update-interval")).is_none());
    assert!(geodata.get(&value_key("url")).is_none());
    assert!(geodata.get(&value_key("geodata-mode")).is_none());
    assert!(geodata.get(&value_key("geodata-loader")).is_none());
    assert!(geodata.get(&value_key("geoip-matcher")).is_none());
}

#[test]
fn imported_subscription_profile_persists_metadata_and_yaml() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("subscription")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Remote Demo",
            "https://example.test/demo.yaml",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            Some("https://example.test/demo.yaml".to_owned()),
        )
        .unwrap();
    drop(store);

    let store = ProfileStore::open(root).unwrap();
    let profile = store.profile(&profile_id).unwrap();
    assert_eq!(profile.name, "Remote Demo");
    assert_eq!(
        profile.subscription_url.as_deref(),
        Some("https://example.test/demo.yaml")
    );
    assert!(store.raw_yaml(&profile_id).unwrap().contains("mixed-port"));
    assert!(profile.yaml_backup_path.is_some());
    assert!(profile.last_refresh_at.is_none());
    assert!(profile.last_refresh_error.is_none());
}

#[test]
fn subscription_name_and_url_can_be_edited_like_reference_client() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("edit-subscription")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Old Name",
            "https://old.example.test/profile.yaml",
            "proxies: []\nproxy-groups: []\nrules: []\n",
            Some("https://old.example.test/profile.yaml".to_owned()),
        )
        .unwrap();

    store
        .update_profile_subscription(
            &profile_id,
            "New Name",
            "https://new.example.test/profile.yaml",
        )
        .unwrap();
    let profile = store.profile(&profile_id).unwrap();
    assert_eq!(profile.name, "New Name");
    assert_eq!(
        profile.subscription_url.as_deref(),
        Some("https://new.example.test/profile.yaml")
    );
    assert_eq!(profile.source, "https://new.example.test/profile.yaml");

    assert!(store
        .update_profile_subscription(&profile_id, "", "https://example.test/profile")
        .is_err());
    assert!(store
        .update_profile_subscription(&profile_id, "Name", "file:///tmp/profile.yaml")
        .is_err());
}

#[test]
fn validation_yaml_removes_app_managed_geodata_fields() {
    let yaml = sanitize_profile_for_meow_validation(
        r#"
geodata:
  mmdb-path: /tmp/user.mmdb
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
  geodata-mode: memconservative
  geodata-loader: standard
  geoip-matcher: trie
proxies: []
"#,
    )
    .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let geodata = value
        .as_mapping()
        .and_then(|root| root.get(&value_key("geodata")))
        .and_then(Value::as_mapping)
        .expect("geodata");

    assert_eq!(
        get_string(geodata, "mmdb-path").as_deref(),
        Some("/tmp/user.mmdb")
    );
    assert!(geodata.get(&value_key("auto-update")).is_none());
    assert!(geodata.get(&value_key("auto-update-interval")).is_none());
    assert!(geodata.get(&value_key("url")).is_none());
    assert!(geodata.get(&value_key("geodata-mode")).is_none());
    assert!(geodata.get(&value_key("geodata-loader")).is_none());
    assert!(geodata.get(&value_key("geoip-matcher")).is_none());
}

#[test]
fn store_validation_uses_safe_geodata_paths_and_degrades_generated_defaults_when_missing() {
    let root =
        std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("validation-geo")));
    let yaml = proxy_subscription_yaml(vec![proxy_base(
        "Node".to_owned(),
        "socks5",
        "127.0.0.1".to_owned(),
        1080,
    )])
    .unwrap();

    let missing_yaml = sanitize_profile_for_meow_validation_at(&yaml, &root).unwrap();
    let missing_value: Value = serde_yaml::from_str(&missing_yaml).unwrap();
    let missing_root = missing_value.as_mapping().unwrap();
    assert_eq!(get_string_list(missing_root, "rules"), vec!["MATCH,Proxy"]);
    let geodata = missing_root
        .get(&value_key("geodata"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(geodata, "mmdb-path").as_deref(),
        Some(root.join("geodata/Country.mmdb").to_string_lossy().as_ref())
    );

    fs::write(root.join("geodata/Country.mmdb"), b"test").unwrap();
    fs::write(root.join("geodata/geosite.dat"), b"test").unwrap();
    let available_yaml = sanitize_profile_for_meow_validation_at(&yaml, &root).unwrap();
    let available_value: Value = serde_yaml::from_str(&available_yaml).unwrap();
    assert_eq!(
        get_string_list(available_value.as_mapping().unwrap(), "rules"),
        vec!["GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT", "MATCH,Proxy"]
    );
}

#[test]
fn validation_yaml_removes_app_managed_listener_and_dns_fields() {
    let yaml = sanitize_profile_for_meow_validation(
        r#"
port: 7890
mixed-port: 7890
external-controller: 0.0.0.0:9090
listeners:
  - name: duplicated
    type: mixed
    port: 7890
dns:
  enable: true
  listen: 0.0.0.0:53
  default-nameserver:
    - bad bootstrap
  nameserver:
    - 223.5.5.5
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fallback-filter:
    geoip: true
  use-system-hosts: true
proxies: []
"#,
    )
    .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().expect("root");
    let dns = root
        .get(&value_key("dns"))
        .and_then(Value::as_mapping)
        .expect("dns");

    assert!(root.get(&value_key("port")).is_none());
    assert!(root.get(&value_key("mixed-port")).is_none());
    assert!(root.get(&value_key("external-controller")).is_none());
    assert!(root.get(&value_key("listeners")).is_none());
    assert!(dns.get(&value_key("listen")).is_none());
    assert!(dns.get(&value_key("default-nameserver")).is_none());
    assert!(dns.get(&value_key("enhanced-mode")).is_none());
    assert!(dns.get(&value_key("fake-ip-range")).is_none());
    assert!(dns.get(&value_key("fallback-filter")).is_none());
    assert_eq!(get_bool(dns, "use-system-hosts"), Some(false));
    assert_eq!(get_string_list(dns, "nameserver"), vec!["223.5.5.5"]);
}

#[test]
fn subscription_userinfo_header_is_parsed() {
    let info =
        parse_subscription_userinfo("upload=1024; download=2048; total=4096; expire=1893456000")
            .expect("subscription userinfo");
    assert_eq!(info.upload_bytes, 1024);
    assert_eq!(info.download_bytes, 2048);
    assert_eq!(info.total_bytes, Some(4096));
    assert_eq!(info.expire_at.as_deref(), Some("1893456000"));

    let partial = parse_subscription_userinfo("download=7").expect("partial userinfo");
    assert_eq!(partial.upload_bytes, 0);
    assert_eq!(partial.download_bytes, 7);
    assert_eq!(partial.total_bytes, None);
    assert_eq!(partial.expire_at, None);
    assert!(parse_subscription_userinfo("profile-title=demo").is_none());
}

#[test]
fn subscription_userinfo_comment_is_parsed() {
    let info = parse_subscription_userinfo_comment(
            "# subscription-userinfo: upload=11; download=22; total=33; expire=1893456000;\nproxies: []",
        )
        .expect("comment userinfo");
    assert_eq!(info.upload_bytes, 11);
    assert_eq!(info.download_bytes, 22);
    assert_eq!(info.total_bytes, Some(33));
    assert_eq!(info.expire_at.as_deref(), Some("1893456000"));

    let compact = parse_subscription_userinfo_comment(
        "# upload=44; download=55; total=66; expire=0;\nproxies: []",
    )
    .expect("compact comment userinfo");
    assert_eq!(compact.upload_bytes, 44);
    assert_eq!(compact.download_bytes, 55);
    assert_eq!(compact.total_bytes, Some(66));
    assert_eq!(compact.expire_at, None);
    let second_line = parse_subscription_userinfo_comment(
            "# profile-title: Demo\n# subscription-userinfo: upload=77; download=88; total=99\nproxies: []",
        )
        .expect("second line userinfo");
    assert_eq!(second_line.upload_bytes, 77);
    assert_eq!(second_line.download_bytes, 88);
    assert_eq!(second_line.total_bytes, Some(99));
    assert!(parse_subscription_userinfo_comment("proxies: []").is_none());
    assert!(parse_subscription_userinfo_comment("proxies: []\n# upload=1; download=2").is_none());
}

#[test]
fn imported_profile_uses_subscription_userinfo_comment() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("comment-info")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
            .import_profile_content(
                "Comment Info",
                "local",
                "# upload=12; download=34; total=100; expire=1893456000;\nmixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();
    let profile = store.profile(&profile_id).unwrap();
    let info = profile
        .subscription_user_info
        .as_ref()
        .expect("comment userinfo");
    assert_eq!(info.upload_bytes, 12);
    assert_eq!(info.download_bytes, 34);
    assert_eq!(info.total_bytes, Some(100));
    assert_eq!(info.expire_at.as_deref(), Some("1893456000"));
}

#[test]
fn subscription_metadata_headers_are_parsed() {
    let metadata = parse_subscription_metadata(
        Some("%E6%B5%8B%E8%AF%95%E8%AE%A2%E9%98%85"),
        Some("24"),
        Some("https://example.com/dashboard"),
        Some("https://example.com/support"),
    )
    .expect("subscription metadata");
    assert_eq!(metadata.title.as_deref(), Some("测试订阅"));
    assert_eq!(metadata.update_interval_hours, Some(24));
    assert_eq!(
        metadata.web_page_url.as_deref(),
        Some("https://example.com/dashboard")
    );
    assert_eq!(
        metadata.support_url.as_deref(),
        Some("https://example.com/support")
    );
    assert!(parse_subscription_metadata(None, Some("bad"), Some("ftp://bad"), None).is_none());
}

#[test]
fn subscription_metadata_comment_is_parsed() {
    let metadata = parse_subscription_metadata_comment(
            "# subscription-metadata: profile-title=%E6%B5%8B%E8%AF%95; profile-update-interval=12; profile-web-page-url=https://example.com/portal; support-url=https://example.com/support\nproxies: []",
        )
        .expect("subscription metadata comment");
    assert_eq!(metadata.title.as_deref(), Some("测试"));
    assert_eq!(metadata.update_interval_hours, Some(12));
    assert_eq!(
        metadata.web_page_url.as_deref(),
        Some("https://example.com/portal")
    );
    assert_eq!(
        metadata.support_url.as_deref(),
        Some("https://example.com/support")
    );

    let compact = parse_subscription_metadata_comment(
            "# profile-title=Compact; update_interval=6; web_page_url=https://example.com/home\nproxies: []",
        )
        .expect("compact metadata comment");
    assert_eq!(compact.title.as_deref(), Some("Compact"));
    assert_eq!(compact.update_interval_hours, Some(6));
    assert_eq!(
        compact.web_page_url.as_deref(),
        Some("https://example.com/home")
    );
    let multiline = parse_subscription_metadata_comment(
            "# profile-title: Multi Line\n# profile-update-interval: 18\n# profile-web-page-url: https://example.com/multi\n# support-url: https://example.com/help\nproxies: []",
        )
        .expect("multiline metadata comment");
    assert_eq!(multiline.title.as_deref(), Some("Multi Line"));
    assert_eq!(multiline.update_interval_hours, Some(18));
    assert_eq!(
        multiline.web_page_url.as_deref(),
        Some("https://example.com/multi")
    );
    assert_eq!(
        multiline.support_url.as_deref(),
        Some("https://example.com/help")
    );
    assert!(parse_subscription_metadata_comment("# upload=1; download=2\nproxies: []").is_none());
    assert!(parse_subscription_metadata_comment("proxies: []\n# profile-title: Ignored").is_none());
}

#[test]
fn imported_profile_uses_subscription_metadata_comment_as_fallback() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("comment-metadata")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
            .import_profile_content_with_subscription_metadata(
                "Metadata Comment",
                "local",
                "# profile-title=Body Title; profile-update-interval=8; profile-web-page-url=https://example.com/body; support-url=https://example.com/support\nmixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
                None,
                Some(SubscriptionMetadata {
                    title: Some("Header Title".to_owned()),
                    update_interval_hours: None,
                    web_page_url: None,
                    support_url: None,
                }),
            )
            .unwrap();
    let profile = store.profile(&profile_id).unwrap();
    let metadata = profile
        .subscription_metadata
        .as_ref()
        .expect("merged metadata");
    assert_eq!(metadata.title.as_deref(), Some("Header Title"));
    assert_eq!(metadata.update_interval_hours, Some(8));
    assert_eq!(
        metadata.web_page_url.as_deref(),
        Some("https://example.com/body")
    );
    assert_eq!(
        metadata.support_url.as_deref(),
        Some("https://example.com/support")
    );
}

#[test]
fn content_disposition_filename_is_parsed() {
    assert_eq!(
        parse_content_disposition_filename("attachment; filename*=UTF-8''%E6%B5%8B%E8%AF%95.yaml")
            .as_deref(),
        Some("测试.yaml")
    );
    assert_eq!(
        parse_content_disposition_filename("attachment; filename=\"abc.yaml\"").as_deref(),
        Some("abc.yaml")
    );
}

#[test]
fn subscription_update_interval_marks_due_profiles() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("due-refresh")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content_with_subscription_metadata(
            "Due Demo",
            "https://example.test/due.yaml",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            Some("https://example.test/due.yaml".to_owned()),
            None,
            Some(SubscriptionMetadata {
                title: None,
                update_interval_hours: Some(1),
                web_page_url: None,
                support_url: None,
            }),
        )
        .unwrap();
    {
        let profile = store.profiles.get_mut(&profile_id).unwrap();
        profile.updated_at = Some("1000".to_owned());
        profile.last_refresh_at = Some("1000".to_owned());
    }

    let profile = store.profile(&profile_id).unwrap();
    assert_eq!(profile.next_refresh_at().as_deref(), Some("3600000001000"));
    assert!(!profile.refresh_due_at(3_600_000_000_999));
    assert!(profile.refresh_due_at(3_600_000_001_000));

    let summaries = store.due_subscription_summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, profile_id);
    assert!(summaries[0].refresh_due);
    assert_eq!(
        summaries[0].next_refresh_at.as_deref(),
        Some("3600000001000")
    );
}

#[test]
fn refresh_success_and_failure_metadata_persist_with_profile() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("refresh-meta")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Remote Demo",
            "https://example.test/demo.yaml",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            Some("https://example.test/demo.yaml".to_owned()),
        )
        .unwrap();
    store
        .mark_profile_refresh_failed(&profile_id, "HTTP 500")
        .unwrap();
    let failed = store.profile(&profile_id).unwrap();
    assert_eq!(failed.last_refresh_error.as_deref(), Some("HTTP 500"));
    assert!(failed.last_refresh_at.is_some());

    store
        .replace_profile_content(
            &profile_id,
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
        )
        .unwrap();
    drop(store);

    let store = ProfileStore::open(root).unwrap();
    let profile = store.profile(&profile_id).unwrap();
    assert!(profile.last_refresh_at.is_some());
    assert!(profile.last_refresh_error.is_none());
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.id == profile_id)
        .unwrap();
    assert!(summary.last_refresh_at.is_some());
    assert!(summary.last_refresh_error.is_none());
}

#[test]
fn imported_base64_share_subscription_is_normalized_to_clash_yaml() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("share-sub")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let links = "\
vless://00000000-0000-0000-0000-000000000001@127.0.0.1:443?type=tcp&security=none#VLESS%20A
trojan://secret@proxy.example.test:443?sni=proxy.example.test#Trojan%20A
";
    let encoded = base64::engine::general_purpose::STANDARD.encode(links);
    let profile_id = store
        .import_profile_content(
            "Share Links",
            "subscription",
            encoded,
            Some("https://example.test/links".to_owned()),
        )
        .unwrap();
    let yaml = store.raw_yaml(&profile_id).unwrap();
    assert!(yaml.contains("type: vless"));
    assert!(yaml.contains("type: trojan"));
    assert!(yaml.contains("name: VLESS A"));
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let raw_root = value.as_mapping().unwrap();
    assert_eq!(
        get_string_list(raw_root, "rules"),
        vec!["GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT", "MATCH,Proxy"]
    );

    fs::write(root.join("geodata/Country.mmdb"), b"test").unwrap();
    fs::write(root.join("geodata/geosite.dat"), b"test").unwrap();
    let runtime_yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let runtime_value: Value = serde_yaml::from_str(&runtime_yaml).unwrap();
    let runtime_root = runtime_value.as_mapping().unwrap();
    assert_eq!(
        get_string_list(runtime_root, "rules"),
        vec!["GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT", "MATCH,Proxy"]
    );
    assert!(store.vpn_options_for_profile(&profile_id).is_ok());
}

#[test]
fn legacy_generated_subscription_is_upgraded_in_runtime_without_rewriting_raw_yaml() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("legacy-generated-rules")
    ));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let current_yaml = proxy_subscription_yaml(vec![proxy_base(
        "Node".to_owned(),
        "socks5",
        "127.0.0.1".to_owned(),
        1080,
    )])
    .unwrap();
    let mut legacy_value: Value = serde_yaml::from_str(&current_yaml).unwrap();
    legacy_value.as_mapping_mut().unwrap().insert(
        value_key("rules"),
        Value::Sequence(vec![Value::String("MATCH,Proxy".to_owned())]),
    );
    let legacy_yaml = serde_yaml::to_string(&legacy_value).unwrap();
    let profile_id = store
        .import_profile_content("Legacy", "subscription", &legacy_yaml, None)
        .unwrap();
    store
        .import_rules_for_profile(
            &profile_id,
            MANUAL_ACTIVITY_RULE_SOURCE,
            "DOMAIN-SUFFIX,qq.com,DIRECT",
        )
        .unwrap();
    fs::write(root.join("geodata/Country.mmdb"), b"test").unwrap();
    fs::write(root.join("geodata/geosite.dat"), b"test").unwrap();

    let runtime_yaml = store
        .render_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let runtime_value: Value = serde_yaml::from_str(&runtime_yaml).unwrap();
    assert_eq!(
        get_string_list(runtime_value.as_mapping().unwrap(), "rules"),
        vec![
            "DOMAIN-SUFFIX,qq.com,DIRECT",
            "GEOSITE,cn,DIRECT",
            "GEOIP,CN,DIRECT",
            "MATCH,Proxy"
        ]
    );

    let raw_yaml = store.raw_yaml(&profile_id).unwrap();
    let raw_value: Value = serde_yaml::from_str(&raw_yaml).unwrap();
    assert_eq!(
        get_string_list(raw_value.as_mapping().unwrap(), "rules"),
        vec!["MATCH,Proxy"]
    );
}

#[test]
fn explicit_yaml_subscription_rules_are_preserved() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("explicit-subscription-rules")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Custom rules",
            "subscription",
            r#"
mixed-port: 7890
proxies: []
proxy-groups:
  - name: Custom
    type: select
    proxies:
      - DIRECT
rules:
  - DOMAIN-SUFFIX,example.cn,Custom
  - MATCH,Custom
"#,
            Some("https://example.test/custom.yaml".to_owned()),
        )
        .unwrap();
    let yaml = store.raw_yaml(&profile_id).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();

    assert_eq!(
        get_string_list(root, "rules"),
        vec!["DOMAIN-SUFFIX,example.cn,Custom", "MATCH,Custom"]
    );

    let runtime_yaml = store
        .render_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let runtime_value: Value = serde_yaml::from_str(&runtime_yaml).unwrap();
    assert_eq!(
        get_string_list(runtime_value.as_mapping().unwrap(), "rules"),
        vec!["DOMAIN-SUFFIX,example.cn,Custom", "MATCH,Custom"]
    );
}

#[test]
fn multiline_share_subscription_skips_comments_and_bad_links_when_valid_links_exist() {
    let links = "\
# generated by provider
not-a-share-link
ss://not-valid-base64
// disabled node
vless://00000000-0000-0000-0000-000000000001@127.0.0.1:443?type=tcp&security=none#VLESS%20Good
vmess://not-json
trojan://secret@proxy.example.test:443?sni=proxy.example.test#Trojan%20Good
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let names = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .filter_map(|proxy| get_string(proxy, "name"))
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["VLESS Good", "Trojan Good"]);
    assert_eq!(
        get_string_list(root, "rules"),
        vec!["GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT", "MATCH,Proxy"]
    );
}

#[test]
fn single_bad_share_link_still_reports_parse_error() {
    let err = normalize_profile_content("ss://not-valid-base64").unwrap_err();
    assert!(err.to_string().contains("invalid ss subscription link"));
}

#[test]
fn imported_single_share_link_is_normalized_to_clash_yaml() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("single-share")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
            .import_profile_content(
                "Single",
                "clipboard",
                "vless://00000000-0000-0000-0000-000000000001@example.test:8443?security=tls&sni=edge.example.test#Edge",
                None,
            )
            .unwrap();
    let yaml = store.raw_yaml(&profile_id).unwrap();
    assert!(yaml.contains("server: example.test"));
    assert!(yaml.contains("port: 8443"));
    assert!(yaml.contains("servername: edge.example.test"));
}

#[test]
fn ssr_share_link_is_normalized_to_clash_yaml() {
    let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let body = format!(
            "ssr.example.test:8388:auth_sha1_v4:aes-256-cfb:http_simple:{}/?obfsparam={}&protoparam={}&remarks={}&group={}",
            b64("ssr-pass"),
            b64("cdn.example.test"),
            b64("proto-param"),
            b64("SSR Alias"),
            b64("HMeta")
        );
    let link = format!("ssr://{}", b64(&body));
    let yaml = normalize_profile_content(&link).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let ssr = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SSR Alias"))
        .unwrap();

    assert_eq!(get_string(ssr, "type").as_deref(), Some("ssr"));
    assert_eq!(
        get_string(ssr, "server").as_deref(),
        Some("ssr.example.test")
    );
    assert_eq!(get_u16(ssr, "port"), Some(8388));
    assert_eq!(get_string(ssr, "cipher").as_deref(), Some("aes-256-cfb"));
    assert_eq!(get_string(ssr, "password").as_deref(), Some("ssr-pass"));
    assert_eq!(get_string(ssr, "protocol").as_deref(), Some("auth_sha1_v4"));
    assert_eq!(
        get_string(ssr, "protocol-param").as_deref(),
        Some("proto-param")
    );
    assert_eq!(get_string(ssr, "obfs").as_deref(), Some("http_simple"));
    assert_eq!(
        get_string(ssr, "obfs-param").as_deref(),
        Some("cdn.example.test")
    );
    assert_eq!(get_string(ssr, "group").as_deref(), Some("HMeta"));
    assert_eq!(get_bool(ssr, "udp"), Some(true));
}

#[test]
fn ssr_share_link_query_aliases_are_normalized() {
    let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let body = format!(
            "alias.example.test:9443:auth_chain_a:aes-128-gcm:tls1.2_ticket_auth:{}/?Remark={}&ProtocolParam={}&ObfsParam={}&GroupName={}",
            b64("alias-pass"),
            b64("SSR Alias Case"),
            b64("protocol-case"),
            b64("obfs-case.example.test"),
            b64("Alias Group")
        );
    let link = format!("ssr://{}", b64(&body));
    let yaml = normalize_profile_content(&link).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let ssr = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SSR Alias Case"))
        .unwrap();

    assert_eq!(get_string(ssr, "type").as_deref(), Some("ssr"));
    assert_eq!(
        get_string(ssr, "server").as_deref(),
        Some("alias.example.test")
    );
    assert_eq!(get_u16(ssr, "port"), Some(9443));
    assert_eq!(
        get_string(ssr, "protocol-param").as_deref(),
        Some("protocol-case")
    );
    assert_eq!(
        get_string(ssr, "obfs-param").as_deref(),
        Some("obfs-case.example.test")
    );
    assert_eq!(get_string(ssr, "group").as_deref(), Some("Alias Group"));
    assert_eq!(get_bool(ssr, "udp"), Some(true));
}

#[test]
fn legacy_full_shadowsocks_share_link_is_normalized_to_clash_yaml() {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode("aes-128-gcm:legacy-pass@legacy-ss.example.test:8388");
    let link = format!("ss://{encoded}#Legacy%20SS");
    let yaml = normalize_profile_content(&link).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let ss = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("Legacy SS"))
        .unwrap();

    assert_eq!(get_string(ss, "type").as_deref(), Some("ss"));
    assert_eq!(
        get_string(ss, "server").as_deref(),
        Some("legacy-ss.example.test")
    );
    assert_eq!(get_u16(ss, "port"), Some(8388));
    assert_eq!(get_string(ss, "cipher").as_deref(), Some("aes-128-gcm"));
    assert_eq!(get_string(ss, "password").as_deref(), Some("legacy-pass"));
    assert_eq!(get_bool(ss, "udp"), Some(true));
}

#[test]
fn share_link_schemes_are_case_insensitive() {
    let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let ssr_body = format!(
        "ssr-upper.example.test:8388:auth_sha1_v4:aes-256-cfb:http_simple:{}/?remarks={}",
        b64("ssr-pass"),
        b64("SSR Upper")
    );
    let ssr = b64(&ssr_body);
    let vmess = base64::engine::general_purpose::STANDARD.encode(
            r#"{"V":"2","PS":"VMess Upper","ADD":"vmess-upper.example.test","PORT":"443","ID":"00000000-0000-0000-0000-000000000009","AID":"0","SCY":"auto","NET":"tcp","TLS":"tls","SNI":"vmess-sni.example.test","ALPN":"h2,http/1.1","UDP":true,"FASTOPEN":true}"#,
        );
    let links = format!(
            "\
VLESS://00000000-0000-0000-0000-000000000001@vless-upper.example.test:443?security=TLS&sni=edge.example.test#VLESS%20Upper
TROJAN://secret@trojan-upper.example.test:443?sni=edge.example.test#Trojan%20Upper
SS://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-upper.example.test:8388#SS%20Upper
SSR://{ssr}
VMESS://{vmess}
HY2://hy-pass@hy2-upper.example.test:443?sni=hy2-sni.example.test#HY2%20Upper
TUIC://00000000-0000-0000-0000-000000000008:tuic-pass@tuic-upper.example.test:443?sni=tuic-sni.example.test#TUIC%20Upper
HTTPS://user:pass@https-upper.example.test:8443?allow_insecure=true#HTTPS%20Upper
SOCKS5://user:pass@socks-upper.example.test:1080?tls=true#SOCKS%20Upper
"
        );
    let yaml = normalize_profile_content(&links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let find = |name: &str| {
        proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
            .unwrap()
    };

    assert_eq!(
        get_string(find("VLESS Upper"), "type").as_deref(),
        Some("vless")
    );
    assert_eq!(
        get_string(find("Trojan Upper"), "type").as_deref(),
        Some("trojan")
    );
    assert_eq!(get_string(find("SS Upper"), "type").as_deref(), Some("ss"));
    assert_eq!(
        get_string(find("SSR Upper"), "type").as_deref(),
        Some("ssr")
    );
    assert_eq!(
        get_string(find("VMess Upper"), "type").as_deref(),
        Some("vmess")
    );
    assert_eq!(
        get_string(find("VMess Upper"), "server").as_deref(),
        Some("vmess-upper.example.test")
    );
    assert_eq!(get_u16(find("VMess Upper"), "port"), Some(443));
    assert_eq!(get_bool(find("VMess Upper"), "tls"), Some(true));
    assert_eq!(
        get_string(find("VMess Upper"), "servername").as_deref(),
        Some("vmess-sni.example.test")
    );
    assert_eq!(
        get_string_list(find("VMess Upper"), "alpn"),
        vec!["h2", "http/1.1"]
    );
    assert_eq!(get_bool(find("VMess Upper"), "udp"), Some(true));
    assert_eq!(get_bool(find("VMess Upper"), "tfo"), Some(true));
    assert_eq!(
        get_string(find("HY2 Upper"), "type").as_deref(),
        Some("hysteria2")
    );
    assert_eq!(
        get_string(find("TUIC Upper"), "type").as_deref(),
        Some("tuic")
    );
    assert_eq!(
        get_string(find("HTTPS Upper"), "type").as_deref(),
        Some("http")
    );
    assert_eq!(
        get_string(find("SOCKS Upper"), "type").as_deref(),
        Some("socks5")
    );
    assert_eq!(get_bool(find("HTTPS Upper"), "tls"), Some(true));
    assert_eq!(get_bool(find("SOCKS Upper"), "tls"), Some(true));
}

#[test]
fn share_link_query_parameters_are_case_insensitive() {
    let links = "\
vless://00000000-0000-0000-0000-000000000001@case.example.test:443?TYPE=WS&SECURITY=Reality&SNI=edge.example.test&WsHost=cdn.example.test&WsPath=%2Fws&FP=chrome&ALPN=h2%2Chttp%2F1.1&PBK=pub&SID=abcd&SPX=%2Fspider&ED=2048&EH=Sec-WebSocket-Protocol&ALLOWINSECURE=TRUE#VLESS%20Case
trojan://secret@trojan-case.example.test:443?TYPE=GRPC&ServiceName=svc&MODE=gun&SERVERNAME=trojan-sni.example.test&Allow-Insecure=Allow#Trojan%20Case
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-case.example.test:8388?PLUGIN=v2ray-plugin&PLUGINOPTS=mode%3Dwebsocket%3Bhost%3Dedge.example.test%3Bpath%3D%2Fcase#SS%20Case
https://user:pass@http-case.example.test:8443?ALLOW_INSECURE=1#HTTPS%20Case
socks5://user:pass@socks-case.example.test:1080?TLS=TRUE&SKIP-CERT-VERIFY=TRUE#SOCKS%20Case
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let find = |name: &str| {
        proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
            .unwrap()
    };

    let vless = find("VLESS Case");
    assert_eq!(get_bool(vless, "tls"), Some(true));
    assert_eq!(get_bool(vless, "skip-cert-verify"), Some(true));
    assert_eq!(get_string(vless, "network").as_deref(), Some("ws"));
    assert_eq!(
        get_string(vless, "client-fingerprint").as_deref(),
        Some("chrome")
    );
    assert_eq!(get_string_list(vless, "alpn"), vec!["h2", "http/1.1"]);
    let ws_opts = vless
        .get(&value_key("ws-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(get_string(ws_opts, "path").as_deref(), Some("/ws"));
    assert_eq!(get_i64(ws_opts, "max-early-data"), Some(2048));
    assert_eq!(
        get_string(ws_opts, "early-data-header-name").as_deref(),
        Some("Sec-WebSocket-Protocol")
    );
    assert_eq!(
        get_string(
            ws_opts
                .get(&value_key("headers"))
                .and_then(Value::as_mapping)
                .unwrap(),
            "Host"
        )
        .as_deref(),
        Some("cdn.example.test")
    );
    let reality_opts = vless
        .get(&value_key("reality-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(reality_opts, "public-key").as_deref(),
        Some("pub")
    );
    assert_eq!(
        get_string(reality_opts, "short-id").as_deref(),
        Some("abcd")
    );
    assert_eq!(
        get_string(reality_opts, "spider-x").as_deref(),
        Some("/spider")
    );

    let trojan = find("Trojan Case");
    assert_eq!(get_bool(trojan, "skip-cert-verify"), Some(true));
    assert_eq!(
        get_string(trojan, "sni").as_deref(),
        Some("trojan-sni.example.test")
    );
    let grpc_opts = trojan
        .get(&value_key("grpc-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(grpc_opts, "grpc-service-name").as_deref(),
        Some("svc")
    );
    assert_eq!(get_string(grpc_opts, "grpc-mode").as_deref(), Some("gun"));

    let ss = find("SS Case");
    assert_eq!(get_string(ss, "plugin").as_deref(), Some("v2ray-plugin"));
    assert_eq!(
        get_string(ss, "plugin-opts").as_deref(),
        Some("mode=websocket;host=edge.example.test;path=/case")
    );
    assert_eq!(get_bool(find("HTTPS Case"), "skip-cert-verify"), Some(true));
    assert_eq!(get_bool(find("SOCKS Case"), "tls"), Some(true));
    assert_eq!(get_bool(find("SOCKS Case"), "skip-cert-verify"), Some(true));
}

#[test]
fn vless_tls_query_aliases_are_normalized() {
    let links = "\
vless://00000000-0000-0000-0000-000000000001@tls-query.example.test:443?tls=true&sni=tls-query.example.test#VLESS%20TLS%20Query
vless://00000000-0000-0000-0000-000000000002@enable-tls.example.test:443?enable-tls=1&sni=enable-tls.example.test#VLESS%20Enable%20TLS
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let find = |name: &str| {
        proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
            .unwrap()
    };

    assert_eq!(get_bool(find("VLESS TLS Query"), "tls"), Some(true));
    assert_eq!(
        get_string(find("VLESS TLS Query"), "servername").as_deref(),
        Some("tls-query.example.test")
    );
    assert_eq!(get_bool(find("VLESS Enable TLS"), "tls"), Some(true));
    assert_eq!(
        get_string(find("VLESS Enable TLS"), "servername").as_deref(),
        Some("enable-tls.example.test")
    );
}

#[test]
fn share_link_query_names_are_used_when_fragment_is_missing() {
    let links = "\
vless://00000000-0000-0000-0000-000000000001@name-vless.example.test:443?security=tls&remarks=VLESS%20Query
trojan://secret@name-trojan.example.test:443?name=Trojan%20Query
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@name-ss.example.test:8388?ps=SS%20Query
hysteria2://hy-pass@name-hy2.example.test:443?alias=HY2%20Query
tuic://00000000-0000-0000-0000-000000000008:tuic-pass@name-tuic.example.test:443?node-name=TUIC%20Query
http://user:pass@name-http.example.test:8080?nodeName=HTTP%20Query
socks5://user:pass@name-socks.example.test:1080?remark=SOCKS%20Query#SOCKS%20Fragment
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let names = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .filter_map(|proxy| get_string(proxy, "name"))
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "VLESS Query",
            "Trojan Query",
            "SS Query",
            "HY2 Query",
            "TUIC Query",
            "HTTP Query",
            "SOCKS Fragment"
        ]
    );
}

#[test]
fn vmess_json_field_aliases_are_normalized() {
    let vmess = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            r#"{"name":"VMess Query Alias","server":"vmess-alias.example.test","port":"443","uuid":"00000000-0000-0000-0000-000000000011","alter_id":"0","security":"tls","network":"ws","wsHost":"alias-cdn.example.test","wsPath":"/alias","serverName":"alias-sni.example.test","allow_insecure":"true"}"#,
        );
    let yaml = normalize_profile_content(&format!("vmess://{vmess}")).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let vmess = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess Query Alias"))
        .unwrap();

    assert_eq!(get_string(vmess, "type").as_deref(), Some("vmess"));
    assert_eq!(
        get_string(vmess, "server").as_deref(),
        Some("vmess-alias.example.test")
    );
    assert_eq!(get_u16(vmess, "port"), Some(443));
    assert_eq!(
        get_string(vmess, "uuid").as_deref(),
        Some("00000000-0000-0000-0000-000000000011")
    );
    assert_eq!(get_i64(vmess, "alterId"), Some(0));
    assert_eq!(get_bool(vmess, "tls"), Some(true));
    assert!(get_string(vmess, "cipher").is_none());
    assert_eq!(get_bool(vmess, "skip-cert-verify"), Some(true));
    assert_eq!(get_string(vmess, "network").as_deref(), Some("ws"));
    assert_eq!(
        get_string(vmess, "servername").as_deref(),
        Some("alias-sni.example.test")
    );
    let ws_opts = vmess
        .get(&value_key("ws-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(get_string(ws_opts, "path").as_deref(), Some("/alias"));
    assert_eq!(
        get_string(
            ws_opts
                .get(&value_key("headers"))
                .and_then(Value::as_mapping)
                .unwrap(),
            "Host"
        )
        .as_deref(),
        Some("alias-cdn.example.test")
    );
}

#[test]
fn hysteria_family_and_tuic_share_links_are_normalized_to_clash_yaml() {
    let links = "\
hysteria://hy1-auth@hy1.example.test:443?protocol=udp&peer=hy1-sni.example.test&insecure=1&alpn=h3&obfs=obfs-pass&upmbps=20&downmbps=80&mport=10000-10010&recv-window-conn=1048576&recv-window=2097152&disable-mtu-discovery=true&fast-open=true#HY1%20Alias
hysteria2://hy-pass@hy2.example.test:443?sni=hy2-sni.example.test&insecure=1&alpn=h3,h2&obfs=salamander&obfs-password=obfs-pass&upmbps=50&downmbps=100&mport=20000-20010&recvWindowConn=3145728&recvWindow=4194304&disableMtuDiscovery=true&fastOpen=true#HY2%20Alias
tuic://00000000-0000-0000-0000-000000000008:tuic-pass@tuic.example.test:443?sni=tuic-sni.example.test&allow_insecure=true&alpn=h3&congestion_control=bbr&udp_relay_mode=native&disableSni=true&reduceRtt=true&requestTimeout=8s&heartbeatInterval=10s&maxUdpRelayPacketSize=1500&fastOpen=true#TUIC%20Alias
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();

    let hy1 = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY1 Alias"))
        .unwrap();
    assert_eq!(get_string(hy1, "type").as_deref(), Some("hysteria"));
    assert_eq!(
        get_string(hy1, "server").as_deref(),
        Some("hy1.example.test")
    );
    assert_eq!(get_u16(hy1, "port"), Some(443));
    assert_eq!(get_string(hy1, "auth-str").as_deref(), Some("hy1-auth"));
    assert_eq!(get_string(hy1, "protocol").as_deref(), Some("udp"));
    assert_eq!(
        get_string(hy1, "sni").as_deref(),
        Some("hy1-sni.example.test")
    );
    assert_eq!(get_bool(hy1, "skip-cert-verify"), Some(true));
    assert_eq!(get_string_list(hy1, "alpn"), vec!["h3"]);
    assert_eq!(get_string(hy1, "obfs").as_deref(), Some("obfs-pass"));
    assert_eq!(get_string(hy1, "up").as_deref(), Some("20"));
    assert_eq!(get_string(hy1, "down").as_deref(), Some("80"));
    assert_eq!(get_string(hy1, "ports").as_deref(), Some("10000-10010"));
    assert_eq!(
        get_string(hy1, "recv-window-conn").as_deref(),
        Some("1048576")
    );
    assert_eq!(get_string(hy1, "recv-window").as_deref(), Some("2097152"));
    assert_eq!(get_bool(hy1, "disable-mtu-discovery"), Some(true));
    assert_eq!(get_bool(hy1, "fast-open"), Some(true));

    let hy2 = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY2 Alias"))
        .unwrap();
    assert_eq!(get_string(hy2, "type").as_deref(), Some("hysteria2"));
    assert_eq!(
        get_string(hy2, "server").as_deref(),
        Some("hy2.example.test")
    );
    assert_eq!(get_u16(hy2, "port"), Some(443));
    assert_eq!(get_string(hy2, "password").as_deref(), Some("hy-pass"));
    assert_eq!(
        get_string(hy2, "sni").as_deref(),
        Some("hy2-sni.example.test")
    );
    assert_eq!(get_bool(hy2, "skip-cert-verify"), Some(true));
    assert_eq!(get_string_list(hy2, "alpn"), vec!["h3", "h2"]);
    assert_eq!(get_string(hy2, "obfs").as_deref(), Some("salamander"));
    assert_eq!(
        get_string(hy2, "obfs-password").as_deref(),
        Some("obfs-pass")
    );
    assert_eq!(get_string(hy2, "up").as_deref(), Some("50"));
    assert_eq!(get_string(hy2, "down").as_deref(), Some("100"));
    assert_eq!(get_string(hy2, "ports").as_deref(), Some("20000-20010"));
    assert_eq!(
        get_string(hy2, "recv-window-conn").as_deref(),
        Some("3145728")
    );
    assert_eq!(get_string(hy2, "recv-window").as_deref(), Some("4194304"));
    assert_eq!(get_bool(hy2, "disable-mtu-discovery"), Some(true));
    assert_eq!(get_bool(hy2, "fast-open"), Some(true));

    let tuic = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("TUIC Alias"))
        .unwrap();
    assert_eq!(get_string(tuic, "type").as_deref(), Some("tuic"));
    assert_eq!(
        get_string(tuic, "uuid").as_deref(),
        Some("00000000-0000-0000-0000-000000000008")
    );
    assert_eq!(get_string(tuic, "password").as_deref(), Some("tuic-pass"));
    assert_eq!(
        get_string(tuic, "sni").as_deref(),
        Some("tuic-sni.example.test")
    );
    assert_eq!(get_bool(tuic, "skip-cert-verify"), Some(true));
    assert_eq!(get_string_list(tuic, "alpn"), vec!["h3"]);
    assert_eq!(
        get_string(tuic, "congestion-controller").as_deref(),
        Some("bbr")
    );
    assert_eq!(
        get_string(tuic, "udp-relay-mode").as_deref(),
        Some("native")
    );
    assert_eq!(get_bool(tuic, "disable-sni"), Some(true));
    assert_eq!(get_bool(tuic, "reduce-rtt"), Some(true));
    assert_eq!(get_string(tuic, "request-timeout").as_deref(), Some("8s"));
    assert_eq!(
        get_string(tuic, "heartbeat-interval").as_deref(),
        Some("10s")
    );
    assert_eq!(
        get_string(tuic, "max-udp-relay-packet-size").as_deref(),
        Some("1500")
    );
    assert_eq!(get_bool(tuic, "fast-open"), Some(true));
}

#[test]
fn hysteria2_query_auth_aliases_are_normalized() {
    let link = "hysteria2://hy2-query.example.test:443?auth=query-pass&peer=query-sni.example.test&remarks=HY2%20Query%20Auth";
    let yaml = normalize_profile_content(link).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let hy2 = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY2 Query Auth"))
        .unwrap();

    assert_eq!(get_string(hy2, "type").as_deref(), Some("hysteria2"));
    assert_eq!(
        get_string(hy2, "server").as_deref(),
        Some("hy2-query.example.test")
    );
    assert_eq!(get_u16(hy2, "port"), Some(443));
    assert_eq!(get_string(hy2, "password").as_deref(), Some("query-pass"));
    assert_eq!(
        get_string(hy2, "sni").as_deref(),
        Some("query-sni.example.test")
    );
}

#[test]
fn share_link_transport_options_are_normalized_to_clash_yaml() {
    let vmess_h2 = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess H2","add":"vmess-h2.example.test","port":443,"id":"00000000-0000-0000-0000-000000000003","aid":0,"scy":"auto","net":"h2","host":"h2a.example.test,h2b.example.test","path":"/vmess-h2","tls":"tls","sni":"edge.example.test"}"#,
        );
    let vmess_httpupgrade = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess HTTPUpgrade","add":"vmess-up.example.test","port":"443","id":"00000000-0000-0000-0000-000000000004","aid":"0","scy":"auto","net":"httpupgrade","host":"upgrade.example.test","path":"/vmess-upgrade","tls":"tls","sni":"edge.example.test"}"#,
        );
    let vmess_ws_alias = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess WS Alias","add":"vmess-ws-alias.example.test","port":"443","id":"00000000-0000-0000-0000-000000000006","aid":"0","cipher":"auto","type":"ws","wsHost":"alias.example.test","wsPath":"/alias-ws","tls":"tls","serverName":"alias-sni.example.test","clientFingerprint":"chrome","allowInsecure":"allow"}"#,
        );
    let vmess_grpc_alias = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess GRPC Alias","add":"vmess-grpc-alias.example.test","port":"443","id":"00000000-0000-0000-0000-000000000007","aid":"0","scy":"auto","type":"grpc","grpc-service-name":"vmess-svc","grpc-mode":"gun","tls":"tls","sni":"grpc-sni.example.test","allow_insecure":true}"#,
        );
    let links = format!(
            "\
vless://00000000-0000-0000-0000-000000000001@example.test:443?type=ws&security=Reality&sni=edge.example.test&host=cdn.example.test&path=%2Fws&fp=chrome&alpn=h2%2Chttp%2F1.1&pbk=public-key&sid=abcd&spx=%2Ffingerprint&ed=2048&eh=Sec-WebSocket-Protocol&flow=xtls-rprx-vision&encryption=NONE#VLESS%20WS
trojan://secret@example.test:443?type=grpc&serviceName=svc&mode=gun&sni=edge.example.test&allowInsecure=1#Trojan%20GRPC
trojan://secret@example.test:443?type=grpc&grpc-service-name=alias-svc&grpc-mode=gun&serverName=trojan-alias.example.test&allow-insecure=allow#Trojan%20GRPC%20Alias
vless://00000000-0000-0000-0000-000000000002@example.test:443?type=h2&security=tls&sni=edge.example.test&host=h2a.example.test,h2b.example.test&path=%2Fh2#VLESS%20H2
vless://00000000-0000-0000-0000-000000000005@example.test:443?type=httpupgrade&security=tls&sni=edge.example.test&host=upgrade.example.test&path=%2Fupgrade#VLESS%20HTTPUpgrade
vless://00000000-0000-0000-0000-000000000006@example.test:443?network=ws&security=tls&serverName=alias-sni.example.test&wsHost=alias.example.test&wsPath=%2Falias-ws&client-fingerprint=chrome&allow-insecure=allow#VLESS%20WS%20Alias
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8388?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dcdn.example.test#SS%20OBFS
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8389?plugin=v2ray-plugin%3Bmode%3Dwebsocket%3Bhost%3Dedge.example.test%3Bpath%3D%2Fss-ws%3Btls#SS%20V2Ray
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8390?plugin=obfs-local&plugin-opts=obfs%3Dtls%3Bobfs-host%3Dexplicit.example.test#SS%20OBFS%20Explicit
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8391?plugin=v2ray-plugin&pluginOpts=mode%3Dwebsocket%3Bhost%3Dpluginopts.example.test%3Bpath%3D%2Fexplicit#SS%20V2Ray%20Explicit
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8392?plugin=Simple-Obfs%3Bobfs%3Dhttp%3Bobfs-host%3Dcase.example.test#SS%20OBFS%20Case
vmess://{vmess_h2}
vmess://{vmess_httpupgrade}
vmess://{vmess_ws_alias}
vmess://{vmess_grpc_alias}
"
        );
    let yaml = normalize_profile_content(&links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let vless = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS WS"))
        .unwrap();
    assert_eq!(get_bool(vless, "tls"), Some(true));
    assert_eq!(get_string(vless, "network").as_deref(), Some("ws"));
    assert_eq!(
        get_string(vless, "client-fingerprint").as_deref(),
        Some("chrome")
    );
    assert_eq!(
        get_string(vless, "flow").as_deref(),
        Some("xtls-rprx-vision")
    );
    assert_eq!(get_string(vless, "encryption").as_deref(), Some("none"));
    let reality_opts = vless
        .get(&value_key("reality-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(reality_opts, "public-key").as_deref(),
        Some("public-key")
    );
    assert_eq!(
        get_string(reality_opts, "short-id").as_deref(),
        Some("abcd")
    );
    assert_eq!(
        get_string(reality_opts, "spider-x").as_deref(),
        Some("/fingerprint")
    );
    let vless_ws_opts = vless
        .get(&value_key("ws-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(get_string(vless_ws_opts, "path").as_deref(), Some("/ws"));
    assert_eq!(get_i64(vless_ws_opts, "max-early-data"), Some(2048));
    assert_eq!(
        get_string(vless_ws_opts, "early-data-header-name").as_deref(),
        Some("Sec-WebSocket-Protocol")
    );
    assert_eq!(get_string_list(vless, "alpn"), vec!["h2", "http/1.1"]);

    let trojan = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("Trojan GRPC"))
        .unwrap();
    let grpc_opts = trojan
        .get(&value_key("grpc-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(grpc_opts, "grpc-service-name").as_deref(),
        Some("svc")
    );
    assert_eq!(get_string(grpc_opts, "grpc-mode").as_deref(), Some("gun"));
    let trojan_alias = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("Trojan GRPC Alias"))
        .unwrap();
    assert_eq!(
        get_string(trojan_alias, "sni").as_deref(),
        Some("trojan-alias.example.test")
    );
    assert_eq!(get_bool(trojan_alias, "skip-cert-verify"), Some(true));
    let grpc_alias_opts = trojan_alias
        .get(&value_key("grpc-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(grpc_alias_opts, "grpc-service-name").as_deref(),
        Some("alias-svc")
    );
    assert_eq!(
        get_string(grpc_alias_opts, "grpc-mode").as_deref(),
        Some("gun")
    );

    let vless_h2 = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS H2"))
        .unwrap();
    assert_eq!(get_string(vless_h2, "network").as_deref(), Some("h2"));
    let h2_opts = vless_h2
        .get(&value_key("h2-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(get_string(h2_opts, "path").as_deref(), Some("/h2"));
    assert_eq!(
        get_string_list(h2_opts, "host"),
        vec!["h2a.example.test", "h2b.example.test"]
    );

    let vless_httpupgrade = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS HTTPUpgrade"))
        .unwrap();
    assert_eq!(
        get_string(vless_httpupgrade, "network").as_deref(),
        Some("httpupgrade")
    );
    let http_upgrade_opts = vless_httpupgrade
        .get(&value_key("http-upgrade-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(http_upgrade_opts, "path").as_deref(),
        Some("/upgrade")
    );
    assert_eq!(
        get_string(http_upgrade_opts, "host").as_deref(),
        Some("upgrade.example.test")
    );
    let vless_ws_alias = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS WS Alias"))
        .unwrap();
    assert_eq!(
        get_string(vless_ws_alias, "servername").as_deref(),
        Some("alias-sni.example.test")
    );
    assert_eq!(
        get_string(vless_ws_alias, "client-fingerprint").as_deref(),
        Some("chrome")
    );
    assert_eq!(get_bool(vless_ws_alias, "skip-cert-verify"), Some(true));
    let vless_ws_alias_opts = vless_ws_alias
        .get(&value_key("ws-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(vless_ws_alias_opts, "path").as_deref(),
        Some("/alias-ws")
    );
    assert_eq!(
        get_string(
            vless_ws_alias_opts
                .get(&value_key("headers"))
                .and_then(Value::as_mapping)
                .unwrap(),
            "Host"
        )
        .as_deref(),
        Some("alias.example.test")
    );

    let vmess_h2 = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess H2"))
        .unwrap();
    assert_eq!(get_string(vmess_h2, "network").as_deref(), Some("h2"));
    let vmess_h2_opts = vmess_h2
        .get(&value_key("h2-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(vmess_h2_opts, "path").as_deref(),
        Some("/vmess-h2")
    );
    assert_eq!(
        get_string_list(vmess_h2_opts, "host"),
        vec!["h2a.example.test", "h2b.example.test"]
    );

    let vmess_httpupgrade = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess HTTPUpgrade"))
        .unwrap();
    assert_eq!(
        get_string(vmess_httpupgrade, "network").as_deref(),
        Some("httpupgrade")
    );
    let vmess_http_upgrade_opts = vmess_httpupgrade
        .get(&value_key("http-upgrade-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(vmess_http_upgrade_opts, "path").as_deref(),
        Some("/vmess-upgrade")
    );
    assert_eq!(
        get_string(vmess_http_upgrade_opts, "host").as_deref(),
        Some("upgrade.example.test")
    );
    let vmess_ws_alias = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess WS Alias"))
        .unwrap();
    assert_eq!(get_string(vmess_ws_alias, "network").as_deref(), Some("ws"));
    assert_eq!(
        get_string(vmess_ws_alias, "servername").as_deref(),
        Some("alias-sni.example.test")
    );
    assert_eq!(
        get_string(vmess_ws_alias, "client-fingerprint").as_deref(),
        Some("chrome")
    );
    assert_eq!(get_bool(vmess_ws_alias, "skip-cert-verify"), Some(true));
    let vmess_ws_alias_opts = vmess_ws_alias
        .get(&value_key("ws-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(vmess_ws_alias_opts, "path").as_deref(),
        Some("/alias-ws")
    );
    assert_eq!(
        get_string(
            vmess_ws_alias_opts
                .get(&value_key("headers"))
                .and_then(Value::as_mapping)
                .unwrap(),
            "Host"
        )
        .as_deref(),
        Some("alias.example.test")
    );
    let vmess_grpc_alias = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess GRPC Alias"))
        .unwrap();
    assert_eq!(
        get_string(vmess_grpc_alias, "network").as_deref(),
        Some("grpc")
    );
    assert_eq!(
        get_string(vmess_grpc_alias, "servername").as_deref(),
        Some("grpc-sni.example.test")
    );
    assert_eq!(get_bool(vmess_grpc_alias, "skip-cert-verify"), Some(true));
    let vmess_grpc_alias_opts = vmess_grpc_alias
        .get(&value_key("grpc-opts"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(
        get_string(vmess_grpc_alias_opts, "grpc-service-name").as_deref(),
        Some("vmess-svc")
    );
    assert_eq!(
        get_string(vmess_grpc_alias_opts, "grpc-mode").as_deref(),
        Some("gun")
    );

    let ss_obfs = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS"))
        .unwrap();
    assert_eq!(get_string(ss_obfs, "plugin").as_deref(), Some("obfs"));
    assert_eq!(
        get_string(ss_obfs, "plugin-opts").as_deref(),
        Some("obfs=http;obfs-host=cdn.example.test")
    );

    let ss_v2ray = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS V2Ray"))
        .unwrap();
    assert_eq!(
        get_string(ss_v2ray, "plugin").as_deref(),
        Some("v2ray-plugin")
    );
    assert_eq!(
        get_string(ss_v2ray, "plugin-opts").as_deref(),
        Some("mode=websocket;host=edge.example.test;path=/ss-ws;tls")
    );

    let ss_obfs_explicit = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS Explicit"))
        .unwrap();
    assert_eq!(
        get_string(ss_obfs_explicit, "plugin").as_deref(),
        Some("obfs")
    );
    assert_eq!(
        get_string(ss_obfs_explicit, "plugin-opts").as_deref(),
        Some("obfs=tls;obfs-host=explicit.example.test")
    );

    let ss_obfs_case = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS Case"))
        .unwrap();
    assert_eq!(get_string(ss_obfs_case, "plugin").as_deref(), Some("obfs"));
    assert_eq!(
        get_string(ss_obfs_case, "plugin-opts").as_deref(),
        Some("obfs=http;obfs-host=case.example.test")
    );

    let ss_v2ray_explicit = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS V2Ray Explicit"))
        .unwrap();
    assert_eq!(
        get_string(ss_v2ray_explicit, "plugin").as_deref(),
        Some("v2ray-plugin")
    );
    assert_eq!(
        get_string(ss_v2ray_explicit, "plugin-opts").as_deref(),
        Some("mode=websocket;host=pluginopts.example.test;path=/explicit")
    );
}

#[test]
fn http_and_socks5_share_links_are_normalized_to_clash_yaml() {
    let links = "\
http://user:pass@example.test:8080?allow_insecure=true&headers=User-Agent%3DHMeta%3BProxy-Authorization%3DBearer%20token#HTTP%20Proxy
socks5://sock%20user:sock%20pass@example.test:1080?tls=true&skip-cert-verify=true#SOCKS5%20Proxy
";
    let yaml = normalize_profile_content(links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();

    let http = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("HTTP Proxy"))
        .unwrap();
    assert_eq!(get_string(http, "type").as_deref(), Some("http"));
    assert_eq!(get_string(http, "username").as_deref(), Some("user"));
    assert_eq!(get_string(http, "password").as_deref(), Some("pass"));
    assert_eq!(get_bool(http, "skip-cert-verify"), Some(true));
    let headers = http
        .get(&value_key("headers"))
        .and_then(Value::as_mapping)
        .unwrap();
    assert_eq!(get_string(headers, "User-Agent").as_deref(), Some("HMeta"));
    assert_eq!(
        get_string(headers, "Proxy-Authorization").as_deref(),
        Some("Bearer token")
    );

    let socks = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .find(|proxy| get_string(proxy, "name").as_deref() == Some("SOCKS5 Proxy"))
        .unwrap();
    assert_eq!(get_string(socks, "type").as_deref(), Some("socks5"));
    assert_eq!(get_string(socks, "username").as_deref(), Some("sock user"));
    assert_eq!(get_string(socks, "password").as_deref(), Some("sock pass"));
    assert!(matches!(
        socks.get(&value_key("tls")),
        Some(Value::Bool(true))
    ));
    assert!(matches!(
        socks.get(&value_key("skip-cert-verify")),
        Some(Value::Bool(true))
    ));
}

#[test]
fn share_links_preserve_udp_and_tfo_options() {
    let vmess_tfo = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess TFO","add":"vmess-tfo.example.test","port":443,"id":"00000000-0000-0000-0000-000000000010","aid":0,"scy":"auto","net":"tcp","udp":true,"fastOpen":true}"#,
        );
    let links = format!(
            "\
vless://00000000-0000-0000-0000-000000000001@vless-tfo.example.test:443?security=none&tfo=1#VLESS%20TFO
trojan://secret@trojan-tfo.example.test:443?fast-open=true#Trojan%20TFO
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-tfo.example.test:8388?TFO=true#SS%20TFO
socks5://sock:sock-pass@socks-tfo.example.test:1080?udp=true&fastOpen=true#SOCKS5%20UDP%20TFO
vmess://{vmess_tfo}
"
        );
    let yaml = normalize_profile_content(&links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let find = |name: &str| {
        proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
            .unwrap()
    };

    for name in ["VLESS TFO", "Trojan TFO", "SS TFO", "VMess TFO"] {
        let proxy = find(name);
        assert_eq!(get_bool(proxy, "udp"), Some(true));
        assert_eq!(get_bool(proxy, "tfo"), Some(true));
    }

    let socks = find("SOCKS5 UDP TFO");
    assert_eq!(get_string(socks, "type").as_deref(), Some("socks5"));
    assert_eq!(get_bool(socks, "udp"), Some(true));
    assert_eq!(get_bool(socks, "tfo"), Some(true));
}

#[test]
fn share_links_preserve_explicit_udp_disabled_options() {
    let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let ssr_body = format!(
        "ssr-udp-off.example.test:8388:origin:aes-256-cfb:plain:{}/?remarks={}&udp=false",
        b64("ssr-pass"),
        b64("SSR UDP Off")
    );
    let ss_cipher = base64::engine::general_purpose::STANDARD.encode("aes-256-gcm:password");
    let links = format!(
        "\
vless://00000000-0000-0000-0000-000000000001@vless-udp-off.example.test:443?udp=0#VLESS%20UDP%20Off
trojan://secret@trojan-udp-off.example.test:443?udp=false#Trojan%20UDP%20Off
ss://{ss_cipher}@ss-udp-off.example.test:8388?udp=off#SS%20UDP%20Off
ssr://{}
",
        b64(&ssr_body)
    );
    let yaml = normalize_profile_content(&links).unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(&value_key("proxies"))
        .and_then(Value::as_sequence)
        .unwrap();
    let find = |name: &str| {
        proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
            .unwrap()
    };

    for name in [
        "VLESS UDP Off",
        "Trojan UDP Off",
        "SS UDP Off",
        "SSR UDP Off",
    ] {
        assert_eq!(get_bool(find(name), "udp"), Some(false));
    }
}

#[test]
fn selected_proxy_choices_persist_with_profile() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("select")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Remote Demo",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();
    store
        .set_selected_proxy(&profile_id, "Proxy", "DIRECT")
        .unwrap();
    drop(store);

    let store = ProfileStore::open(root).unwrap();
    let selections = store.selected_proxies(&profile_id).unwrap();
    assert_eq!(selections.get("Proxy").map(String::as_str), Some("DIRECT"));
    assert_eq!(
        store.summaries()[0]
            .selected_proxies
            .get("Proxy")
            .map(String::as_str),
        Some("DIRECT")
    );
}

#[test]
fn profile_content_can_restore_original_backup_after_edit() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("backup")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Editable",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
            None,
        )
        .unwrap();

    store
        .update_profile_content(
            &profile_id,
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,REJECT\n",
        )
        .unwrap();
    assert!(store
        .raw_yaml(&profile_id)
        .unwrap()
        .contains("MATCH,REJECT"));

    store.restore_profile_backup(&profile_id).unwrap();
    let restored = store.raw_yaml(&profile_id).unwrap();
    assert!(restored.contains("MATCH,DIRECT"));
    assert!(!restored.contains("MATCH,REJECT"));
}

#[test]
fn profile_traffic_is_accumulated_in_summary() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("traffic")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Traffic",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();
    store.add_profile_traffic(&profile_id, 10, 20).unwrap();
    store.add_profile_traffic(&profile_id, 5, 7).unwrap();

    let summary = store
        .summaries()
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .unwrap();
    assert_eq!(summary.upload_bytes, 15);
    assert_eq!(summary.download_bytes, 27);
}

#[test]
fn delete_profile_removes_runtime_and_profile_scoped_provider_cache() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("delete")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Providers",
            "local",
            r#"
mixed-port: 7890
proxy-providers:
  remote:
    type: http
    url: https://example.test/proxies.yaml
    interval: 3600
    path: ignored.yaml
    proxy: DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - remote
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
            None,
        )
        .unwrap();
    store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();

    let runtime_path = root.join("runtime").join(format!("{profile_id}.yaml"));
    let provider_dir = root.join("providers/proxy").join(&profile_id);
    let provider_path = provider_dir.join("remote.yaml");
    let runtime_yaml = std::fs::read_to_string(&runtime_path).unwrap();
    let provider = store
        .providers_from_yaml(&runtime_yaml)
        .into_iter()
        .find(|provider| provider.name == "remote")
        .expect("remote provider");
    assert_eq!(
        provider.path.as_deref(),
        Some(provider_path.to_string_lossy().as_ref())
    );
    assert!(!provider.cache_exists);
    assert!(provider.cache_bytes.is_none());

    std::fs::write(&provider_path, "proxies: []\n").unwrap();
    let provider = store
        .providers_from_yaml(&runtime_yaml)
        .into_iter()
        .find(|provider| provider.name == "remote")
        .expect("remote provider");
    assert!(provider.cache_exists);
    assert_eq!(provider.cache_bytes, Some("proxies: []\n".len() as u64));
    assert!(provider.cache_updated_at.is_some());
    assert!(runtime_path.exists());
    assert!(provider_dir.exists());

    store.delete_profile(&profile_id).unwrap();
    assert!(!runtime_path.exists());
    assert!(!provider_dir.exists());
}

#[test]
fn provider_cache_paths_are_profile_scoped_and_sanitized() {
    let root =
        std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("provider-path")));
    let mut store = ProfileStore::open(root.clone()).unwrap();
    let profile_id = store
        .import_profile_content(
            "Providers",
            "local",
            r#"
mixed-port: 7890
proxy-providers:
  "../escape":
    type: http
    url: https://example.test/proxies.yaml
    interval: 3600
    filter: "HK|香港"
    exclude-filter: Premium
    path: ../../escape.yaml
    proxy: DIRECT
    health-check:
      enable: true
      url: https://cp.cloudflare.com/generate_204
      interval: "600"
rule-providers:
  remote-rules:
    type: http
    behavior: domain
    format: mrs
    url: https://example.test/rules.mrs
    interval: "7200"
    path: ../../rules.mrs
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - "../escape"
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
            None,
        )
        .unwrap();

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let provider = store
        .providers_from_yaml(&yaml)
        .into_iter()
        .find(|provider| provider.name == "../escape")
        .expect("escaped provider");
    let provider_path = provider.path.as_deref().expect("provider path");
    let expected_dir = root.join("providers/proxy").join(&profile_id);
    let provider_path = Path::new(provider_path);
    assert!(provider_path.starts_with(&expected_dir));
    assert_eq!(provider_path.parent(), Some(expected_dir.as_path()));
    assert!(!provider_path.to_string_lossy().contains("../escape.yaml"));
    assert_eq!(provider.interval_seconds, Some(3600));
    assert_eq!(provider.filter.as_deref(), Some("HK|香港"));
    assert_eq!(provider.exclude_filter.as_deref(), Some("Premium"));
    assert!(provider.health_check_enabled);
    assert_eq!(
        provider.health_check_url.as_deref(),
        Some("https://cp.cloudflare.com/generate_204")
    );
    assert_eq!(provider.health_check_interval_seconds, Some(600));

    let rule_provider = store
        .providers_from_yaml(&yaml)
        .into_iter()
        .find(|provider| provider.name == "remote-rules")
        .expect("rule provider");
    let rule_provider_path = rule_provider.path.as_deref().expect("rule provider path");
    let expected_rule_dir = root.join("providers/rule").join(&profile_id);
    let rule_provider_path = Path::new(rule_provider_path);
    assert!(rule_provider_path.starts_with(&expected_rule_dir));
    assert_eq!(
        rule_provider_path.parent(),
        Some(expected_rule_dir.as_path())
    );
    assert_eq!(rule_provider.provider_type, "rule");
    assert_eq!(rule_provider.behavior.as_deref(), Some("domain"));
    assert_eq!(rule_provider.format.as_deref(), Some("mrs"));
    assert_eq!(rule_provider.interval_seconds, Some(7200));
}

#[test]
fn inline_rule_provider_keeps_payload_without_runtime_cache_fields() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("inline-rule-provider")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Inline Rules",
            "local",
            r#"
mixed-port: 7890
rule-providers:
  InlineRules:
    type: inline
    behavior: classical
    interval: 3600
    path: ../../inline.yaml
    payload:
      - DOMAIN-SUFFIX,inline.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - RULE-SET,InlineRules,DIRECT
  - MATCH,DIRECT
"#,
            None,
        )
        .unwrap();

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let provider = root
        .get(&value_key("rule-providers"))
        .and_then(Value::as_mapping)
        .and_then(|providers| providers.get(&value_key("InlineRules")))
        .and_then(Value::as_mapping)
        .expect("inline rule provider");

    assert_eq!(get_string(provider, "type").as_deref(), Some("inline"));
    assert!(provider.get(&value_key("path")).is_none());
    assert!(provider.get(&value_key("interval")).is_none());
    assert_eq!(
        get_string(provider, "behavior").as_deref(),
        Some("classical")
    );
    assert_eq!(
        provider
            .get(&value_key("payload"))
            .and_then(Value::as_sequence)
            .and_then(|payload| payload.first())
            .and_then(Value::as_str),
        Some("DOMAIN-SUFFIX,inline.example,DIRECT")
    );
}

#[test]
fn geodata_files_report_app_private_resource_state() {
    let root =
        std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("geodata-state")));
    let store = ProfileStore::open(root.clone()).unwrap();
    let files = store.geodata_files();
    assert_eq!(files.len(), 3);
    assert!(files.iter().any(|file| file.path.ends_with("Country.mmdb")));
    assert!(files.iter().any(|file| file.path.ends_with("geosite.dat")));
    assert!(files.iter().all(|file| !file.exists));

    std::fs::write(root.join("geodata").join("geosite.dat"), b"dat").unwrap();
    let files = store.geodata_files();
    let geosite = files
        .iter()
        .find(|file| file.path.ends_with("geosite.dat"))
        .expect("geosite summary");
    assert!(geosite.exists);
    assert_eq!(geosite.bytes, Some(3));
    assert!(geosite.updated_at.is_some());
}

#[test]
fn derives_vpn_options_from_profile_yaml() {
    let options = vpn_options_from_yaml(
        r#"
ipv6: true
dns:
  nameserver:
    - 9.9.9.9
  fallback:
    - https://dns.google/dns-query
  nameserver-policy:
    geosite:cn:
      - https://dns.alidns.com/dns-query
tun:
  mtu: 1400
  stack: lwip
  inet4-address:
    - 198.18.0.1/16
  route-address:
    - 10.0.0.0/8
hmeta:
  system-proxy: true
  allow-bypass: true
"#,
    )
    .unwrap();
    assert_eq!(options.mtu, 1400);
    assert_eq!(options.stack, "lwip");
    assert_eq!(options.dns_servers, vec!["9.9.9.9"]);
    assert_eq!(options.dns_fallbacks, vec!["https://dns.google/dns-query"]);
    assert_eq!(
        options
            .dns_nameserver_policy
            .get("geosite:cn")
            .cloned()
            .unwrap_or_default(),
        vec!["https://dns.alidns.com/dns-query"]
    );
    assert_eq!(options.dns_addresses, vec![MEOW_V4_ROUTER]);
    assert!(options.addresses.contains(&"198.18.0.1/16".to_owned()));
    assert!(options.addresses.contains(&MEOW_V6_CLIENT.to_owned()));
    assert_eq!(
        options.routes,
        vec!["10.0.0.0/8".to_owned(), "::/0".to_owned()]
    );
    assert!(options.system_proxy);
    assert!(options.allow_bypass);
}

#[test]
fn legacy_per_app_yaml_does_not_enter_vpn_options() {
    let options = vpn_options_from_yaml(
        r#"
hmeta:
  per-app-mode: bypass
  trusted-applications:
    - com.example.browser
  blocked-applications:
    - com.example.video
"#,
    )
    .unwrap();
    let json = serde_json::to_string(&options).unwrap();
    assert!(!json.contains("perAppMode"));
    assert!(!json.contains("trustedApplications"));
    assert!(!json.contains("blockedApplications"));
}

#[test]
fn runtime_yaml_uses_default_china_dns_split_policy() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-defaults")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS Defaults",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();

    let options = store.vpn_options_for_profile(&profile_id).unwrap();
    assert_eq!(
        options.dns_servers,
        vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
    );
    assert_eq!(
        options.dns_fallbacks,
        vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
    );
    assert_eq!(
        options
            .dns_nameserver_policy
            .get("geosite:cn")
            .cloned()
            .unwrap_or_default(),
        vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
    );
    assert_eq!(
        options
            .dns_nameserver_policy
            .get("geosite:geolocation-!cn")
            .cloned()
            .unwrap_or_default(),
        vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
    );

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let dns = value
        .as_mapping()
        .and_then(|root| root.get(&value_key("dns")))
        .and_then(Value::as_mapping)
        .expect("dns");
    assert_eq!(
        get_string_list(dns, "default-nameserver"),
        vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
    );
    assert_eq!(
        get_string_list(dns, "fallback"),
        vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
    );
    let policy = get_string_list_map(dns, "nameserver-policy");
    assert!(policy.contains_key("geosite:cn"));
    assert!(policy.contains_key("geosite:geolocation-!cn"));
}

#[test]
fn updates_vpn_config_in_profile_yaml() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("vpn")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "VPN",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();

    store
        .set_profile_vpn_config(&profile_id, true, false, true, " lwip ".to_owned())
        .unwrap();

    let raw_yaml = store.raw_yaml(&profile_id).unwrap();
    let value: Value = serde_yaml::from_str(&raw_yaml).unwrap();
    let root = value.as_mapping().expect("root");
    let hmeta = root
        .get(&value_key("hmeta"))
        .and_then(Value::as_mapping)
        .expect("hmeta");
    let tun = root
        .get(&value_key("tun"))
        .and_then(Value::as_mapping)
        .expect("tun");
    assert_eq!(get_bool(hmeta, "system-proxy"), Some(true));
    assert_eq!(get_bool(hmeta, "allow-bypass"), Some(true));
    assert_eq!(get_string(tun, "stack"), Some("lwip".to_owned()));
    assert_eq!(get_bool(tun, "dns-hijack"), Some(false));

    let options = store.vpn_options_for_profile(&profile_id).unwrap();
    assert!(options.system_proxy);
    assert!(!options.dns_hijacking);
    assert!(options.allow_bypass);
    assert_eq!(options.stack, "lwip");
}

#[test]
fn rejects_unsupported_stack_updates_and_safely_imports_legacy_values() {
    let options = vpn_options_from_yaml("tun:\n  stack: gvisor\n").unwrap();
    assert_eq!(options.stack, VpnStack::Smoltcp.as_str());

    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("stack")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "Stack",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();
    let error = store
        .set_profile_vpn_config(&profile_id, false, true, false, "gvisor".to_owned())
        .expect_err("unsupported stack must be rejected");
    assert!(error.to_string().contains("unsupported VPN network stack"));
}

#[test]
fn updates_dns_config_in_profile_yaml() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();

    store
        .set_profile_dns_config(
            &profile_id,
            vec![
                "https://dns.alidns.com/dns-query".to_owned(),
                "https://dns.alidns.com/dns-query".to_owned(),
                "1.1.1.1".to_owned(),
                " ".to_owned(),
            ],
            vec![
                "https://dns.google/dns-query".to_owned(),
                "https://dns.google/dns-query".to_owned(),
            ],
            BTreeMap::from([(
                "geosite:cn".to_owned(),
                vec![
                    "https://dns.alidns.com/dns-query".to_owned(),
                    " ".to_owned(),
                ],
            )]),
        )
        .unwrap();

    let options = store.vpn_options_for_profile(&profile_id).unwrap();
    assert_eq!(
        options.dns_servers,
        vec![
            "https://dns.alidns.com/dns-query".to_owned(),
            "1.1.1.1".to_owned()
        ]
    );
    assert_eq!(
        options.dns_fallbacks,
        vec!["https://dns.google/dns-query".to_owned()]
    );
    assert_eq!(
        options
            .dns_nameserver_policy
            .get("geosite:cn")
            .cloned()
            .unwrap_or_default(),
        vec!["https://dns.alidns.com/dns-query".to_owned()]
    );
}

#[test]
fn runtime_yaml_adds_default_nameserver_for_encrypted_dns_hostnames() {
    let root =
        std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-bootstrap")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS Bootstrap",
            "local",
            "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .unwrap();
    let options = VpnOptions {
        dns_servers: vec!["https://dns.alidns.com/dns-query".to_owned()],
        ..VpnOptions::default()
    };

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let dns = value
        .as_mapping()
        .and_then(|root| root.get(&value_key("dns")))
        .and_then(Value::as_mapping)
        .expect("dns");

    assert_eq!(
        get_string_list(dns, "default-nameserver"),
        vec![
            "223.5.5.5".to_owned(),
            "119.29.29.29".to_owned(),
            "1.1.1.1".to_owned(),
            "8.8.8.8".to_owned()
        ]
    );
}

#[test]
fn runtime_yaml_replaces_subscription_default_nameserver() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-managed")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS Managed",
            "local",
            r#"
mixed-port: 7890
dns:
  default-nameserver:
    - 127.0.0.1
  nameserver:
    - https://dns.alidns.com/dns-query
proxies: []
proxy-groups: []
rules: []
"#,
            None,
        )
        .unwrap();

    let options = store.vpn_options_for_profile(&profile_id).unwrap();
    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let dns = value
        .as_mapping()
        .and_then(|root| root.get(&value_key("dns")))
        .and_then(Value::as_mapping)
        .expect("dns");

    assert_eq!(
        get_string_list(dns, "default-nameserver"),
        vec![
            "223.5.5.5".to_owned(),
            "119.29.29.29".to_owned(),
            "1.1.1.1".to_owned(),
            "8.8.8.8".to_owned()
        ]
    );
}

#[test]
fn runtime_yaml_disables_subscription_system_hosts_dns_lookup() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-profile-test-{}",
        next_id("dns-system-hosts")
    ));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS System Hosts",
            "local",
            r#"
mixed-port: 7890
dns:
  use-hosts: true
  use-system-hosts: true
  nameserver:
    - 223.5.5.5
hosts:
  example.test: 203.0.113.10
proxies: []
proxy-groups: []
rules: []
"#,
            None,
        )
        .unwrap();

    let options = store.vpn_options_for_profile(&profile_id).unwrap();
    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let root = value.as_mapping().expect("root");
    let dns = root
        .get(&value_key("dns"))
        .and_then(Value::as_mapping)
        .expect("dns");

    assert_eq!(get_bool(dns, "use-hosts"), Some(true));
    assert_eq!(get_bool(dns, "use-system-hosts"), Some(false));
    assert!(root.get(&value_key("hosts")).is_some());
}

#[test]
fn runtime_yaml_removes_subscription_fallback_and_policy_when_unset() {
    let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-clear")));
    let mut store = ProfileStore::open(root).unwrap();
    let profile_id = store
        .import_profile_content(
            "DNS Clear",
            "local",
            r#"
mixed-port: 7890
dns:
  nameserver:
    - 9.9.9.9
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - '*.lan'
  fallback:
    - 8.8.8.8
  fallback-filter:
    geoip: true
    geoip-code: CN
  nameserver-policy:
    geosite:cn:
      - 223.5.5.5
proxies: []
proxy-groups: []
rules: []
"#,
            None,
        )
        .unwrap();
    let options = VpnOptions {
        dns_servers: vec!["1.1.1.1".to_owned()],
        dns_fallbacks: Vec::new(),
        dns_nameserver_policy: BTreeMap::new(),
        ..VpnOptions::default()
    };

    let yaml = store
        .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
        .unwrap();
    let value: Value = serde_yaml::from_str(&yaml).unwrap();
    let dns = value
        .as_mapping()
        .and_then(|root| root.get(&value_key("dns")))
        .and_then(Value::as_mapping)
        .expect("dns");

    assert_eq!(get_string_list(dns, "nameserver"), vec!["1.1.1.1"]);
    assert!(dns.get(&value_key("fallback")).is_none());
    assert!(dns.get(&value_key("nameserver-policy")).is_none());
    assert!(dns.get(&value_key("enhanced-mode")).is_none());
    assert!(dns.get(&value_key("fake-ip-range")).is_none());
    assert!(dns.get(&value_key("fake-ip-filter")).is_none());
    assert!(dns.get(&value_key("fallback-filter")).is_none());
}
