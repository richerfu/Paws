use super::*;
use base64::Engine;
use futures::StreamExt;

static TEST_LOG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn track_test_connection(tunnel: &Tunnel, host: &str) -> String {
    tunnel
        .statistics()
        .track_connection(
            Metadata {
                network: Network::Tcp,
                conn_type: ConnType::Inner,
                host: host.into(),
                dst_port: 443,
                ..Metadata::default()
            },
            "DOMAIN".into(),
            host.into(),
            std::iter::once(Arc::<str>::from("DIRECT")).collect(),
        )
        .to_string()
}

#[test]
fn core_snapshot_is_json() {
    let core = CoreHandle::new();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.proxy_groups.is_empty());
    let json = to_json(&snapshot).unwrap();
    assert!(json.contains("proxyGroups"));
    assert!(json.contains("vpnLifecycle"));
    assert!(json.contains("networkProtected"));
    assert!(json.contains("networkPorts"));
    assert!(json.contains("trafficHistory"));
    assert!(json.contains("handledPackets"));
    assert!(json.contains("meowRsVersion"));
    assert!(json.contains("privacySummary"));
    assert!(json.contains("geodata"));
    assert_eq!(snapshot.dns.listen, "127.0.0.1:1053");
    assert!(snapshot.dns.hijacking);
    assert_eq!(snapshot.geodata.len(), 3);
    assert!(snapshot
        .geodata
        .iter()
        .any(|file| file.path.ends_with("geosite.dat")));
    assert_eq!(snapshot.about.app_version, APP_VERSION);
    assert_eq!(snapshot.about.meow_rs_version, MEOW_RS_VERSION);
    assert_eq!(snapshot.about.arkit_rev, ARKIT_REV);
    assert!(!snapshot.about.privacy_summary.is_empty());
    assert!(snapshot.about.privacy_summary.iter().any(|note| {
        note.contains("HTTPS 服务")
            && note.contains("出口 IP")
            && note.contains("不包含订阅、节点、规则")
    }));
    assert!(snapshot
        .about
        .privacy_summary
        .iter()
        .any(|note| note.contains("不接入广告、行为分析或远程遥测服务")));
    assert_eq!(snapshot.about.exit_ip_services.len(), 6);
    assert!(snapshot
        .about
        .exit_ip_services
        .iter()
        .any(|service| service.name == "IPWho.is"));
}

#[test]
fn proxy_selection_refresh_preserves_the_existing_member_order() {
    let item = |name: &str, selected: bool| ProxyItem {
        name: name.to_owned(),
        proxy_type: "VLESS".to_owned(),
        delay_ms: None,
        selected,
    };
    let previous = vec![ProxyGroup {
        name: "GLOBAL".to_owned(),
        group_type: "Selector".to_owned(),
        selected: Some("Tokyo 04".to_owned()),
        fixed: None,
        proxies: vec![
            item("Tokyo 04", true),
            item("DIRECT", false),
            item("Tokyo 01", false),
            item("Tokyo 02", false),
        ],
    }];
    let mut refreshed = vec![ProxyGroup {
        name: "GLOBAL".to_owned(),
        group_type: "Selector".to_owned(),
        selected: Some("Tokyo 01".to_owned()),
        fixed: None,
        proxies: vec![
            item("Tokyo 01", true),
            item("DIRECT", false),
            item("Tokyo 02", false),
            item("Tokyo 04", false),
        ],
    }];

    preserve_proxy_group_member_order(&previous, &mut refreshed);

    assert_eq!(
        refreshed[0]
            .proxies
            .iter()
            .map(|proxy| proxy.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Tokyo 04", "DIRECT", "Tokyo 01", "Tokyo 02"]
    );
    assert_eq!(refreshed[0].selected.as_deref(), Some("Tokyo 01"));
    assert!(refreshed[0].proxies[2].selected);
}

#[test]
fn traffic_history_is_bounded_and_exposed_in_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-traffic-history-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    {
        let mut state = core.lock_state().unwrap();
        for speed in 0..40 {
            state.traffic.download_speed = speed;
            state.traffic.upload_speed = speed * 2;
            record_traffic_history(&mut state);
        }
        state.traffic.download_speed = 99;
        state.traffic.upload_speed = 199;
    }

    let snapshot = core.snapshot().unwrap();

    assert_eq!(snapshot.traffic_history.len(), MAX_TRAFFIC_HISTORY);
    assert_eq!(snapshot.traffic_history[0].download_speed, 9);
    assert_eq!(snapshot.traffic_history[30].download_speed, 39);
    let latest = snapshot.traffic_history.last().unwrap();
    assert_eq!(latest.download_speed, 0);
    assert_eq!(latest.upload_speed, 0);
}

#[test]
fn mode_changes_are_reflected() {
    let root = std::env::temp_dir().join(format!("hmeta-core-mode-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    core.set_mode(RuntimeMode::Direct).unwrap();
    assert_eq!(core.snapshot().unwrap().mode, RuntimeMode::Direct);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn global_mode_is_rejected_without_an_active_tunnel() {
    let root = std::env::temp_dir().join(format!("hmeta-global-empty-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);

    let error = core.set_mode(RuntimeMode::Global).unwrap_err();

    assert!(error
        .to_string()
        .contains("Global mode requires an active profile"));
    assert_eq!(core.snapshot().unwrap().mode, RuntimeMode::Rule);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_vpn_control_accepts_legacy_mode_only_payloads() {
    let control: PlatformVpnControl =
        serde_json::from_str(r#"{"mode":"direct","updatedAt":1}"#).unwrap();
    assert_eq!(control.mode, RuntimeMode::Direct);
    assert!(control.global_proxy.is_none());
    assert!(control.active_profile.is_none());
    assert!(control.proxy_selections.is_empty());
}

#[tokio::test]
async fn routing_modes_have_proxy_rule_and_direct_semantics() {
    let root = std::env::temp_dir().join(format!("hmeta-routing-mode-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Routing modes",
            "test",
            r#"
proxies:
  - name: HTTP-MOCK
    type: http
    server: 127.0.0.1
    port: 18080
rules:
  - DOMAIN,rule-proxy.example,HTTP-MOCK
  - MATCH,DIRECT
"#,
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let tunnel = core.lock_state().unwrap().tunnel.clone().unwrap();
    assert_eq!(
        tunnel.proxy("GLOBAL").unwrap().current().as_deref(),
        Some("DIRECT"),
        "the upstream auto-created GLOBAL selector defaults to DIRECT"
    );
    core.select_proxy("GLOBAL", "HTTP-MOCK").await.unwrap();
    assert_eq!(
        tunnel.proxy("GLOBAL").unwrap().current().as_deref(),
        Some("HTTP-MOCK"),
        "the selected subscription node must be stored in the GLOBAL selector"
    );

    core.set_mode(RuntimeMode::Global).unwrap();
    let global = tunnel.proxy("GLOBAL").unwrap();
    assert_eq!(tunnel.mode(), TunnelMode::Global);
    assert_eq!(global.current().as_deref(), Some("HTTP-MOCK"));
    assert!(target_routes_through_proxy(
        &tunnel,
        global.current().as_deref().unwrap(),
        &mut BTreeSet::new()
    ));
    core.set_mode(RuntimeMode::Rule).unwrap();
    let proxy_metadata = Metadata {
        network: Network::Tcp,
        host: "rule-proxy.example".into(),
        dst_port: 443,
        ..Metadata::default()
    };
    let direct_metadata = Metadata {
        network: Network::Tcp,
        host: "rule-direct.example".into(),
        dst_port: 443,
        ..Metadata::default()
    };
    let (rule_proxy, _, _) = tunnel.inner().resolve_proxy(&proxy_metadata).unwrap();
    let (rule_direct, _, _) = tunnel.inner().resolve_proxy(&direct_metadata).unwrap();
    assert_eq!(rule_proxy.adapter_type(), AdapterType::Http);
    assert_eq!(rule_direct.adapter_type(), AdapterType::Direct);

    core.set_mode(RuntimeMode::Direct).unwrap();
    let (direct, _, _) = tunnel.inner().resolve_proxy(&proxy_metadata).unwrap();
    assert_eq!(tunnel.mode(), TunnelMode::Direct);
    assert_eq!(direct.adapter_type(), AdapterType::Direct);

    let snapshot = core.snapshot().unwrap();
    assert_eq!(
        snapshot.profiles[0]
            .selected_proxies
            .get("GLOBAL")
            .map(String::as_str),
        Some("HTTP-MOCK")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rule_lookup_uses_compiled_rule_order_independently_of_runtime_mode() {
    let root = std::env::temp_dir().join(format!("hmeta-rule-lookup-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Rule lookup",
            "test",
            r#"
proxies:
  - name: HTTP-MOCK
    type: http
    server: 127.0.0.1
    port: 18080
proxy-groups:
  - name: Proxy
    type: select
    proxies: [HTTP-MOCK, DIRECT]
rules:
  - DOMAIN-SUFFIX,example.com,Proxy
  - IP-CIDR,203.0.113.0/24,DIRECT
  - MATCH,Proxy
"#,
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    core.set_mode(RuntimeMode::Global).unwrap();
    let tunnel = core.lock_state().unwrap().tunnel.clone().unwrap();
    let rule_match_count_before = tunnel
        .statistics()
        .rule_match
        .snapshot()
        .into_iter()
        .map(|(_, count)| count)
        .sum::<u64>();

    let domain = core.lookup_rule(" API.Example.COM. ").await.unwrap();
    assert_eq!(domain.query, "api.example.com");
    assert_eq!(domain.input_kind, RuleLookupInputKind::Domain);
    assert!(domain.matched);
    assert_eq!(domain.rule_type.as_deref(), Some("DOMAIN-SUFFIX"));
    assert_eq!(domain.rule_payload.as_deref(), Some("example.com"));
    assert_eq!(domain.target, "Proxy");
    assert_eq!(
        domain.rule_line.as_deref(),
        Some("DOMAIN-SUFFIX,example.com,Proxy")
    );
    assert!(!domain.resolution_attempted);

    let ip = core.lookup_rule("203.0.113.42").await.unwrap();
    assert_eq!(ip.query, "203.0.113.42");
    assert_eq!(ip.input_kind, RuleLookupInputKind::Ip);
    assert!(ip.matched);
    assert_eq!(ip.rule_type.as_deref(), Some("IP-CIDR"));
    assert_eq!(ip.target, "DIRECT");
    assert_eq!(
        ip.rule_line.as_deref(),
        Some("IP-CIDR,203.0.113.0/24,DIRECT")
    );

    let error = core
        .lookup_rule("https://example.com/path")
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("valid domain name or IP address"));
    let rule_match_count_after = tunnel
        .statistics()
        .rule_match
        .snapshot()
        .into_iter()
        .map(|(_, count)| count)
        .sum::<u64>();
    assert_eq!(rule_match_count_after, rule_match_count_before);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rule_groups_keep_independent_selections_and_nested_edges() {
    let root = std::env::temp_dir().join(format!("hmeta-rule-groups-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Independent rule groups",
            "test",
            r#"
proxies:
  - name: HTTP-A
    type: http
    server: 127.0.0.1
    port: 18080
  - name: HTTP-B
    type: http
    server: 127.0.0.1
    port: 18081
proxy-groups:
  - name: Child
    type: select
    proxies: [HTTP-A, HTTP-B]
  - name: Parent
    type: select
    proxies: [DIRECT, Child]
  - name: Streaming
    type: select
    proxies: [HTTP-B, DIRECT]
rules:
  - DOMAIN,parent.example,Parent
  - DOMAIN,child.example,Child
  - DOMAIN,stream.example,Streaming
  - MATCH,DIRECT
"#,
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    core.select_proxy("Child", "HTTP-B").await.unwrap();
    core.select_proxy("Parent", "Child").await.unwrap();
    core.select_proxy("Streaming", "DIRECT").await.unwrap();
    core.select_proxy("GLOBAL", "HTTP-A").await.unwrap();

    let snapshot = core.snapshot().unwrap();
    let child = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Child")
        .expect("Child group");
    let parent = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Parent")
        .expect("Parent group");
    let streaming = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Streaming")
        .expect("Streaming group");
    assert_eq!(child.selected.as_deref(), Some("HTTP-B"));
    assert_eq!(parent.selected.as_deref(), Some("Child"));
    assert_eq!(streaming.selected.as_deref(), Some("DIRECT"));
    assert!(parent
        .proxies
        .iter()
        .any(|proxy| proxy.name == "Child" && proxy.proxy_type == "Selector"));

    let subscription_selections = |snapshot: &RuntimeSnapshot| {
        snapshot
            .proxy_groups
            .iter()
            .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
            .map(|group| (group.name.clone(), group.selected.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let selections_before_mode_change = subscription_selections(&snapshot);
    core.set_mode(RuntimeMode::Global).unwrap();
    let global_snapshot = core.snapshot().unwrap();
    assert_eq!(
        subscription_selections(&global_snapshot),
        selections_before_mode_change
    );
    assert_eq!(
        global_snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "GLOBAL")
            .and_then(|group| group.selected.as_deref()),
        Some("HTTP-A"),
        "Global mode follows the separately selected subscription node"
    );

    core.select_proxy("Streaming", "HTTP-B").await.unwrap();
    let changed_in_global_mode = core.snapshot().unwrap();
    assert_eq!(
        changed_in_global_mode
            .proxy_groups
            .iter()
            .find(|group| group.name == "GLOBAL")
            .and_then(|group| group.selected.as_deref()),
        Some("HTTP-A"),
        "changing a rule group must not rewrite the selected Global node"
    );
    let mut selections_after_group_change = selections_before_mode_change.clone();
    selections_after_group_change.insert("Streaming".to_owned(), Some("HTTP-B".to_owned()));
    assert_eq!(
        subscription_selections(&changed_in_global_mode),
        selections_after_group_change
    );
    for mode in [RuntimeMode::Direct, RuntimeMode::Global, RuntimeMode::Rule] {
        core.set_mode(mode).unwrap();
        assert_eq!(
            subscription_selections(&core.snapshot().unwrap()),
            selections_after_group_change,
            "switching to {mode:?} must preserve subscription group selections"
        );
    }

    for (domain, target) in [
        ("parent.example", "Parent"),
        ("child.example", "Child"),
        ("stream.example", "Streaming"),
    ] {
        let lookup = core.lookup_rule(domain).await.unwrap();
        assert_eq!(lookup.target, target);
    }

    let tunnel = core.lock_state().unwrap().tunnel.clone().unwrap();
    for (domain, expected_group) in [
        ("parent.example", "Parent"),
        ("child.example", "Child"),
        ("stream.example", "Streaming"),
    ] {
        let metadata = Metadata {
            network: Network::Tcp,
            host: domain.into(),
            dst_port: 443,
            ..Metadata::default()
        };
        let (proxy, _, _) = tunnel.inner().resolve_proxy(&metadata).unwrap();
        assert_eq!(proxy.name(), expected_group);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn global_mode_falls_back_to_direct_without_subscription_nodes() {
    let root =
        std::env::temp_dir().join(format!("hmeta-global-no-proxy-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content("Direct only", "test", "rules:\n  - MATCH,DIRECT\n", None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    // Community (meow/mihomo) semantics: an unselected GLOBAL falls back
    // to its built-in DIRECT outbound when the subscription exposes no
    // proxy nodes, so Global mode must switch without error.
    core.set_mode(RuntimeMode::Global).unwrap();
    let state = core.lock_state().unwrap();
    assert_eq!(state.mode, RuntimeMode::Global);
    assert_eq!(state.tunnel.as_ref().unwrap().mode(), TunnelMode::Global);
    let global = state.tunnel.as_ref().unwrap().proxy("GLOBAL").unwrap();
    assert_eq!(
        global.current().as_deref(),
        Some("DIRECT"),
        "the no-subscription GLOBAL selector defaults to DIRECT"
    );
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_vpn_status_is_reflected_in_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-platform-vpn-status-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let snapshot = core.snapshot().unwrap();
    assert!(!snapshot.engine_loaded);
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Stopped);
    assert!(!snapshot.running);
    assert!(!snapshot.vpn_running);
    assert!(!snapshot.network_protected);
    core.set_platform_vpn_starting(true).unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(!snapshot.engine_loaded);
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    assert!(!snapshot.running);
    assert!(!snapshot.vpn_running);
    assert!(!snapshot.network_protected);
    core.set_platform_vpn_running(true).unwrap();
    core.set_platform_network_protected(true, None).unwrap();
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Connected);
    assert!(snapshot.vpn_running);
    assert!(snapshot.network_protected);
    assert!(snapshot.network_protect_error.is_none());
    core.set_platform_vpn_running(false).unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(!snapshot.engine_loaded);
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Stopped);
    assert!(!snapshot.running);
    assert!(!snapshot.vpn_running);
    assert!(!snapshot.network_protected);
    assert!(snapshot.network_protect_error.is_none());
    core.set_platform_network_protected(false, Some("denied".to_owned()))
        .unwrap();
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Failed);
    assert!(!snapshot.network_protected);
    assert_eq!(snapshot.network_protect_error.as_deref(), Some("denied"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_vpn_state_revision_is_strictly_monotonic() {
    let root =
        std::env::temp_dir().join(format!("hmeta-platform-vpn-revision-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let future_revision = now_unix_nanos().saturating_add(3_600_000_000_000);
    {
        let mut state = core.lock_state().unwrap();
        state.platform_vpn_state_updated_at = future_revision;
    }

    core.set_platform_vpn_starting(true).unwrap();

    let revision = core.lock_state().unwrap().platform_vpn_state_updated_at;
    assert_eq!(revision, future_revision + 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_vpn_start_timeout_becomes_visible_failure() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-platform-vpn-timeout-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    core.set_platform_vpn_starting(true).unwrap();
    assert!(core.expire_platform_vpn_start().unwrap());
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Failed);
    assert!(!snapshot.vpn_running);
    assert!(snapshot
        .network_protect_error
        .as_deref()
        .is_some_and(|error| error.contains("startup timeout")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_vpn_state_accepts_legacy_frames_without_start_transaction() {
    let state: PlatformVpnState =
        serde_json::from_str(r#"{"starting":true,"running":false,"updatedAt":1}"#).unwrap();

    assert_eq!(state.start_outcome, PlatformStartOutcome::Idle);
    assert!(state.start_attempt_id.is_empty());
    assert!(!state.extension_attached);
    assert!(state.starting);
}

#[tokio::test]
async fn platform_start_completes_only_on_matching_connected_terminal() {
    let root =
        std::env::temp_dir().join(format!("hmeta-platform-vpn-connected-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    assert!(!core
        .fail_platform_vpn_start("older-attempt", "late rejection".to_owned())
        .unwrap());
    core.set_platform_vpn_running(true).unwrap();

    assert_eq!(
        core.await_platform_vpn_start(&attempt_id).await.unwrap(),
        PlatformStartOutcome::Connected
    );
    assert!(!core
        .fail_platform_vpn_start(&attempt_id, "late rejection".to_owned())
        .unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn platform_start_failure_is_exactly_once() {
    let root = std::env::temp_dir().join(format!("hmeta-platform-vpn-failed-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    assert!(core
        .fail_platform_vpn_start(&attempt_id, "system rejected".to_owned())
        .unwrap());
    assert!(!core.cancel_platform_vpn_start(&attempt_id).unwrap());
    let error = core
        .await_platform_vpn_start(&attempt_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("system rejected"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn platform_start_attachment_wait_distinguishes_authorization_bootstrap() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-platform-vpn-attachment-wait-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    assert!(!core
        .await_platform_vpn_start_attachment(&attempt_id, Duration::from_millis(10))
        .await
        .unwrap());
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core
        .await_platform_vpn_start_attachment(&attempt_id, Duration::from_millis(10))
        .await
        .unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn system_rejection_only_fails_before_extension_attachment() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-platform-vpn-attachment-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let unattached = core.begin_platform_vpn_start().unwrap();
    assert!(core
        .fail_unattached_platform_vpn_start(&unattached, "system rejected".to_owned())
        .unwrap());

    let attached = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attached).unwrap();
    assert!(!core
        .fail_unattached_platform_vpn_start(&attached, "late system rejection".to_owned())
        .unwrap());
    core.set_platform_vpn_running(true).unwrap();
    assert!(!core
        .fail_unattached_platform_vpn_start(&attached, "late timeout".to_owned())
        .unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn platform_start_deadline_produces_one_failed_terminal() {
    let root =
        std::env::temp_dir().join(format!("hmeta-platform-vpn-deadline-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    let error = core
        .await_platform_vpn_start_with_deadline(&attempt_id, Duration::from_millis(10))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("startup deadline"));
    assert!(!core
        .fail_platform_vpn_start(&attempt_id, "late failure".to_owned())
        .unwrap());
    assert!(!core.cancel_platform_vpn_start(&attempt_id).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn vpn_lifecycle_derives_service_state_from_engine_vpn_and_protect_status() {
    assert_eq!(
        vpn_lifecycle(false, false, false, false, false, None),
        VpnLifecycle::Stopped
    );
    assert_eq!(
        vpn_lifecycle(true, false, false, false, false, None),
        VpnLifecycle::EngineLoaded
    );
    assert_eq!(
        vpn_lifecycle(true, true, false, false, false, None),
        VpnLifecycle::Starting
    );
    assert_eq!(
        vpn_lifecycle(true, false, true, false, true, None),
        VpnLifecycle::Connected
    );
    assert_eq!(
        vpn_lifecycle(true, false, true, false, false, Some("denied")),
        VpnLifecycle::ProtectFailed
    );
    assert_eq!(
        vpn_lifecycle(true, false, false, false, false, Some("denied")),
        VpnLifecycle::Failed
    );
}

#[tokio::test]
async fn meow_crate_feature_matrix_loads_reference_client_protocols() {
    let yaml = r#"
mixed-port: 7890
external-controller: 127.0.0.1:0
proxies:
  - name: SS
    type: ss
    server: 127.0.0.1
    port: 8388
    cipher: aes-128-gcm
    password: test-password
  - name: Trojan
    type: trojan
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: VLESS
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: 00000000-0000-0000-0000-000000000001
  - name: AnyTLS
    type: anytls
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: VMess
    type: vmess
    server: 127.0.0.1
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    cipher: auto
  - name: Snell
    type: snell
    server: 127.0.0.1
    port: 8388
    psk: test-password
    version: 4
  - name: Hysteria2
    type: hysteria2
    server: 127.0.0.1
    port: 443
    password: test-password
    skip-cert-verify: true
  - name: HTTP
    type: http
    server: 127.0.0.1
    port: 8080
  - name: SOCKS5
    type: socks5
    server: 127.0.0.1
    port: 1080
proxy-groups:
  - name: Proxy
    type: select
    proxies: [SS, Trojan, VLESS, AnyTLS, VMess, Snell, Hysteria2, HTTP, SOCKS5, DIRECT]
rules:
  - MATCH,Proxy
"#;
    let config = load_meow_config(yaml).await.unwrap();
    for proxy in [
        "SS",
        "Trojan",
        "VLESS",
        "AnyTLS",
        "VMess",
        "Snell",
        "Hysteria2",
        "HTTP",
        "SOCKS5",
    ] {
        assert!(
            config.proxies.contains_key(proxy),
            "meow config omitted enabled proxy type {proxy}"
        );
    }
}

#[test]
fn snapshot_includes_runtime_tracing_logs() {
    let _guard = TEST_LOG_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("hmeta-runtime-log-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let message = format!(
        "hmeta runtime log test {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    tracing::warn!(target: "hmeta_core_test", "{}", message);

    let snapshot = core.snapshot().unwrap();
    assert!(snapshot
        .logs
        .iter()
        .any(|log| log.level == "warning" && log.message.contains(&message)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_log_page_excludes_arkit_framework_targets() {
    let _guard = TEST_LOG_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("hmeta-runtime-filter-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let message = format!("arkit framework log {}", now_unix_nanos());

    tracing::warn!(target: "arkit::renderer", "{}", message);

    assert!(!core
        .snapshot()
        .unwrap()
        .logs
        .iter()
        .any(|log| log.message.contains(&message)));
    assert!(is_vpn_log_target("hmeta_vpn::tun"));
    assert!(is_vpn_log_target("meow_tunnel::dispatcher"));
    assert!(!is_vpn_log_target("arkit::renderer"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn clear_logs_removes_state_and_runtime_logs() {
    let _guard = TEST_LOG_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("hmeta-clear-log-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    {
        let mut state = core.lock_state().unwrap();
        state.logs.push(warning_log("state log to clear"));
    }
    tracing::warn!(target: "hmeta_core_test", "runtime log to clear");
    assert!(!core.snapshot().unwrap().logs.is_empty());

    core.clear_logs().unwrap();

    assert!(core.snapshot().unwrap().logs.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_loads_engine_without_marking_vpn_connected() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-reload-state-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();

    core.reload_config(&profile_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.engine_loaded);
    assert!(snapshot.running);
    assert!(!snapshot.vpn_running);
    assert!(!snapshot.rules.is_empty());
    assert!(snapshot
        .rules
        .iter()
        .any(|rule| rule.source == "profile-yaml" && rule.enabled));
    assert_eq!(snapshot.profiles[0].rule_count, snapshot.rules.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_activity_rules_persist_and_hot_update_the_existing_tunnel() {
    let root =
        std::env::temp_dir().join(format!("hmeta-core-manual-rule-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(root.clone());
    let profile_id = core
        .import_profile_from_content(
            "Manual rule",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let original_inner = {
        let state = core.lock_state().unwrap();
        Arc::clone(state.tunnel.as_ref().unwrap().inner())
    };

    let added = core
        .apply_manual_rule(
            &profile_id,
            &ManualRuleSpec {
                match_kind: hmeta_model::ManualRuleMatchKind::Domain,
                value: "API.Example.COM.".to_owned(),
                target: "Proxy".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        added.mutation.kind,
        hmeta_model::ManualRuleMutationKind::Added
    );
    assert_eq!(added.mutation.line, "DOMAIN,api.example.com,Proxy");
    assert!(added.live_updated);
    assert!(added.rule_mode_active);

    let updated = core
        .apply_manual_rule(
            &profile_id,
            &ManualRuleSpec {
                match_kind: hmeta_model::ManualRuleMatchKind::Domain,
                value: "api.example.com".to_owned(),
                target: "DIRECT".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.mutation.rule_id, added.mutation.rule_id);
    assert_eq!(
        updated.mutation.kind,
        hmeta_model::ManualRuleMutationKind::Updated
    );

    let snapshot = core.snapshot().unwrap();
    let matching = snapshot
        .rules
        .iter()
        .filter(|rule| rule.line.starts_with("DOMAIN,api.example.com,"))
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].line, "DOMAIN,api.example.com,DIRECT");
    let current_inner = {
        let state = core.lock_state().unwrap();
        Arc::clone(state.tunnel.as_ref().unwrap().inner())
    };
    assert!(Arc::ptr_eq(&original_inner, &current_inner));

    let reopened = ProfileStore::open(&root).unwrap();
    assert!(reopened
        .rules_for_profile(&profile_id)
        .iter()
        .any(|rule| rule.line == "DOMAIN,api.example.com,DIRECT"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vpn_prepare_reuses_an_already_loaded_tunnel() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-vpn-prepare-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = Arc::new(CoreHandle::new_with_profile_root(root));
    core.import_profile_from_content(
        "Direct",
        "test",
        &hmeta_profile::default_runtime_yaml(),
        None,
    )
    .await
    .unwrap();

    let (first_prepare, second_prepare) =
        tokio::join!(core.prepare_active_vpn(), core.prepare_active_vpn(),);
    assert_ne!(first_prepare.unwrap(), second_prepare.unwrap());
    let reloads_after_cold_prepare = core
        .snapshot()
        .unwrap()
        .logs
        .iter()
        .filter(|log| log.message.starts_with("config reloaded from profile"))
        .count();

    assert!(!core.prepare_active_vpn().await.unwrap());
    let reloads_after_warm_prepare = core
        .snapshot()
        .unwrap()
        .logs
        .iter()
        .filter(|log| log.message.starts_with("config reloaded from profile"))
        .count();
    assert_eq!(reloads_after_warm_prepare, reloads_after_cold_prepare);
    assert_eq!(reloads_after_cold_prepare, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_ui_cache_populates_cold_snapshot_before_reload() {
    let root = std::env::temp_dir().join(format!("hmeta-core-ui-cache-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Cached dashboard",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let loaded = core.snapshot().unwrap();
    assert!(loaded.engine_loaded);
    assert!(!loaded.proxy_groups.is_empty());
    // The UI cache persist is best-effort on a background thread (see
    // persist_runtime_ui_cache_best_effort); wait a bounded window for it.
    let cache_file = root.join(RUNTIME_UI_CACHE_FILE);
    for _ in 0..100 {
        if cache_file.is_file() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(cache_file.is_file());
    drop(core);

    let cold_core = CoreHandle::new_with_profile_root(&root);
    let cold = cold_core.snapshot().unwrap();
    assert!(!cold.engine_loaded);
    assert_eq!(cold.active_profile.as_deref(), Some(profile_id.as_str()));
    assert_eq!(cold.proxy_groups.len(), loaded.proxy_groups.len());
    assert_eq!(cold.proxy_groups[0].name, loaded.proxy_groups[0].name);

    cold_core.select_proxy("Proxy", "DIRECT").await.unwrap();
    let selected = cold_core.snapshot().unwrap();
    assert!(selected.engine_loaded);
    assert_eq!(
        selected
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .and_then(|group| group.selected.as_deref()),
        Some("DIRECT")
    );
    assert!(!cold_core.prepare_active_vpn().await.unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_ui_cache_writer_keeps_the_latest_selection() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-ui-cache-writer-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Cache writer",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    {
        let mut state = core.lock_state().unwrap();
        state
            .proxy_groups
            .iter_mut()
            .find(|group| group.name == "Proxy")
            .unwrap()
            .selected = Some("older".to_owned());
        persist_runtime_ui_cache_best_effort(&mut state);
        state
            .proxy_groups
            .iter_mut()
            .find(|group| group.name == "Proxy")
            .unwrap()
            .selected = Some("latest".to_owned());
        persist_runtime_ui_cache_best_effort(&mut state);
    }

    let cache_file = root.join(RUNTIME_UI_CACHE_FILE);
    let mut selected = None;
    for _ in 0..100 {
        selected = std::fs::read(&cache_file)
            .ok()
            .and_then(|content| serde_json::from_slice::<RuntimeUiCache>(&content).ok())
            .and_then(|cache| {
                cache
                    .proxy_groups
                    .into_iter()
                    .find(|group| group.name == "Proxy")
                    .and_then(|group| group.selected)
            });
        if selected.as_deref() == Some("latest") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(selected.as_deref(), Some("latest"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vpn_process_role_does_not_write_the_ui_cache() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-vpn-cache-writer-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "VPN cache isolation",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.lock_state().unwrap().runtime_ui_cache_writes_enabled = false;
    core.reload_config(&profile_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(!root.join(RUNTIME_UI_CACHE_FILE).exists());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_ui_cache_is_ignored_after_profile_content_changes() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-ui-cache-revision-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Changed dashboard",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    // The UI cache persist is best-effort on a background thread (see
    // persist_runtime_ui_cache_best_effort); wait a bounded window for it.
    let cache_file = root.join(RUNTIME_UI_CACHE_FILE);
    for _ in 0..100 {
        if cache_file.is_file() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(cache_file.is_file());
    drop(core);

    let mut profiles = ProfileStore::open(&root).unwrap();
    let changed_yaml = format!(
        "{}\n# invalidate cached dashboard\n",
        hmeta_profile::default_runtime_yaml()
    );
    profiles
        .update_profile_content(&profile_id, changed_yaml)
        .unwrap();
    drop(profiles);

    let cold_core = CoreHandle::new_with_profile_root(&root);
    let cold = cold_core.snapshot().unwrap();
    assert!(!cold.engine_loaded);
    assert!(cold.proxy_groups.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_ignores_subscription_geodata_auto_update_fields() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-geodata-clean-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Geodata Auto Update",
            "test",
            r#"
mixed-port: 7890
geodata:
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
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
        .await
        .unwrap();

    core.reload_config(&profile_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.engine_loaded);
    assert!(snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .is_some_and(|profile| profile.runtime_yaml_path.ends_with(".yaml")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_ignores_app_managed_listener_and_dns_validation_fields() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-managed-validation-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Managed Fields",
            "test",
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
        .await
        .unwrap();

    core.reload_config(&profile_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.engine_loaded);
    assert_eq!(snapshot.dns.listen, "127.0.0.1:1053");
    assert_eq!(snapshot.vpn_options.dns_servers, vec!["223.5.5.5"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vpn_lifecycle_reloads_tunnel_starts_and_stops() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-vpn-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let mut fds = [0_i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let options_json = to_json(&VpnOptions::default()).unwrap();
    core.start_vpn(fds[0], &options_json).await.unwrap();
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }

    let running = core.snapshot().unwrap();
    assert!(running.engine_loaded);
    assert!(running.running);
    assert!(running.vpn_running);
    assert_eq!(running.vpn_options.mtu, VpnOptions::default().mtu);

    core.stop_vpn().unwrap();
    let stopped = core.snapshot().unwrap();
    assert!(stopped.engine_loaded);
    assert!(stopped.running);
    assert!(!stopped.vpn_running);
    assert_eq!(stopped.traffic.upload_speed, 0);
    assert_eq!(stopped.traffic.download_speed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dns_config_updates_reload_active_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-dns-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content("DNS", "test", &hmeta_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    core.set_profile_dns_config(
        &profile_id,
        vec!["223.5.5.5".to_owned()],
        vec!["1.1.1.1".to_owned()],
        BTreeMap::from([("geosite:cn".to_owned(), vec!["223.5.5.5".to_owned()])]),
    )
    .await
    .unwrap();

    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.vpn_options.dns_servers, vec!["223.5.5.5"]);
    assert_eq!(snapshot.dns.fallbacks, vec!["1.1.1.1"]);
    assert_eq!(
        snapshot
            .dns
            .nameserver_policy
            .get("geosite:cn")
            .cloned()
            .unwrap_or_default(),
        vec!["223.5.5.5"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vpn_config_updates_reload_active_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-vpn-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content("VPN", "test", &hmeta_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    core.set_profile_vpn_config(&profile_id, true, false, true, "lwip".to_owned())
        .await
        .unwrap();

    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.vpn_options.system_proxy);
    assert!(!snapshot.vpn_options.dns_hijacking);
    assert!(snapshot.vpn_options.allow_bypass);
    assert_eq!(snapshot.vpn_options.stack, "lwip");
    assert!(!snapshot.dns.hijacking);
}

#[test]
fn dns_snapshot_exposes_tun_cache_diagnostics() {
    let stats = TunStats {
        dns_packets: 7,
        dns_cache_hits: 3,
        dns_cache_misses: 4,
        ..TunStats::default()
    };
    let snapshot = dns_snapshot(&VpnOptions::default(), Some(&stats));

    assert_eq!(snapshot.handled_packets, 7);
    assert_eq!(snapshot.cache_hits, 3);
    assert_eq!(snapshot.cache_misses, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_proxy_delay_reaches_local_tcp_listener() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-direct-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_handle = tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    let delay = core
        .test_proxy_delay("DIRECT", Some(&format!("http://{addr}")), Some(1000))
        .await
        .unwrap();
    assert!(delay < 1000);
    accept_handle.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_proxy_echo_roundtrips_local_tcp_payload() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-direct-echo-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut payload = vec![0_u8; "hmeta-echo-payload".len()];
        stream.read_exact(&mut payload).await.unwrap();
        stream.write_all(&payload).await.unwrap();
    });

    let echoed = core
        .test_proxy_echo(
            "DIRECT",
            &format!("http://{addr}"),
            "hmeta-echo-payload",
            Some(1000),
        )
        .await
        .unwrap();
    assert_eq!(echoed, "hmeta-echo-payload");
    accept_handle.await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot
        .logs
        .iter()
        .any(|log| { log.message.contains("DIRECT echo roundtrip: 18 bytes") }));
}

#[test]
fn proxy_echo_metadata_uses_an_opaque_tcp_tunnel() {
    let metadata = proxy_test_metadata("http://127.0.0.1:8080", "hmeta-echo").unwrap();
    assert_eq!(metadata.conn_type, ConnType::Inner);
    assert_eq!(metadata.host.as_str(), "127.0.0.1");
    assert_eq!(metadata.dst_port, 8080);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_uses_meow_tunnel_statistics_for_connections_and_traffic() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-meow-stats-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let tunnel = {
        let state = core.lock_state().unwrap();
        state.tunnel.clone().expect("loaded tunnel")
    };
    tunnel.statistics().add_upload(128);
    tunnel.statistics().add_download(256);
    let connection_id = track_test_connection(&tunnel, "example.com");

    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.traffic.meow_upload_bytes, 128);
    assert_eq!(snapshot.traffic.meow_download_bytes, 256);
    assert_eq!(snapshot.traffic.upload_bytes, 128);
    assert_eq!(snapshot.traffic.download_bytes, 256);
    assert_eq!(snapshot.connections.len(), 1);
    let connection = &snapshot.connections[0];
    assert_eq!(connection.id, connection_id);
    assert_eq!(connection.host, "example.com:443");
    assert_eq!(connection.network, "tcp");
    assert_eq!(connection.rule, "DOMAIN(example.com)");
    assert_eq!(connection.rule_payload, "example.com");
    assert_eq!(connection.proxy, "DIRECT");
    assert_eq!(connection.chains, vec!["DIRECT"]);
    assert!(!connection.started_at.is_empty());
    assert_eq!(connection.started_at.len(), 20);
    assert_eq!(connection.started_at.as_bytes().get(10), Some(&b'T'));
    assert!(connection.started_at.ends_with('Z'));
    assert_eq!(snapshot.request_history.len(), 1);
    let request = &snapshot.request_history[0];
    assert_eq!(request.id, connection_id);
    assert_eq!(request.host, "example.com:443");
    assert_eq!(request.network, "tcp");
    assert_eq!(request.rule, "DOMAIN(example.com)");
    assert_eq!(request.proxy, "DIRECT");
    assert!(request.active);

    core.close_connection(&connection_id).unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.connections.is_empty());
    assert_eq!(snapshot.request_history.len(), 1);
    assert_eq!(snapshot.request_history[0].id, connection_id);
    assert!(!snapshot.request_history[0].active);

    core.clear_request_history().unwrap();
    assert!(core.snapshot().unwrap().request_history.is_empty());

    let first = track_test_connection(&tunnel, "one.example");
    let second = track_test_connection(&tunnel, "two.example");
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.connections.len(), 2);
    assert!(snapshot.request_history.iter().any(|item| item.id == first));
    assert!(snapshot
        .request_history
        .iter()
        .any(|item| item.id == second));

    core.close_all_connections().unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.connections.is_empty());
    assert_eq!(snapshot.request_history.len(), 2);
    assert!(snapshot.request_history.iter().all(|item| !item.active));
    assert!(snapshot
        .logs
        .iter()
        .any(|log| log.message == "all connections closed: 2"));
}

#[test]
fn tun_descriptor_rx_is_upload_and_tx_is_download() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-tun-direction-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    {
        let mut state = core.lock_state().unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                rx_bytes: 340,
                tx_bytes: 120,
                ..TunStats::default()
            },
        )
        .unwrap();
        assert_eq!(state.traffic.upload_bytes, 340);
        assert_eq!(state.traffic.download_bytes, 120);
        assert_eq!(state.traffic.tun_upload_bytes, 340);
        assert_eq!(state.traffic.tun_download_bytes, 120);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_traffic_is_not_double_counted_after_vpn_stop_baseline() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-traffic-stop-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let tunnel = {
        let state = core.lock_state().unwrap();
        state.tunnel.clone().expect("loaded tunnel")
    };
    tunnel.statistics().add_upload(128);
    tunnel.statistics().add_download(256);

    {
        let mut state = core.lock_state().unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                tx_bytes: 128,
                rx_bytes: 256,
                ..TunStats::default()
            },
        )
        .unwrap();
        baseline_meow_traffic_sample(&mut state);
    }

    let snapshot = core.snapshot().unwrap();
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .expect("profile summary");
    assert_eq!(profile.upload_bytes, 256);
    assert_eq!(profile.download_bytes, 128);
    // With no live native TUN handle this snapshot intentionally falls
    // back to meow's already-semantic upload/download counters.
    assert_eq!(snapshot.traffic.upload_bytes, 128);
    assert_eq!(snapshot.traffic.download_bytes, 256);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_switch_settles_tun_traffic_to_previous_profile() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-profile-switch-tun-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content(
            "First",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();

    {
        let mut state = core.lock_state().unwrap();
        state.profiles.set_active(&first_id).unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                tx_bytes: 100,
                rx_bytes: 200,
                ..TunStats::default()
            },
        )
        .unwrap();
        settle_traffic_before_profile_switch(
            &mut state,
            Some(&TunStats {
                tx_bytes: 150,
                rx_bytes: 260,
                ..TunStats::default()
            }),
        )
        .unwrap();
        state.profiles.set_active(&second_id).unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                tx_bytes: 180,
                rx_bytes: 300,
                ..TunStats::default()
            },
        )
        .unwrap();
    }

    let snapshot = core.snapshot().unwrap();
    let first = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == first_id)
        .expect("first profile");
    let second = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == second_id)
        .expect("second profile");
    assert_eq!(first.upload_bytes, 260);
    assert_eq!(first.download_bytes, 150);
    assert_eq!(second.upload_bytes, 40);
    assert_eq!(second.download_bytes, 30);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_switch_settles_meow_traffic_when_native_stats_are_unavailable() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-profile-switch-meow-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content(
            "First",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&first_id).await.unwrap();
    let tunnel = {
        let state = core.lock_state().unwrap();
        state.tunnel.clone().expect("loaded tunnel")
    };
    tunnel.statistics().add_upload(320);
    tunnel.statistics().add_download(640);

    {
        let mut state = core.lock_state().unwrap();
        settle_traffic_before_profile_switch(&mut state, None).unwrap();
        state.profiles.set_active(&second_id).unwrap();
    }

    let snapshot = core.snapshot().unwrap();
    let first = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == first_id)
        .expect("first profile");
    let second = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == second_id)
        .expect("second profile");
    assert_eq!(first.upload_bytes, 320);
    assert_eq!(first.download_bytes, 640);
    assert_eq!(second.upload_bytes, 0);
    assert_eq!(second.download_bytes, 0);
    assert_eq!(snapshot.traffic.upload_speed, 0);
    assert_eq!(snapshot.traffic.download_speed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_active_profile_settles_traffic_baseline_before_next_profile() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-profile-delete-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content(
            "First",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();

    {
        let mut state = core.lock_state().unwrap();
        state.profiles.set_active(&first_id).unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                tx_bytes: 100,
                rx_bytes: 200,
                ..TunStats::default()
            },
        )
        .unwrap();
        settle_traffic_before_profile_switch(
            &mut state,
            Some(&TunStats {
                tx_bytes: 150,
                rx_bytes: 260,
                ..TunStats::default()
            }),
        )
        .unwrap();
        state.profiles.delete_profile(&first_id).unwrap();
        state.profiles.set_active(&second_id).unwrap();
        apply_traffic_sample(
            &mut state,
            &TunStats {
                tx_bytes: 180,
                rx_bytes: 300,
                ..TunStats::default()
            },
        )
        .unwrap();
    }

    let snapshot = core.snapshot().unwrap();
    assert!(snapshot
        .profiles
        .iter()
        .all(|profile| profile.id != first_id));
    let second = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == second_id)
        .expect("second profile");
    assert_eq!(second.upload_bytes, 40);
    assert_eq!(second.download_bytes, 30);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn platform_stop_settles_meow_traffic_when_native_stats_are_unavailable() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-platform-stop-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Direct",
            "test",
            &hmeta_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    core.set_platform_vpn_running(true).unwrap();
    let tunnel = {
        let state = core.lock_state().unwrap();
        state.tunnel.clone().expect("loaded tunnel")
    };
    tunnel.statistics().add_upload(320);
    tunnel.statistics().add_download(640);

    core.set_platform_vpn_running(false).unwrap();
    let snapshot = core.snapshot().unwrap();
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .expect("profile summary");

    assert_eq!(profile.upload_bytes, 320);
    assert_eq!(profile.download_bytes, 640);
    assert_eq!(snapshot.traffic.upload_bytes, 320);
    assert_eq!(snapshot.traffic.download_bytes, 640);
    assert_eq!(snapshot.traffic.upload_speed, 0);
    assert_eq!(snapshot.traffic.download_speed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_starts_meow_external_controller() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-controller-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let core = CoreHandle::new_with_profile_root_and_controller(root, addr);
    let yaml = format!(
        r#"mixed-port: 7890
external-controller: {addr}
proxies:
  - name: HTTP-MOCK
    type: http
    server: 127.0.0.1
    port: 18080
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
      - HTTP-MOCK
  - name: Auto
    type: url-test
    proxies:
      - DIRECT
    url: https://www.gstatic.com/generate_204
    interval: 3600
rules:
  - MATCH,Proxy
"#
    );
    let profile_id = core
        .import_profile_from_content("Direct", "test", &yaml, None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.controller_running);
    let addr_string = addr.to_string();
    assert_eq!(
        snapshot.controller_addr.as_deref(),
        Some(addr_string.as_str())
    );

    let version = wait_for_json(&format!("http://{addr}/version")).await;
    assert_eq!(
        version.get("meta").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
    assert!(proxies
        .get("proxies")
        .and_then(|value| value.get("DIRECT"))
        .is_some());

    core.select_proxy_via_controller("Proxy", "HTTP-MOCK")
        .await
        .unwrap();
    let snapshot = core.snapshot().unwrap();
    assert_eq!(
        snapshot.profiles[0]
            .selected_proxies
            .get("Proxy")
            .map(String::as_str),
        Some("HTTP-MOCK")
    );
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .unwrap();
    assert_eq!(proxy_group.selected.as_deref(), Some("HTTP-MOCK"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "HTTP-MOCK" && proxy.selected));
    core.select_proxy_via_controller("Auto", "DIRECT")
        .await
        .unwrap();
    let auto_group = core
        .snapshot()
        .unwrap()
        .proxy_groups
        .into_iter()
        .find(|group| group.name == "Auto")
        .expect("URLTest group");
    assert_eq!(auto_group.fixed.as_deref(), Some("DIRECT"));
    core.unfix_proxy_via_controller("Auto").await.unwrap();
    let auto_group = core
        .snapshot()
        .unwrap()
        .proxy_groups
        .into_iter()
        .find(|group| group.name == "Auto")
        .expect("URLTest group");
    assert_eq!(auto_group.fixed.as_deref(), Some(""));
    let rules = wait_for_json(&format!("http://{addr}/rules")).await;
    assert!(rules
        .get("rules")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|rules| !rules.is_empty()));
    let health_url = spawn_healthcheck_http_server().await;
    let delay = core
        .test_proxy_delay_via_controller("DIRECT", Some(&health_url), Some(1000))
        .await
        .unwrap();
    assert!(delay > 0);
    let proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
    assert!(proxies
        .get("proxies")
        .and_then(|value| value.get("DIRECT"))
        .and_then(|value| value.get("history"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|history| !history.is_empty()));

    let group_health_url = spawn_healthcheck_http_server().await;
    let group_delays = core
        .test_proxy_group_via_controller("Auto", Some(&group_health_url), Some(1000))
        .await
        .unwrap();
    assert!(group_delays.get("DIRECT").is_some_and(|delay| *delay > 0));
    core.flush_dns_cache_via_controller().await.unwrap();
    core.flush_fake_ip_cache_via_controller().await.unwrap();

    let memory = wait_for_first_json_frame(&format!("ws://{addr}/memory")).await;
    assert!(memory.get("inuse").is_some_and(serde_json::Value::is_u64));
    assert!(memory.get("oslimit").is_some_and(serde_json::Value::is_u64));

    let tunnel = {
        let state = core.lock_state().unwrap();
        state.tunnel.clone().expect("loaded tunnel")
    };
    tunnel.statistics().add_upload(64);
    tunnel.statistics().add_download(96);
    let connection_id = track_test_connection(&tunnel, "api.example.test");
    let traffic = wait_for_traffic_frame(&format!("ws://{addr}/traffic"), 64, 96).await;
    assert_eq!(
        traffic.get("up").and_then(serde_json::Value::as_i64),
        Some(64)
    );
    assert_eq!(
        traffic.get("down").and_then(serde_json::Value::as_i64),
        Some(96)
    );
    let connections = wait_for_json(&format!("http://{addr}/connections")).await;
    assert_eq!(
        connections
            .get("connections")
            .and_then(serde_json::Value::as_array)
            .and_then(|connections| connections.first())
            .and_then(|connection| connection.get("id"))
            .and_then(serde_json::Value::as_str),
        Some(connection_id.as_str())
    );
    core.close_connection_via_controller(&connection_id)
        .await
        .unwrap();
    assert!(tunnel.statistics().active_connections().is_empty());
    let first = track_test_connection(&tunnel, "first-api.example.test");
    let second = track_test_connection(&tunnel, "second-api.example.test");
    assert!(tunnel
        .statistics()
        .active_connections()
        .iter()
        .any(|connection| connection.id.to_string() == first));
    assert!(tunnel
        .statistics()
        .active_connections()
        .iter()
        .any(|connection| connection.id.to_string() == second));
    core.close_all_connections_via_controller().await.unwrap();
    assert!(tunnel.statistics().active_connections().is_empty());
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot
        .logs
        .iter()
        .any(|log| log.message == format!("connection closed via meow API: {connection_id}")));
    assert!(snapshot
        .logs
        .iter()
        .any(|log| log.message == "all connections closed via meow API: 2"));

    let warning = format!(
        "hmeta controller ws log test {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/logs?level=warning"))
        .await
        .unwrap();
    tracing::warn!(target: "hmeta_core_controller_test", "{}", warning);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
    let mut matched = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(frame) = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("logs websocket frame")
        else {
            break;
        };
        let frame = frame
            .expect("logs websocket receive")
            .into_text()
            .expect("text frame");
        let log: serde_json::Value = serde_json::from_str(&frame).unwrap();
        if log
            .get("payload")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|payload| payload.contains(&warning))
        {
            assert_eq!(
                log.get("type").and_then(serde_json::Value::as_str),
                Some("warning")
            );
            matched = true;
            break;
        }
    }
    assert!(matched, "logs websocket did not receive warning payload");

    core.set_profile_network_config(
        &profile_id,
        NetworkPortConfig {
            mixed_port: 17890,
            controller_port: 19090,
        },
        true,
    )
    .await
    .unwrap();
    let snapshot = core.snapshot().unwrap();
    let secret = snapshot
        .controller_access
        .secret
        .clone()
        .expect("LAN controller secret");
    assert!(snapshot.controller_running);
    assert!(snapshot.controller_access.allow_lan);
    assert_eq!(
        snapshot.network_ports,
        NetworkPortConfig {
            mixed_port: 17890,
            controller_port: 19090,
        }
    );
    assert_eq!(
        snapshot.controller_access.secret.as_deref(),
        Some(secret.as_str())
    );
    let unauthenticated = reqwest::get(format!("http://{addr}/version"))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let authenticated = wait_for_json_with_bearer(&format!("http://{addr}/version"), &secret).await;
    assert_eq!(
        authenticated
            .get("meta")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    core.flush_dns_cache_via_controller().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_controller_config_reload_converges_profile_and_native_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-controller-sync-test-{}",
        now_unix_nanos()
    ));
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let core = CoreHandle::new_with_profile_root_and_controller(root, addr);
    let original = format!(
        r#"mixed-port: 7890
hmeta:
  vpn:
    mtu: 1410
proxies:
  - name: HTTP-OLD
    type: http
    server: 127.0.0.1
    port: 18080
proxy-groups:
  - name: OldProxy
    type: select
    proxies: [DIRECT, HTTP-OLD]
rules:
  - MATCH,OldProxy
"#
    );
    let profile_id = core
        .import_profile_from_content("Controller sync", "test", &original, None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let _ = wait_for_json(&format!("http://{addr}/version")).await;
    let mut fds = [0_i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let options_json = to_json(&VpnOptions::default()).unwrap();
    core.start_vpn(fds[0], &options_json).await.unwrap();
    assert_eq!(core.vpn.fd(), Some(fds[0]));

    let replacement = r#"mode: direct
proxies:
  - name: HTTP-NEW
    type: http
    server: 127.0.0.1
    port: 18081
proxy-groups:
  - name: NewProxy
    type: select
    proxies: [DIRECT, HTTP-NEW]
rules:
  - MATCH,NewProxy
"#;
    let payload = base64::engine::general_purpose::STANDARD.encode(replacement);
    let response = reqwest::Client::new()
        .put(format!("http://{addr}/configs"))
        .json(&serde_json::json!({ "payload": payload }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    assert!(core.sync_external_controller_config().await.unwrap());
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.vpn_running);
    assert_eq!(core.vpn.fd(), Some(fds[0]));
    assert_eq!(snapshot.mode, RuntimeMode::Direct);
    assert!(snapshot
        .proxy_groups
        .iter()
        .any(|group| group.name == "NewProxy"));
    assert!(!snapshot
        .proxy_groups
        .iter()
        .any(|group| group.name == "OldProxy"));
    assert!(snapshot
        .rules
        .iter()
        .any(|rule| rule.line == "MATCH,NewProxy"));
    assert_eq!(snapshot.controller_diagnostics.config_sync_count, 1);
    assert!(snapshot
        .controller_diagnostics
        .last_config_sync_at
        .is_some());
    assert!(snapshot
        .controller_diagnostics
        .last_config_sync_error
        .is_none());
    let controller_proxies = wait_for_json(&format!("http://{addr}/proxies")).await;
    assert!(controller_proxies
        .get("proxies")
        .and_then(|proxies| proxies.get("NewProxy"))
        .is_some());

    let persisted = core.profile_raw_yaml(&profile_id).unwrap();
    assert!(persisted.contains("HTTP-NEW"));
    assert!(!persisted.contains("HTTP-OLD"));
    assert!(persisted.contains("hmeta:"));
    assert!(persisted.contains("mtu: 1410"));
    assert!(!persisted.contains("external-controller:"));
    core.stop_vpn().unwrap();
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_exposes_loaded_provider_registries() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-provider-controller-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let import_provider_path = root.join("import-provider.yaml");
    std::fs::write(&import_provider_path, provider_proxy_yaml()).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let core = CoreHandle::new_with_profile_root_and_controller(root.clone(), addr);
    let profile_id = core
        .import_profile_from_content(
            "Provider",
            "test",
            &provider_profile_yaml(&import_provider_path),
            None,
        )
        .await
        .unwrap();
    let runtime_provider_dir = root.join("providers/proxy").join(&profile_id);
    std::fs::create_dir_all(&runtime_provider_dir).unwrap();
    std::fs::write(
        runtime_provider_dir.join("LocalProxyProvider.yaml"),
        provider_proxy_yaml(),
    )
    .unwrap();

    core.reload_config(&profile_id).await.unwrap();

    let proxy_providers = wait_for_json(&format!("http://{addr}/providers/proxies")).await;
    assert_eq!(
        proxy_providers
            .get("providers")
            .and_then(|providers| providers.get("LocalProxyProvider"))
            .and_then(|provider| provider.get("proxies"))
            .and_then(serde_json::Value::as_array)
            .and_then(|proxies| proxies.first())
            .and_then(|proxy| proxy.get("name"))
            .and_then(serde_json::Value::as_str),
        Some("PROVIDER-HTTP")
    );

    let rule_providers = wait_for_json(&format!("http://{addr}/providers/rules")).await;
    assert_eq!(
        rule_providers
            .get("providers")
            .and_then(|providers| providers.get("LocalRuleProvider"))
            .and_then(|provider| provider.get("ruleCount"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );

    core.refresh_provider("LocalProxyProvider").await.unwrap();
    {
        let state = core.lock_state().unwrap();
        assert!(state.logs.iter().any(|log| {
            log.level == "info"
                && log
                    .message
                    .contains("proxy provider refreshed via meow API: LocalProxyProvider")
        }));
    }
    let snapshot = core.snapshot().unwrap();
    let proxy_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "LocalProxyProvider")
        .expect("LocalProxyProvider summary");
    assert_eq!(
        proxy_provider.path.as_deref(),
        Some(
            runtime_provider_dir
                .join("LocalProxyProvider.yaml")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(proxy_provider.cache_exists);
    assert!(proxy_provider.cache_bytes.is_some_and(|bytes| bytes > 0));
    assert!(proxy_provider.cache_updated_at.is_some());
    assert!(proxy_provider.last_refresh_at.is_some());
    assert!(proxy_provider.last_refresh_error.is_none());
    assert_eq!(proxy_provider.members.len(), 1);
    assert_eq!(proxy_provider.members[0].name, "PROVIDER-HTTP");

    core.healthcheck_proxy_provider_via_controller("LocalProxyProvider")
        .await
        .unwrap();
    let health_url = spawn_healthcheck_http_server().await;
    let error = core
        .healthcheck_provider_proxy_via_controller(
            "LocalProxyProvider",
            "PROVIDER-HTTP",
            &health_url,
            Some(1000),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("HTTP 503"));
    let snapshot = core.snapshot().unwrap();
    let proxy_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "LocalProxyProvider")
        .expect("LocalProxyProvider summary after health check");
    assert!(!proxy_provider.members[0].alive);
    assert_eq!(proxy_provider.members[0].delay_ms, Some(0));

    core.refresh_all_providers().await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let proxy_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "LocalProxyProvider")
        .expect("LocalProxyProvider summary after refresh all");
    let rule_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "LocalRuleProvider")
        .expect("LocalRuleProvider summary after refresh all");
    assert!(proxy_provider.last_refresh_at.is_some());
    assert!(proxy_provider.last_refresh_error.is_none());
    assert!(rule_provider.last_refresh_at.is_none());
    assert!(rule_provider.last_refresh_error.is_none());
    let state = core.lock_state().unwrap();
    assert!(state.logs.iter().any(|log| {
        log.level == "info"
            && log
                .message
                .contains("provider refresh all finished: 1 succeeded, 0 failed")
    }));
    drop(state);

    {
        let mut state = core.lock_state().unwrap();
        state.providers.push(ProviderSummary {
            name: "BrokenProvider".to_owned(),
            provider_type: "broken".to_owned(),
            path: None,
            url: None,
            vehicle_type: None,
            interval_seconds: None,
            filter: None,
            exclude_filter: None,
            behavior: None,
            format: None,
            health_check_enabled: false,
            health_check_url: None,
            health_check_interval_seconds: None,
            expected_status: None,
            members: Vec::new(),
            cache_exists: false,
            cache_bytes: None,
            cache_updated_at: None,
            stale_cache_available: false,
            last_refresh_at: None,
            last_refresh_error: None,
        });
    }
    let err = core.refresh_provider("BrokenProvider").await.unwrap_err();
    assert!(err.to_string().contains("unknown provider type"));
    let snapshot = core.snapshot().unwrap();
    let broken_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "BrokenProvider")
        .expect("BrokenProvider summary");
    assert!(broken_provider.last_refresh_at.is_some());
    assert!(broken_provider
        .last_refresh_error
        .as_deref()
        .unwrap_or_default()
        .contains("unknown provider type"));

    let err = core.refresh_provider("MissingProvider").await.unwrap_err();
    assert!(err.to_string().contains("provider not found"));
    let state = core.lock_state().unwrap();
    assert!(state.logs.iter().any(|log| {
        log.level == "warning"
            && log
                .message
                .contains("provider refresh failed: provider not found: MissingProvider")
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_refresh_disambiguates_same_name_by_type() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-provider-duplicate-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let import_provider_path = root.join("import-provider.yaml");
    std::fs::write(&import_provider_path, provider_proxy_yaml()).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let core = CoreHandle::new_with_profile_root_and_controller(root.clone(), addr);
    let profile_id = core
        .import_profile_from_content(
            "Duplicate Providers",
            "test",
            &duplicate_provider_profile_yaml(&import_provider_path),
            None,
        )
        .await
        .unwrap();
    let runtime_provider_dir = root.join("providers/proxy").join(&profile_id);
    std::fs::create_dir_all(&runtime_provider_dir).unwrap();
    std::fs::write(
        runtime_provider_dir.join("Shared.yaml"),
        provider_proxy_yaml(),
    )
    .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let _ = wait_for_json(&format!("http://{addr}/providers/rules")).await;

    let err = core
        .refresh_provider_of_type("rule", "Shared")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("is inline"));
    let snapshot = core.snapshot().unwrap();
    let rule_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "Shared" && provider.provider_type == "rule")
        .expect("rule provider");
    let proxy_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "Shared" && provider.provider_type == "proxy")
        .expect("proxy provider");
    assert!(rule_provider.last_refresh_at.is_some());
    assert!(rule_provider
        .last_refresh_error
        .as_deref()
        .is_some_and(|error| error.contains("is inline")));
    assert!(proxy_provider.last_refresh_at.is_none());
    let _ = wait_for_json(&format!("http://{addr}/providers/rules")).await;

    core.refresh_all_providers().await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let rule_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "Shared" && provider.provider_type == "rule")
        .expect("rule provider after refresh all");
    let proxy_provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "Shared" && provider.provider_type == "proxy")
        .expect("proxy provider after refresh all");
    assert!(rule_provider
        .last_refresh_error
        .as_deref()
        .is_some_and(|error| error.contains("is inline")));
    assert!(proxy_provider.last_refresh_at.is_some());
    assert!(proxy_provider.last_refresh_error.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_rule_provider_runtime_cache_fields_do_not_break_reload() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-inline-rule-provider-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content(
            "Inline Rule Provider",
            "test",
            r#"
mixed-port: 7890
mode: rule
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
        .await
        .unwrap();

    core.reload_config(&profile_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.engine_loaded);
    let provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "InlineRules" && provider.provider_type == "rule")
        .expect("inline rule provider");
    assert!(provider.path.is_none());
    assert!(provider.interval_seconds.is_none());
    assert_eq!(provider.behavior.as_deref(), Some("classical"));
}

#[test]
fn provider_refresh_failure_marks_stale_cache_available() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-provider-stale-cache-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let cache_path = root.join("providers/proxy/default/StaleProvider.yaml");
    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    std::fs::write(&cache_path, provider_proxy_yaml()).unwrap();

    let core = CoreHandle::new_with_profile_root(root.join("store"));
    let mut state = core.lock_state().unwrap();
    state.providers.push(ProviderSummary {
        name: "StaleProvider".to_owned(),
        provider_type: "proxy".to_owned(),
        path: Some(cache_path.to_string_lossy().into_owned()),
        url: Some("http://127.0.0.1:9/provider.yaml".to_owned()),
        vehicle_type: Some("http".to_owned()),
        interval_seconds: None,
        filter: None,
        exclude_filter: None,
        behavior: None,
        format: None,
        health_check_enabled: false,
        health_check_url: None,
        health_check_interval_seconds: None,
        expected_status: None,
        members: Vec::new(),
        cache_exists: false,
        cache_bytes: None,
        cache_updated_at: None,
        stale_cache_available: false,
        last_refresh_at: None,
        last_refresh_error: None,
    });

    mark_provider_refresh(
        &mut state,
        "proxy",
        "StaleProvider",
        "12345".to_owned(),
        Some("refresh failed".to_owned()),
    );

    let provider = state.providers.first().expect("provider summary");
    assert!(provider.cache_exists);
    assert!(provider.cache_bytes.is_some_and(|bytes| bytes > 0));
    assert!(provider.cache_updated_at.is_some());
    assert!(provider.stale_cache_available);
    assert_eq!(provider.last_refresh_at.as_deref(), Some("12345"));
    assert_eq!(
        provider.last_refresh_error.as_deref(),
        Some("refresh failed")
    );
    assert_eq!(
        provider_refresh_failure_log_message("refresh failed", provider.stale_cache_available),
        "refresh failed; stale provider cache retained"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_all_providers_reports_empty_provider_set() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-provider-empty-refresh-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    core.refresh_all_providers().await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.logs.iter().any(|log| {
        log.level == "info" && log.message == "provider refresh skipped: no refreshable providers"
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_all_providers_skips_inline_only_provider_set() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-provider-inline-refresh-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    {
        let mut state = core.lock_state().unwrap();
        state.providers.push(ProviderSummary {
            name: "InlineRules".to_owned(),
            provider_type: "rule".to_owned(),
            path: None,
            url: None,
            vehicle_type: Some("inline".to_owned()),
            interval_seconds: None,
            filter: None,
            exclude_filter: None,
            behavior: Some("classical".to_owned()),
            format: None,
            health_check_enabled: false,
            health_check_url: None,
            health_check_interval_seconds: None,
            expected_status: None,
            members: Vec::new(),
            cache_exists: false,
            cache_bytes: None,
            cache_updated_at: None,
            stale_cache_available: false,
            last_refresh_at: None,
            last_refresh_error: None,
        });
    }

    core.refresh_all_providers().await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.name == "InlineRules")
        .expect("inline provider");
    assert!(provider.last_refresh_at.is_none());
    assert!(provider.last_refresh_error.is_none());
    assert!(snapshot.logs.iter().any(|log| {
        log.level == "info" && log.message == "provider refresh skipped: no refreshable providers"
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_proxy_and_global_node_are_restored_after_reload() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-selected-proxy-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let id = core
        .import_profile_from_content("http", "local-file", &local_protocol_profile("http"), None)
        .await
        .unwrap();

    core.reload_config(&id).await.unwrap();
    let order_before_selection = core
        .snapshot()
        .unwrap()
        .proxy_groups
        .into_iter()
        .find(|group| group.name == "Proxy")
        .unwrap()
        .proxies
        .into_iter()
        .map(|proxy| proxy.name)
        .collect::<Vec<_>>();
    core.select_proxy("Proxy", "DIRECT").await.unwrap();
    core.select_proxy("GLOBAL", "HTTP-MOCK").await.unwrap();
    core.reload_config(&id).await.unwrap();

    let snapshot = core.snapshot().unwrap();
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert_eq!(proxy_group.selected.as_deref(), Some("DIRECT"));
    assert_eq!(
        proxy_group
            .proxies
            .iter()
            .map(|proxy| proxy.name.as_str())
            .collect::<Vec<_>>(),
        order_before_selection
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        snapshot.profiles[0]
            .selected_proxies
            .get("Proxy")
            .map(String::as_str),
        Some("DIRECT")
    );
    let global = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "GLOBAL")
        .expect("GLOBAL selector");
    assert_eq!(global.selected.as_deref(), Some("HTTP-MOCK"));
    assert_eq!(
        snapshot.profiles[0]
            .selected_proxies
            .get("GLOBAL")
            .map(String::as_str),
        Some("HTTP-MOCK")
    );

    core.set_mode(RuntimeMode::Global).unwrap();
    assert_eq!(
        core.snapshot()
            .unwrap()
            .proxy_groups
            .iter()
            .find(|group| group.name == "GLOBAL")
            .and_then(|group| group.selected.as_deref()),
        Some("HTTP-MOCK"),
        "Global mode must use the saved selected subscription node"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_group_pins_and_auto_mode_persist_across_reload() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-automatic-group-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let yaml = r#"
proxy-groups:
  - name: Auto
    type: url-test
    proxies: [DIRECT]
    url: https://www.gstatic.com/generate_204
    interval: 3600
  - name: Backup
    type: fallback
    proxies: [DIRECT]
    url: https://www.gstatic.com/generate_204
    interval: 3600
rules:
  - MATCH,Auto
"#;
    let profile_id = core
        .import_profile_from_content("Automatic", "test", yaml, None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    for group_name in ["Auto", "Backup"] {
        core.select_proxy(group_name, "DIRECT").await.unwrap();
        let group = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == group_name)
            .expect("automatic group");
        assert_eq!(group.fixed.as_deref(), Some("DIRECT"));

        core.unfix_proxy(group_name).unwrap();
        let snapshot = core.snapshot().unwrap();
        let group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == group_name)
            .expect("automatic group");
        assert_eq!(group.fixed.as_deref(), Some(""));
        assert_eq!(
            snapshot.profiles[0]
                .selected_proxies
                .get(group_name)
                .map(String::as_str),
            Some("")
        );
    }

    core.reload_config(&profile_id).await.unwrap();
    for group_name in ["Auto", "Backup"] {
        let group = core
            .snapshot()
            .unwrap()
            .proxy_groups
            .into_iter()
            .find(|group| group.name == group_name)
            .expect("restored automatic group");
        assert_eq!(group.fixed.as_deref(), Some(""));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_retains_sniffer_config_for_harmony_tun() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-sniffer-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let yaml = r#"
sniffer:
  enable: true
  timeout: 250
  parse-pure-ip: true
  override-destination: true
  sniff:
    TLS:
      ports: [443, 8443]
    HTTP:
      ports: [80, 8080]
proxy-groups:
  - name: Proxy
    type: select
    proxies: [DIRECT]
rules:
  - MATCH,Proxy
"#;
    let profile_id = core
        .import_profile_from_content("Sniffer", "test", yaml, None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let config = core.lock_state().unwrap().sniffer_config.clone();
    assert!(config.enable);
    assert_eq!(config.timeout, std::time::Duration::from_millis(250));
    assert!(config.override_destination);
    assert_eq!(config.tls_ports, vec![443, 8443]);
    assert_eq!(config.http_ports, vec![80, 8080]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_edit_and_backup_restore_reload_active_tunnel() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-profile-edit-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let id = core
        .import_profile_from_content("http", "local-file", &local_protocol_profile("http"), None)
        .await
        .unwrap();

    core.reload_config(&id).await.unwrap();
    let original_yaml = core.profile_raw_yaml(&id).unwrap();
    let invalid = core.update_profile_content(&id, "proxy-groups: [").await;
    assert!(invalid.is_err());
    assert_eq!(core.profile_raw_yaml(&id).unwrap(), original_yaml);
    assert!(core
        .validate_profile_content(&local_protocol_profile("direct"))
        .await
        .is_ok());
    assert!(core
        .validate_profile_content("proxy-groups: [")
        .await
        .is_err());

    core.update_profile_content(&id, &local_protocol_profile("direct"))
        .await
        .unwrap();
    let snapshot = core.snapshot().unwrap();
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "DIRECT"));

    core.restore_profile_backup(&id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "HTTP-MOCK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_active_profile_reloads_next_or_clears_engine() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-profile-delete-active-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let direct_id = core
        .import_profile_from_content(
            "direct",
            "local-file",
            &local_protocol_profile("direct"),
            None,
        )
        .await
        .unwrap();
    let http_id = core
        .import_profile_from_content("http", "local-file", &local_protocol_profile("http"), None)
        .await
        .unwrap();

    core.reload_config(&direct_id).await.unwrap();
    assert_eq!(
        core.snapshot().unwrap().active_profile.as_deref(),
        Some(direct_id.as_str())
    );

    core.delete_profile(&direct_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.active_profile.as_deref(), Some(http_id.as_str()));
    assert!(snapshot.engine_loaded);
    assert!(snapshot
        .proxy_groups
        .iter()
        .any(|group| group.proxies.iter().any(|proxy| proxy.name == "HTTP-MOCK")));

    core.delete_profile(&http_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.active_profile.is_none());
    assert!(!snapshot.engine_loaded);
    assert!(snapshot.proxy_groups.is_empty());
    assert!(snapshot.providers.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_all_profiles_continues_after_single_failure() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-refresh-all-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let (good_url, bad_url) =
        spawn_profile_refresh_http_server(local_protocol_profile("direct")).await;
    let good_id = core
        .import_profile_from_content(
            "good",
            &good_url,
            &local_protocol_profile("http"),
            Some(good_url.clone()),
        )
        .await
        .unwrap();
    let bad_id = core
        .import_profile_from_content(
            "bad",
            &bad_url,
            &local_protocol_profile("http"),
            Some(bad_url.clone()),
        )
        .await
        .unwrap();

    core.reload_config(&good_id).await.unwrap();
    core.refresh_all_profiles().await.unwrap();

    let good_yaml = core.profile_raw_yaml(&good_id).unwrap();
    let bad_yaml = core.profile_raw_yaml(&bad_id).unwrap();
    assert!(!good_yaml.contains("HTTP-MOCK"));
    assert!(good_yaml.contains("MATCH,DIRECT"));
    assert!(bad_yaml.contains("HTTP-MOCK"));

    let snapshot = core.snapshot().unwrap();
    let good_profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == good_id)
        .expect("good profile summary");
    assert!(good_profile.last_refresh_at.is_some());
    assert!(good_profile.last_refresh_error.is_none());
    let bad_profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == bad_id)
        .expect("bad profile summary");
    assert!(bad_profile.last_refresh_at.is_some());
    assert!(bad_profile
        .last_refresh_error
        .as_deref()
        .unwrap_or_default()
        .contains("profile refresh failed"));
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "DIRECT"));
    assert!(snapshot.logs.iter().any(|log| {
        log.level == "warning" && log.message.contains("profile refresh failed: bad")
    }));
    assert!(snapshot.logs.iter().any(|log| {
        log.level == "info"
            && log
                .message
                .contains("profile refresh all finished: 1 succeeded, 1 failed")
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscription_userinfo_header_updates_profile_summary() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-sub-userinfo-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let url = spawn_subscription_userinfo_http_server(local_protocol_profile("direct")).await;

    let profile_id = core.import_profile_from_url(&url, None).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .expect("profile summary");
    assert_eq!(profile.name, "Remote Sub");
    let info = profile
        .subscription_user_info
        .as_ref()
        .expect("subscription userinfo");
    assert_eq!(info.upload_bytes, 100);
    assert_eq!(info.download_bytes, 200);
    assert_eq!(info.total_bytes, Some(1000));
    assert_eq!(info.expire_at.as_deref(), Some("1893456000"));
    let metadata = profile
        .subscription_metadata
        .as_ref()
        .expect("subscription metadata");
    assert_eq!(metadata.title.as_deref(), Some("Remote Sub"));
    assert_eq!(metadata.update_interval_hours, Some(12));
    assert_eq!(
        metadata.web_page_url.as_deref(),
        Some("https://example.test/portal")
    );
    assert_eq!(
        metadata.support_url.as_deref(),
        Some("https://example.test/support")
    );

    core.refresh_profile(&profile_id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .expect("profile summary after refresh");
    let info = profile
        .subscription_user_info
        .as_ref()
        .expect("subscription userinfo after refresh");
    assert_eq!(info.upload_bytes, 300);
    assert_eq!(info.download_bytes, 400);
    assert_eq!(info.total_bytes, Some(2000));
    assert_eq!(info.expire_at.as_deref(), Some("1896048000"));
    let metadata = profile
        .subscription_metadata
        .as_ref()
        .expect("subscription metadata after refresh");
    assert_eq!(metadata.title.as_deref(), Some("Remote Sub Updated"));
    assert_eq!(metadata.update_interval_hours, Some(24));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscription_metadata_comment_fills_missing_header_fields() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-sub-comment-metadata-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let body = format!(
            "{}\n{}",
            "# profile-title=Body%20Title; profile-update-interval=6; profile-web-page-url=https://example.test/body; support-url=https://example.test/help",
            local_protocol_profile("direct")
        );
    let url = spawn_subscription_metadata_comment_http_server(body).await;

    let profile_id = core.import_profile_from_url(&url, None).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .expect("profile summary");
    assert_eq!(profile.name, "Header Title");
    let metadata = profile
        .subscription_metadata
        .as_ref()
        .expect("subscription metadata");
    assert_eq!(metadata.title.as_deref(), Some("Header Title"));
    assert_eq!(metadata.update_interval_hours, Some(6));
    assert_eq!(
        metadata.web_page_url.as_deref(),
        Some("https://example.test/body")
    );
    assert_eq!(
        metadata.support_url.as_deref(),
        Some("https://example.test/help")
    );
}

#[test]
fn content_disposition_title_is_used_as_subscription_metadata_title() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "content-disposition",
        reqwest::header::HeaderValue::from_static(
            "attachment; filename*=UTF-8''%E8%BF%9C%E7%A8%8B.yaml",
        ),
    );
    headers.insert(
        "profile-update-interval",
        reqwest::header::HeaderValue::from_static("24"),
    );

    let metadata = subscription_metadata_from_headers(&headers).expect("metadata");
    assert_eq!(metadata.title.as_deref(), Some("远程.yaml"));
    assert_eq!(metadata.update_interval_hours, Some(24));
    assert_eq!(
        subscription_profile_name_from_headers(&headers).as_deref(),
        Some("远程.yaml")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feature_gated_proxy_types_are_loaded() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-protocol-feature-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile = r#"
mixed-port: 7890
mode: rule
log-level: info
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxies:
  - name: TROJAN-MOCK
    type: trojan
    server: 127.0.0.1
    port: 443
    password: test-trojan-password
    sni: localhost
    skip-cert-verify: true
    udp: false
  - name: VLESS-MOCK
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    tls: false
    udp: false
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - TROJAN-MOCK
      - VLESS-MOCK
      - DIRECT
rules:
  - MATCH,Proxy
"#;
    let id = core
        .import_profile_from_content("feature-gated", "test", profile, None)
        .await
        .unwrap();

    core.reload_config(&id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "TROJAN-MOCK"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-MOCK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_link_subscription_imports_before_meow_validation() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-share-subscription-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let links = "\
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&security=none#VLESS-MOCK
trojan://test-trojan-password@127.0.0.1:443?sni=localhost&allowInsecure=1#TROJAN-MOCK
";
    let encoded = base64::engine::general_purpose::STANDARD.encode(links);
    let id = core
        .import_profile_from_content(
            "share-subscription",
            "https://example.test/sub",
            &encoded,
            Some("https://example.test/sub".to_owned()),
        )
        .await
        .unwrap();

    core.reload_config(&id).await.unwrap();
    let snapshot = core.snapshot().unwrap();
    let proxy_group = snapshot
        .proxy_groups
        .iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-MOCK"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "TROJAN-MOCK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_link_transport_options_reload_with_meow_config() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-share-transport-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let links = "\
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=ws&security=tls&sni=localhost&host=localhost&path=%2Fws&client-fingerprint=chrome&alpn=h2%2Chttp%2F1.1&ed=2048&eh=Sec-WebSocket-Protocol&tfo=1#VLESS-WS
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?network=ws&security=tls&serverName=localhost&wsHost=localhost&wsPath=%2Falias-ws&fingerprint=chrome&allow-insecure=allow#VLESS-WS-ALIAS
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=h2&security=tls&sni=localhost&host=localhost,alt.localhost&path=%2Fh2#VLESS-H2
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=httpupgrade&security=tls&sni=localhost&host=localhost&path=%2Fupgrade#VLESS-HTTPUPGRADE
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&security=tls&sni=localhost&flow=xtls-rprx-vision&allowInsecure=1#VLESS-VISION
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&tls=true&sni=localhost#VLESS-TLS-QUERY
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&encryption=none#VLESS-ENCRYPTION-NONE
vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:443?type=tcp&udp=false#VLESS-UDP-OFF
trojan://test-trojan-password@127.0.0.1:443?type=grpc&serviceName=svc&sni=localhost&allowInsecure=1&fast-open=true#TROJAN-GRPC
trojan://test-trojan-password@127.0.0.1:443?type=grpc&grpc-service-name=alias-svc&grpc-mode=gun&serverName=localhost&allow-insecure=allow#TROJAN-GRPC-ALIAS
http://user:pass@127.0.0.1:8080?headers=User-Agent%3DHMeta%3BProxy-Authorization%3DBearer%20token#HTTP-SHARE
socks5://sock:sockpass@127.0.0.1:1080?tls=true&skip-cert-verify=true&udp=true&fastOpen=true#SOCKS5-SHARE
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8388?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dlocalhost&TFO=true#SS-OBFS
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8389?plugin=v2ray-plugin%3Bmode%3Dwebsocket%3Bhost%3Dlocalhost%3Bpath%3D%2Fss-ws%3Btls#SS-V2RAY
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@127.0.0.1:8390?plugin=Simple-Obfs%3Bobfs%3Dhttp%3Bobfs-host%3Dlocalhost#SS-OBFS-CASE
";
    let id = core
        .import_profile_from_content("share-transport", "clipboard", links, None)
        .await
        .unwrap();

    core.reload_config(&id).await.unwrap();
    let proxy_group = core
        .snapshot()
        .unwrap()
        .proxy_groups
        .into_iter()
        .find(|group| group.name == "Proxy")
        .expect("Proxy group");
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-WS"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-WS-ALIAS"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-H2"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-HTTPUPGRADE"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-VISION"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-TLS-QUERY"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-ENCRYPTION-NONE"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "VLESS-UDP-OFF"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "TROJAN-GRPC"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "TROJAN-GRPC-ALIAS"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "HTTP-SHARE"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "SOCKS5-SHARE"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "SS-OBFS"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "SS-V2RAY"));
    assert!(proxy_group
        .proxies
        .iter()
        .any(|proxy| proxy.name == "SS-OBFS-CASE"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_local_protocol_profiles_import_and_populate_proxy_groups() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-generated-profiles-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let cases = [
        ("direct", "DIRECT"),
        ("http", "HTTP-MOCK"),
        ("http-auth", "HTTP-AUTH-MOCK"),
        ("http-bad-auth", "HTTP-BAD-AUTH-MOCK"),
        ("http-down", "HTTP-DOWN-MOCK"),
        ("socks5", "SOCKS5-MOCK"),
        ("socks5-auth", "SOCKS5-AUTH-MOCK"),
        ("socks5-bad-auth", "SOCKS5-BAD-AUTH-MOCK"),
        ("ss", "SS-MOCK"),
        ("ss-bad-password", "SS-BAD-PASSWORD-MOCK"),
        ("trojan", "TROJAN-MOCK"),
        ("trojan-bad-password", "TROJAN-BAD-PASSWORD-MOCK"),
        ("vless", "VLESS-MOCK"),
        ("vless-bad-uuid", "VLESS-BAD-UUID-MOCK"),
    ];

    for (mode, expected_proxy) in cases {
        let profile = local_protocol_profile(mode);
        let id = core
            .import_profile_from_content(mode, "local-file", &profile, None)
            .await
            .unwrap_or_else(|err| panic!("{mode} profile should import: {err}"));

        core.reload_config(&id)
            .await
            .unwrap_or_else(|err| panic!("{mode} profile should reload: {err}"));
        let snapshot = core.snapshot().unwrap();
        assert_eq!(snapshot.active_profile.as_deref(), Some(id.as_str()));
        assert!(snapshot.profiles.iter().any(|profile| profile.id == id));
        let proxy_group = snapshot
            .proxy_groups
            .iter()
            .find(|group| group.name == "Proxy")
            .unwrap_or_else(|| panic!("{mode} profile should expose Proxy group"));
        assert!(
            proxy_group
                .proxies
                .iter()
                .any(|proxy| proxy.name == expected_proxy),
            "{mode} profile should expose {expected_proxy}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shadowsocks_proxy_echo_roundtrip_and_bad_password_fails() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-ss-echo-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let echo_addr = spawn_tcp_echo_server().await;
    let ss_addr = spawn_shadowsocks_proxy().await;
    let good_profile =
        local_protocol_profile_with_ports("ss", "127.0.0.1", echo_addr.port(), ss_addr.port());
    let good_id = core
        .import_profile_from_content("ss", "local-file", &good_profile, None)
        .await
        .unwrap();
    core.reload_config(&good_id).await.unwrap();

    let delay = core
        .test_proxy_delay("SS-MOCK", Some(&format!("http://{echo_addr}")), Some(1000))
        .await
        .unwrap();
    assert!(delay < 1000);
    let echoed = core
        .test_proxy_echo(
            "SS-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-ss-echo",
            Some(1000),
        )
        .await
        .unwrap();
    assert_eq!(echoed, "hmeta-ss-echo");

    let bad_profile = local_protocol_profile_with_ports(
        "ss-bad-password",
        "127.0.0.1",
        echo_addr.port(),
        ss_addr.port(),
    );
    let bad_id = core
        .import_profile_from_content("ss-bad-password", "local-file", &bad_profile, None)
        .await
        .unwrap();
    core.reload_config(&bad_id).await.unwrap();

    let err = core
        .test_proxy_echo(
            "SS-BAD-PASSWORD-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-ss-echo",
            Some(300),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("echo test"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_and_socks5_auth_echo_roundtrip_and_bad_credentials_fail() {
    let root = std::env::temp_dir().join(format!(
        "hmeta-core-http-socks-echo-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let echo_addr = spawn_tcp_echo_server().await;

    let http_addr =
        spawn_http_connect_proxy(Some("Proxy-Authorization: Basic YWxpY2U6czNjcjN0")).await;
    let http_profile = local_protocol_profile_with_ports(
        "http-auth",
        "127.0.0.1",
        echo_addr.port(),
        http_addr.port(),
    );
    let http_id = core
        .import_profile_from_content("http-auth", "local-file", &http_profile, None)
        .await
        .unwrap();
    core.reload_config(&http_id).await.unwrap();
    assert!(
        core.test_proxy_delay(
            "HTTP-AUTH-MOCK",
            Some(&format!("http://{echo_addr}")),
            Some(1000)
        )
        .await
        .unwrap()
            < 1000
    );
    assert_eq!(
        core.test_proxy_echo(
            "HTTP-AUTH-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-http-echo",
            Some(1000),
        )
        .await
        .unwrap(),
        "hmeta-http-echo"
    );

    let bad_http_profile = local_protocol_profile_with_ports(
        "http-bad-auth",
        "127.0.0.1",
        echo_addr.port(),
        http_addr.port(),
    );
    let bad_http_id = core
        .import_profile_from_content("http-bad-auth", "local-file", &bad_http_profile, None)
        .await
        .unwrap();
    core.reload_config(&bad_http_id).await.unwrap();
    let err = core
        .test_proxy_echo(
            "HTTP-BAD-AUTH-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-http-echo",
            Some(300),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("echo test"));

    let socks_addr = spawn_socks5_proxy(Some((b"bob", b"hunter2"))).await;
    let socks_profile = local_protocol_profile_with_ports(
        "socks5-auth",
        "127.0.0.1",
        echo_addr.port(),
        socks_addr.port(),
    );
    let socks_id = core
        .import_profile_from_content("socks5-auth", "local-file", &socks_profile, None)
        .await
        .unwrap();
    core.reload_config(&socks_id).await.unwrap();
    assert!(
        core.test_proxy_delay(
            "SOCKS5-AUTH-MOCK",
            Some(&format!("http://{echo_addr}")),
            Some(1000)
        )
        .await
        .unwrap()
            < 1000
    );
    assert_eq!(
        core.test_proxy_echo(
            "SOCKS5-AUTH-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-socks-echo",
            Some(1000),
        )
        .await
        .unwrap(),
        "hmeta-socks-echo"
    );

    let bad_socks_profile = local_protocol_profile_with_ports(
        "socks5-bad-auth",
        "127.0.0.1",
        echo_addr.port(),
        socks_addr.port(),
    );
    let bad_socks_id = core
        .import_profile_from_content("socks5-bad-auth", "local-file", &bad_socks_profile, None)
        .await
        .unwrap();
    core.reload_config(&bad_socks_id).await.unwrap();
    let err = core
        .test_proxy_echo(
            "SOCKS5-BAD-AUTH-MOCK",
            &format!("http://{echo_addr}"),
            "hmeta-socks-echo",
            Some(300),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("echo test"));
}

fn local_protocol_profile(mode: &str) -> String {
    local_protocol_profile_with_ports(mode, "127.0.0.1", 58197, 58198)
}

fn local_protocol_profile_with_ports(
    mode: &str,
    host: &str,
    echo_port: u16,
    proxy_port: u16,
) -> String {
    let template_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../local-protocol-tests/profiles")
        .join(format!("{mode}.yaml.in"));
    std::fs::read_to_string(template_path)
        .unwrap()
        .replace("{{HOST}}", host)
        .replace("{{ECHO_PORT}}", &echo_port.to_string())
        .replace("{{PROXY_PORT}}", &proxy_port.to_string())
}

async fn spawn_tcp_echo_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0_u8; 16 * 1024];
                loop {
                    let Ok(n) = stream.read(&mut buffer).await else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    if stream.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

async fn spawn_shadowsocks_proxy() -> SocketAddr {
    use shadowsocks::config::{ServerConfig, ServerType};
    use shadowsocks::context::Context;
    use shadowsocks::crypto::CipherKind;
    use shadowsocks::relay::socks5::Address;
    use shadowsocks::ProxyListener;

    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        "test-shadowsocks-password",
        CipherKind::AES_128_GCM,
    )
    .unwrap();
    let listener = ProxyListener::bind(Context::new_shared(ServerType::Server), &config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let target = match inbound.handshake().await {
                    Ok(Address::SocketAddress(addr)) => addr,
                    Ok(Address::DomainNameAddress(host, port)) => {
                        if host == "localhost" || host == "127.0.0.1" {
                            SocketAddr::from(([127, 0, 0, 1], port))
                        } else {
                            return;
                        }
                    }
                    Err(_) => return,
                };
                let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    addr
}

async fn spawn_http_connect_proxy(required_auth_header: Option<&'static str>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let Ok(request) = read_http_proxy_request(&mut inbound).await else {
                    return;
                };
                let first_line = request.lines().next().unwrap_or_default();
                let mut parts = first_line.split_whitespace();
                if !parts.next().is_some_and(|method| method == "CONNECT") {
                    let _ = inbound
                        .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                        .await;
                    return;
                }
                let Some(authority) = parts.next() else {
                    return;
                };
                if let Some(required) = required_auth_header {
                    if !request.contains(required)
                        || !request.contains("X-HMeta-Test: local-protocol")
                    {
                        let _ = inbound
                            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                            .await;
                        return;
                    }
                }
                let Some(target) = parse_local_authority(authority) else {
                    return;
                };
                let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let _ = inbound
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await;
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    addr
}

async fn read_http_proxy_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<String, std::io::Error> {
    let mut bytes = Vec::with_capacity(1024);
    let mut one = [0_u8; 1];
    while bytes.len() < 16 * 1024 {
        let n = stream.read(&mut one).await?;
        if n == 0 {
            break;
        }
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn spawn_socks5_proxy(required_auth: Option<(&'static [u8], &'static [u8])>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut greeting = [0_u8; 2];
                if inbound.read_exact(&mut greeting).await.is_err() || greeting[0] != 0x05 {
                    return;
                }
                let mut methods = vec![0_u8; greeting[1] as usize];
                if inbound.read_exact(&mut methods).await.is_err() {
                    return;
                }
                let method = if required_auth.is_some() {
                    0x02
                } else if methods.contains(&0x00) {
                    0x00
                } else {
                    0xff
                };
                if inbound.write_all(&[0x05, method]).await.is_err() || method == 0xff {
                    return;
                }
                if let Some((expected_user, expected_pass)) = required_auth {
                    if !read_socks5_auth(&mut inbound, expected_user, expected_pass).await {
                        return;
                    }
                }
                let Some(target) = read_socks5_connect_target(&mut inbound).await else {
                    return;
                };
                let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let _ = inbound
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await;
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    addr
}

async fn read_socks5_auth(
    inbound: &mut tokio::net::TcpStream,
    expected_user: &[u8],
    expected_pass: &[u8],
) -> bool {
    let mut auth_hdr = [0_u8; 2];
    if inbound.read_exact(&mut auth_hdr).await.is_err() || auth_hdr[0] != 0x01 {
        return false;
    }
    let mut user = vec![0_u8; auth_hdr[1] as usize];
    if inbound.read_exact(&mut user).await.is_err() {
        return false;
    }
    let mut pass_len = [0_u8; 1];
    if inbound.read_exact(&mut pass_len).await.is_err() {
        return false;
    }
    let mut pass = vec![0_u8; pass_len[0] as usize];
    if inbound.read_exact(&mut pass).await.is_err() {
        return false;
    }
    let ok = user == expected_user && pass == expected_pass;
    let _ = inbound
        .write_all(&[0x01, if ok { 0x00 } else { 0x01 }])
        .await;
    ok
}

async fn read_socks5_connect_target(inbound: &mut tokio::net::TcpStream) -> Option<SocketAddr> {
    let mut header = [0_u8; 4];
    if inbound.read_exact(&mut header).await.is_err() || header[0] != 0x05 || header[1] != 0x01 {
        return None;
    }
    match header[3] {
        0x01 => {
            let mut octets = [0_u8; 4];
            inbound.read_exact(&mut octets).await.ok()?;
            let port = read_u16(inbound).await?;
            Some(SocketAddr::from((octets, port)))
        }
        0x03 => {
            let mut len = [0_u8; 1];
            inbound.read_exact(&mut len).await.ok()?;
            let mut host = vec![0_u8; len[0] as usize];
            inbound.read_exact(&mut host).await.ok()?;
            let port = read_u16(inbound).await?;
            let host = String::from_utf8_lossy(&host);
            parse_local_authority(&format!("{host}:{port}"))
        }
        _ => None,
    }
}

async fn read_u16(inbound: &mut tokio::net::TcpStream) -> Option<u16> {
    let mut port = [0_u8; 2];
    inbound.read_exact(&mut port).await.ok()?;
    Some(u16::from_be_bytes(port))
}

fn parse_local_authority(authority: &str) -> Option<SocketAddr> {
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    match host.trim_matches(['[', ']']) {
        "localhost" | "127.0.0.1" => Some(SocketAddr::from(([127, 0, 0, 1], port))),
        "::1" => Some(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port))),
        _ => None,
    }
}

fn provider_proxy_yaml() -> &'static str {
    r#"
proxies:
  - name: PROVIDER-HTTP
    type: http
    server: 127.0.0.1
    port: 9
"#
}

fn provider_profile_yaml(import_provider_path: &std::path::Path) -> String {
    format!(
        r#"
mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxy-providers:
  LocalProxyProvider:
    type: file
    path: "{}"
rule-providers:
  LocalRuleProvider:
    type: inline
    behavior: classical
    payload:
      - DOMAIN-SUFFIX,provider.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - LocalProxyProvider
    proxies:
      - DIRECT
rules:
  - RULE-SET,LocalRuleProvider,DIRECT
  - MATCH,DIRECT
"#,
        import_provider_path.to_string_lossy()
    )
}

fn duplicate_provider_profile_yaml(import_provider_path: &std::path::Path) -> String {
    format!(
        r#"
mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  nameserver:
    - 1.1.1.1
proxy-providers:
  Shared:
    type: file
    path: "{}"
rule-providers:
  Shared:
    type: inline
    behavior: classical
    payload:
      - DOMAIN-SUFFIX,provider.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - Shared
    proxies:
      - DIRECT
rules:
  - RULE-SET,Shared,DIRECT
  - MATCH,DIRECT
"#,
        import_provider_path.to_string_lossy()
    )
}

async fn wait_for_json(url: &str) -> serde_json::Value {
    let mut last_error = String::new();
    for _ in 0..40 {
        match reqwest::get(url).await {
            Ok(response) if response.status().is_success() => {
                return response.json().await.expect("JSON response");
            }
            Ok(response) => {
                last_error = format!("HTTP {}", response.status());
            }
            Err(err) => {
                last_error = err.to_string();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("{url} did not become ready: {last_error}");
}

async fn wait_for_json_with_bearer(url: &str, secret: &str) -> serde_json::Value {
    let client = reqwest::Client::new();
    let mut last_error = String::new();
    for _ in 0..40 {
        match client.get(url).bearer_auth(secret).send().await {
            Ok(response) if response.status().is_success() => {
                return response.json().await.expect("JSON response");
            }
            Ok(response) => {
                last_error = format!("HTTP {}", response.status());
            }
            Err(err) => {
                last_error = err.to_string();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("{url} did not become ready with bearer auth: {last_error}");
}

async fn wait_for_traffic_frame(
    url: &str,
    expected_upload: i64,
    expected_download: i64,
) -> serde_json::Value {
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("traffic websocket connects");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(frame) = tokio::time::timeout(remaining, socket.next())
            .await
            .expect("traffic websocket frame before timeout")
        else {
            break;
        };
        let Ok(frame) = frame else {
            continue;
        };
        let Ok(text) = frame.into_text() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.get("up").and_then(serde_json::Value::as_i64) == Some(expected_upload)
            && value.get("down").and_then(serde_json::Value::as_i64) == Some(expected_download)
        {
            return value;
        }
    }
    panic!(
            "{url} did not publish expected traffic frame: up={expected_upload}, down={expected_download}"
        );
}

async fn wait_for_first_json_frame(url: &str) -> serde_json::Value {
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("websocket connects");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .expect("websocket frame before timeout")
        .expect("websocket frame")
        .expect("valid websocket frame")
        .into_text()
        .expect("text websocket frame");
    serde_json::from_str(&frame).expect("JSON websocket frame")
}

async fn spawn_healthcheck_http_server() -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let _ = stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    });
    format!("http://{addr}/generate_204")
}

async fn spawn_profile_refresh_http_server(good_body: String) -> (String, String) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buffer = [0_u8; 1024];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            if request.starts_with("GET /good ") {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/yaml\r\nContent-Length: {}\r\n\r\n{}",
                    good_body.len(),
                    good_body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            } else {
                let _ = stream
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        }
    });
    (format!("http://{addr}/good"), format!("http://{addr}/bad"))
}

async fn spawn_subscription_userinfo_http_server(body: String) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let headers = [
            (
                "upload=100; download=200; total=1000; expire=1893456000",
                "Remote%20Sub",
                "12",
            ),
            (
                "upload=300; download=400; total=2000; expire=1896048000",
                "Remote%20Sub%20Updated",
                "24",
            ),
        ];
        for (userinfo, title, interval) in headers {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/yaml\r\n",
                    "Subscription-Userinfo: {}\r\n",
                    "Profile-Title: {}\r\n",
                    "Profile-Update-Interval: {}\r\n",
                    "Profile-Web-Page-Url: https://example.test/portal\r\n",
                    "Support-Url: https://example.test/support\r\n",
                    "Content-Length: {}\r\n\r\n{}"
                ),
                userinfo,
                title,
                interval,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}/sub.yaml")
}

async fn spawn_subscription_metadata_comment_http_server(body: String) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).await;
        let response = format!(
            concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/yaml\r\n",
                "Profile-Title: Header%20Title\r\n",
                "Content-Length: {}\r\n\r\n{}"
            ),
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });
    format!("http://{addr}/sub.yaml")
}
