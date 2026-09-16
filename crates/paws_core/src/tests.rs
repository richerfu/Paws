use super::*;
use base64::Engine;
use futures::StreamExt;

static TEST_LOG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
const PLATFORM_RECOVERY_OWNER_CHILD_MARKER: &str = "PAWS_PLATFORM_RECOVERY_OWNER_CHILD_MARKER";

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
        "paws-core-traffic-history-test-{}",
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
    assert_eq!(snapshot.traffic_history[0].download_speed, 8);
    assert_eq!(snapshot.traffic_history[30].download_speed, 38);
    let latest = snapshot.traffic_history.last().unwrap();
    assert_eq!(latest.download_speed, 39);
    assert_eq!(latest.upload_speed, 78);
}

#[test]
fn snapshot_reads_are_pure_and_keep_the_same_revision() {
    let root =
        std::env::temp_dir().join(format!("paws-core-pure-snapshot-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);

    let first = core.snapshot().unwrap();
    let second = core.snapshot().unwrap();

    assert_eq!(first.revision, second.revision);
    assert_eq!(first.config_revision, second.config_revision);
    assert_eq!(first.observed_at_unix_nanos, second.observed_at_unix_nanos);
    assert_eq!(first.traffic_history, second.traffic_history);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn explicit_telemetry_refresh_publishes_a_new_telemetry_revision() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-telemetry-revision-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let receiver = core.subscribe_runtime_revisions();
    let before = *receiver.borrow();

    let refreshed = core.refresh_telemetry().unwrap();

    assert!(refreshed.revision > before.revision);
    assert!(refreshed.telemetry_revision > before.telemetry_revision);
    assert_eq!(refreshed.config_revision, before.config_revision);
    assert_eq!(refreshed.resource_revision, before.resource_revision);
    assert_eq!(*receiver.borrow(), refreshed);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_services_guard_allows_restart_after_task_drop() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-runtime-service-guard-test-{}",
        now_unix_nanos()
    ));
    let core = Arc::new(CoreHandle::new_with_profile_root(&root));
    let first_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    first_runtime.block_on(async {
        core.ensure_runtime_services();
        tokio::task::yield_now().await;
        assert!(core.runtime_services_task_started.load(Ordering::Acquire));
    });
    drop(first_runtime);

    assert!(!core.runtime_services_task_started.load(Ordering::Acquire));

    let before = core.telemetry_projection().unwrap().revisions;
    let replacement_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    replacement_runtime.block_on(async {
        core.ensure_runtime_services();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if core
                    .telemetry_projection()
                    .unwrap()
                    .revisions
                    .telemetry_revision
                    > before.telemetry_revision
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement runtime must restart telemetry services");
    });
    drop(replacement_runtime);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_blocked_runtime_service_does_not_stall_telemetry_sampling() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-independent-runtime-services-test-{}",
        now_unix_nanos()
    ));
    let core = Arc::new(CoreHandle::new_with_profile_root(&root));
    let config_guard = core.config_reload_lock.lock().await;
    let before = core.telemetry_projection().unwrap().revisions;

    core.ensure_runtime_services();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if core
                .telemetry_projection()
                .unwrap()
                .revisions
                .telemetry_revision
                > before.telemetry_revision
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("telemetry loop must advance while config synchronization is blocked");

    drop(config_guard);
    drop(core);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn disconnect_clears_exit_location_and_publishes_status_revision() {
    let root =
        std::env::temp_dir().join(format!("paws-core-exit-status-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let before = {
        let mut state = core.lock_state().unwrap();
        state.exit_location.ip = "203.0.113.7".to_owned();
        state.last_exit_location_check = Some(Instant::now());
        runtime_revisions(&state)
    };

    assert!(!core.refresh_exit_location_if_due().await.unwrap());

    let status = core.runtime_status_projection().unwrap();
    assert!(status.revisions.status_revision > before.status_revision);
    assert!(status.exit_location.ip.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn checked_provider_refresh_rejects_a_stale_resource_revision() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-stale-resource-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let expected = {
        let mut state = core.lock_state().unwrap();
        state.providers.push(ProviderSummary {
            name: "Remote".to_owned(),
            provider_type: "proxy".to_owned(),
            path: None,
            url: Some("https://example.test/provider.yaml".to_owned()),
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
        let expected = state.resource_revision;
        core.publish_resource_change_locked(&mut state);
        expected
    };

    let error = core
        .refresh_provider_checked("proxy", "Remote", expected)
        .await
        .expect_err("stale resource action must not start");

    assert!(matches!(error, PawsError::StaleResourceRevision { .. }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn newer_resource_operation_supersedes_an_older_completion() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-resource-operation-order-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let mut state = core.lock_state().unwrap();
    let operation_key = "provider:proxy:Remote";

    let (first_sequence, config_revision) =
        CoreHandle::begin_resource_operation_locked(&mut state, operation_key, None).unwrap();
    let (second_sequence, second_config_revision) =
        CoreHandle::begin_resource_operation_locked(&mut state, operation_key, None).unwrap();

    let stale_error = CoreHandle::ensure_resource_operation_current_locked(
        &state,
        operation_key,
        first_sequence,
        config_revision,
    )
    .expect_err("the older asynchronous completion must be rejected");
    assert!(
        matches!(stale_error, PawsError::Core(message) if message.contains("stale resource operation"))
    );
    CoreHandle::ensure_resource_operation_current_locked(
        &state,
        operation_key,
        second_sequence,
        second_config_revision,
    )
    .unwrap();

    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_proxy_selection_completion_is_rejected_before_persistence() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-stale-proxy-selection-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let expected = {
        let mut state = core.lock_state().unwrap();
        let expected = state.config_revision;
        core.publish_runtime_change_locked(&mut state, true, false);
        expected
    };

    let error = core
        .record_proxy_selection("GLOBAL", "DIRECT", false, expected)
        .expect_err("an old selector completion must not target newer configuration");

    assert!(matches!(error, PawsError::StaleConfigRevision { .. }));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn checked_profile_mutation_rejects_a_stale_revision_without_writing() {
    let root =
        std::env::temp_dir().join(format!("paws-core-stale-config-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let first_profile = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_profile = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    assert_ne!(first_profile, second_profile);
    let base_revision = core.snapshot().unwrap().config_revision;

    let receipt = core
        .set_profile_dns_config_checked(
            &second_profile,
            base_revision,
            vec!["9.9.9.9".to_owned()],
            Vec::new(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let current_revision = core.snapshot().unwrap().config_revision;
    assert!(current_revision > base_revision);
    assert_eq!(receipt.revisions.config_revision, current_revision);
    assert_eq!(receipt, core.config_projection().unwrap());

    let error = core
        .set_profile_dns_config_checked(
            &second_profile,
            base_revision,
            vec!["1.1.1.1".to_owned()],
            Vec::new(),
            BTreeMap::new(),
        )
        .await
        .expect_err("old form completion must not overwrite newer data");
    assert!(error.to_string().contains("stale configuration revision"));
    let raw_yaml = core.profile_raw_yaml(&second_profile).unwrap();
    assert!(raw_yaml.contains("9.9.9.9"));
    assert!(!raw_yaml.contains("1.1.1.1"));
    assert_eq!(core.snapshot().unwrap().config_revision, current_revision);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn failed_activation_keeps_the_previous_profile_active() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-activation-rollback-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let first_profile = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_profile = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let second_path = core
        .config_projection()
        .unwrap()
        .profiles
        .into_iter()
        .find(|profile| profile.id == second_profile)
        .unwrap()
        .raw_yaml_path;
    std::fs::write(second_path, "this: [is not valid YAML").unwrap();
    let before = core.config_projection().unwrap();

    core.activate_profile(&second_profile)
        .await
        .expect_err("invalid target must not become active");

    let after = core.config_projection().unwrap();
    assert_eq!(
        after.active_profile.as_deref(),
        Some(first_profile.as_str())
    );
    assert_eq!(
        after.revisions.config_revision,
        before.revisions.config_revision
    );
    let reopened = ProfileStore::open(root.clone()).unwrap();
    assert_eq!(reopened.active_profile(), Some(first_profile.as_str()));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn checked_profile_activation_rejects_a_stale_page_revision() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-checked-activation-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let first_profile = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_profile = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let stale_revision = core.config_projection().unwrap().revisions.config_revision;
    core.set_mode(RuntimeMode::Direct).unwrap();

    let error = core
        .activate_profile_checked(&second_profile, stale_revision)
        .await
        .expect_err("stale activation must not replace the current profile");

    assert!(matches!(error, PawsError::StaleConfigRevision { .. }));
    assert_eq!(
        core.config_projection().unwrap().active_profile.as_deref(),
        Some(first_profile.as_str())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn checked_profile_import_is_one_revision_and_rejects_stale_reuse() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-checked-import-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let initial = core.config_projection().unwrap();

    let receipt = core
        .import_profile_from_content_and_activate_checked(
            "Imported",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
            initial.revisions.config_revision,
        )
        .await
        .unwrap();

    assert_eq!(
        receipt.config.active_profile.as_deref(),
        Some(receipt.profile_id.as_str())
    );
    assert_eq!(
        receipt.config.revisions.config_revision,
        initial.revisions.config_revision + 1
    );
    let error = core
        .import_profile_from_content_and_activate_checked(
            "Late",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
            initial.revisions.config_revision,
        )
        .await
        .expect_err("an old async completion must not import or activate");
    assert!(matches!(error, PawsError::StaleConfigRevision { .. }));
    assert_eq!(core.config_projection().unwrap().profiles.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn profile_import_preparation_rejects_malformed_app_owned_fields_without_writing() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-invalid-app-profile-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let before = core.config_projection().unwrap();

    let error = core
        .prepare_profile_import_from_content(
            "Invalid app config",
            "test",
            "paws:\n  mixed-port: '17890'\nproxies: []\nproxy-groups: []\nrules: []\n",
            None,
        )
        .await
        .expect_err("a typed string port must not be laundered into the default port");

    assert!(error.to_string().contains("mixed-port"));
    assert_eq!(core.config_projection().unwrap(), before);
    assert_eq!(std::fs::read_dir(root.join("profiles")).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn profile_update_reports_both_primary_and_secondary_rollback_failures() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-secondary-rollback-failure-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Profile",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let expected_revision = core.config_projection().unwrap().revisions.config_revision;
    core.fail_next_config_reload.store(true, Ordering::Release);
    core.fail_next_profile_rollback
        .store(true, Ordering::Release);

    let error = core
        .set_profile_dns_config_checked(
            &profile_id,
            expected_revision,
            vec!["9.9.9.9".to_owned()],
            Vec::new(),
            BTreeMap::new(),
        )
        .await
        .expect_err("an injected rollback failure must be reported alongside the primary error");

    let message = error.to_string();
    assert!(message.contains("injected configuration reload failure"));
    assert!(message.contains("rollback also failed"));
    assert!(message.contains("injected profile rollback failure"));
    assert!(!message.contains("was rolled back"));
    assert!(core.config_projection().unwrap().revisions.config_revision > expected_revision);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn log_recording_creation_failure_is_projected_until_control_retry_succeeds() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-log-recording-create-failure-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    core.set_log_recording_enabled(false).unwrap();
    std::fs::remove_dir_all(root.join("logs")).unwrap();
    std::fs::write(root.join("logs"), "not a directory").unwrap();

    let error = core
        .set_log_recording_enabled(true)
        .expect_err("an unreadable log directory must fail explicitly");
    assert!(error.to_string().contains("log storage operation failed"));
    let projection = core.telemetry_projection().unwrap();
    assert!(projection
        .log_recording_error
        .as_deref()
        .is_some_and(|message| message.contains("log storage operation failed")));

    core.clear_logs().unwrap();
    assert!(core
        .telemetry_projection()
        .unwrap()
        .log_recording_error
        .is_some());

    std::fs::remove_file(root.join("logs")).unwrap();
    let status = core.set_log_recording_enabled(true).unwrap();
    assert!(status.enabled);
    assert_eq!(status.last_error, None);
    assert_eq!(
        core.telemetry_projection().unwrap().log_recording_error,
        None
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn vpn_start_rejects_unsupported_legacy_capabilities() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-vpn-start-capability-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);

    for (system_proxy, allow_bypass) in [(true, false), (false, true)] {
        let options = VpnOptions {
            system_proxy,
            allow_bypass,
            ..VpnOptions::default()
        };
        let error = core
            .start_vpn(-1, &to_json(&options).unwrap())
            .await
            .expect_err("unsupported VPN capabilities must be rejected before startup");
        assert!(error.to_string().contains("not supported"));
        assert!(!core.vpn.is_running());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn failed_checked_profile_activation_removes_the_imported_profile() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-import-activation-rollback-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let before = core.config_projection().unwrap();
    core.fail_next_config_reload.store(true, Ordering::Release);

    let error = core
        .import_profile_from_content_and_activate_checked(
            "Cannot activate",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
            before.revisions.config_revision,
        )
        .await
        .expect_err("reload failure must roll back the imported profile");

    assert!(error.to_string().contains("was rolled back"));
    let after = core.config_projection().unwrap();
    assert!(after.profiles.is_empty());
    assert!(after.active_profile.is_none());
    assert_eq!(
        after.revisions.config_revision,
        before.revisions.config_revision
    );
    assert_eq!(std::fs::read_dir(root.join("profiles")).unwrap().count(), 0);
    let reopened = ProfileStore::open(root.clone()).unwrap();
    assert!(reopened.summaries().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn checked_rule_import_rolls_back_persisted_rules_when_reload_fails() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-rule-import-rollback-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content("Rules", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();
    let before = core.config_projection().unwrap();
    core.fail_next_config_reload.store(true, Ordering::Release);

    let error = core
        .import_rules_from_content_checked(
            &profile_id,
            before.revisions.config_revision,
            "rules:injected-reload-failure",
            "DOMAIN,rollback.invalid,DIRECT",
        )
        .await
        .expect_err("reload failure must roll back imported rules");

    assert!(error.to_string().contains("rolled back"));
    let after = core.config_projection().unwrap();
    assert_eq!(after.rules, before.rules);
    assert_eq!(
        after.revisions.config_revision,
        before.revisions.config_revision
    );
    let reopened = ProfileStore::open(root.clone()).unwrap();
    assert_eq!(reopened.active_rules(), before.rules);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn publishing_platform_telemetry_does_not_sample_or_append_history() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-telemetry-publish-purity-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let before = core.snapshot().unwrap();
    let projected = {
        let state = core.lock_state().unwrap();
        platform_vpn_telemetry_projection(&state)
    };

    assert_eq!(projected.active_profile, before.active_profile);
    assert_eq!(projected.traffic, before.traffic);
    assert_eq!(projected.traffic_history, before.traffic_history);
    assert_eq!(projected.connections, before.connections);
    assert_eq!(projected.request_history, before.request_history);
    assert_eq!(projected.logs, before.logs);

    core.persist_vpn_telemetry().unwrap();

    let after = core.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.traffic_history, before.traffic_history);
    assert_eq!(after.traffic, before.traffic);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mode_changes_are_reflected() {
    let root = std::env::temp_dir().join(format!("paws-core-mode-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    core.set_mode(RuntimeMode::Direct).unwrap();
    assert_eq!(core.snapshot().unwrap().mode, RuntimeMode::Direct);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn global_mode_is_rejected_without_an_active_tunnel() {
    let root = std::env::temp_dir().join(format!("paws-global-empty-test-{}", now_unix_nanos()));
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
    let root = std::env::temp_dir().join(format!("paws-routing-mode-test-{}", now_unix_nanos()));
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
    let root = std::env::temp_dir().join(format!("paws-rule-lookup-test-{}", now_unix_nanos()));
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
    let root = std::env::temp_dir().join(format!("paws-rule-groups-test-{}", now_unix_nanos()));
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
    let root = std::env::temp_dir().join(format!("paws-global-no-proxy-test-{}", now_unix_nanos()));
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
        "paws-platform-vpn-status-{}",
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
        std::env::temp_dir().join(format!("paws-platform-vpn-revision-{}", now_unix_nanos()));
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
        "paws-platform-vpn-timeout-{}",
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

#[test]
fn app_home_requires_a_non_root_absolute_path() {
    assert!(validate_app_home_path(Path::new("")).is_err());
    assert!(validate_app_home_path(Path::new("relative/paws")).is_err());
    assert!(validate_app_home_path(Path::new("/")).is_err());
    assert!(
        validate_app_home_path(Path::new("/data/storage/el2/base/haps/entry/files/paws")).is_ok()
    );
}

#[test]
fn heartbeat_watchdog_uses_monotonic_wake_grace_and_resets_on_progress() {
    let root =
        std::env::temp_dir().join(format!("paws-platform-vpn-watchdog-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let mut state = core.lock_state().unwrap();
    state.platform_vpn_running = true;
    let now = Instant::now();
    state.platform_remote_state_seen_at =
        Some(now - PLATFORM_HEARTBEAT_STALE_AFTER - PLATFORM_HEARTBEAT_WAKE_GRACE);

    assert!(!platform_heartbeat_watchdog_expired(
        &mut state, now, true, true, true, false,
    ));
    state.platform_remote_stale_since = Some(now - PLATFORM_HEARTBEAT_WAKE_GRACE);
    assert!(platform_heartbeat_watchdog_expired(
        &mut state, now, true, true, true, false,
    ));
    assert!(!platform_heartbeat_watchdog_expired(
        &mut state, now, true, true, true, true,
    ));
    assert!(state.platform_remote_stale_since.is_none());
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_session_liveness_matches_watchdog_wake_grace() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-live-predicate-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let mut state = core.lock_state().unwrap();
    state.platform_start_attempt_id = "live-owner".to_owned();
    state.platform_start_outcome = PlatformStartOutcome::Connected;
    state.platform_extension_attached = true;
    state.platform_vpn_running = true;
    state.platform_vpn_cleanup_complete = false;
    let now = Instant::now();
    state.platform_remote_state_seen_at = Some(now - Duration::from_secs(1));
    assert!(platform_remote_session_is_live(&state, now));

    state.platform_remote_state_seen_at = Some(now - PLATFORM_HEARTBEAT_STALE_AFTER);
    state.platform_remote_stale_since = Some(now - Duration::from_secs(1));
    assert!(platform_remote_session_is_live(&state, now));
    state.platform_remote_stale_since = Some(now - PLATFORM_HEARTBEAT_WAKE_GRACE);
    assert!(!platform_remote_session_is_live(&state, now));

    state.platform_remote_state_seen_at = Some(now);
    state.platform_vpn_running = false;
    assert!(!platform_remote_session_is_live(&state, now));
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn owner_liveness_detects_killed_extension_without_waiting_for_heartbeat() {
    const CHILD_ROOT: &str = "PAWS_OWNER_LIVENESS_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let journal_path = root.join("runtime/platform-vpn-owner.json");
        let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
            panic!("pending owner journal missing");
        };
        let owner = current_process_identity().unwrap();
        let _lease = platform_owner::acquire_owner_lease_exact(
            &root.join("runtime/platform-vpn-owner.extension.lease"),
            platform_owner_lease_record(
                &journal.attempt_id,
                owner.clone(),
                PlatformVpnOwnerLeaseRole::Extension,
            ),
        )
        .unwrap();
        platform_owner::upgrade_attached_exact(
            &journal_path,
            &journal.attempt_id,
            journal.issuer,
            owner,
        )
        .unwrap();
        std::fs::write(root.join("extension-ready"), b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(10));
        return;
    }

    let root = std::env::temp_dir().join(format!("paws-owner-liveness-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tests::owner_liveness_detects_killed_extension_without_waiting_for_heartbeat")
        .arg("--test-threads=1")
        .env(CHILD_ROOT, &root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if root.join("extension-ready").exists() {
            break;
        }
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(root.join("extension-ready").exists());
    let JournalRead::Present(journal) =
        platform_owner::read(&root.join("runtime/platform-vpn-owner.json")).unwrap()
    else {
        panic!("attached owner journal missing");
    };
    let owner = journal.extension.unwrap();
    let mut remote = PlatformVpnState {
        start_attempt_id: attempt_id.clone(),
        start_outcome: PlatformStartOutcome::Connected,
        extension_attached: true,
        extension_owner_pid: owner.pid,
        extension_owner_start_time: owner.start_time,
        running: true,
        network_protected: true,
        updated_at: 1,
        ..PlatformVpnState::default()
    };
    {
        let mut state = core.lock_state().unwrap();
        core.apply_platform_envelope_locked(&mut state, true, None, Some(remote.clone()));
        core.refresh_platform_vpn_owner_liveness_locked(&mut state)
            .unwrap();
        assert!(
            state.platform_vpn_running,
            "a held lease must stay connected"
        );
    }
    let mut revisions = core.subscribe_runtime_revisions();
    let before = *revisions.borrow_and_update();
    let event_before = core.platform_vpn_event_revision();
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    {
        let mut state = core.lock_state().unwrap();
        assert!(
            state.platform_remote_state_seen_at.unwrap().elapsed() < PLATFORM_HEARTBEAT_STALE_AFTER
        );
        core.refresh_platform_vpn_owner_liveness_locked(&mut state)
            .unwrap();
        assert!(
            !state.platform_vpn_cleanup_complete,
            "owner exit is not OS stop confirmation"
        );
        // Even a newer timestamp from the dead owner's last publication must
        // not revive this terminal session.
        remote.updated_at = 2;
        core.apply_platform_envelope_locked(&mut state, true, None, Some(remote));
    }
    assert!(revisions.has_changed().unwrap());
    assert!(revisions.borrow_and_update().status_revision > before.status_revision);
    assert!(core.platform_vpn_event_revision() > event_before);
    let snapshot = core.snapshot().unwrap();
    assert_eq!(snapshot.vpn_lifecycle, VpnLifecycle::Failed);
    assert!(!snapshot.vpn_running);
    assert!(!snapshot.network_protected);
    assert!(snapshot
        .network_protect_error
        .unwrap()
        .contains("owner lease was released"));
    assert!(
        core.begin_platform_vpn_start().is_err(),
        "keep the exact cleanup barrier"
    );
    let event_after = core.platform_vpn_event_revision();
    core.refresh_platform_vpn_owner_liveness_locked(&mut core.lock_state().unwrap())
        .unwrap();
    assert_eq!(core.platform_vpn_event_revision(), event_after);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn owner_liveness_ignores_cleanup_and_a_rebound_journal() {
    for replacement in [false, true] {
        let root =
            std::env::temp_dir().join(format!("paws-owner-liveness-journal-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        let attempt_id = core.begin_platform_vpn_start().unwrap();
        core.bind_platform_vpn_start(&attempt_id).unwrap();
        core.set_platform_vpn_running(true).unwrap();
        let owner = current_process_identity().unwrap();
        let journal_path = root.join("runtime/platform-vpn-owner.json");
        let mut state = core.lock_state().unwrap();
        state.platform_vpn_extension_lease = None;
        if replacement {
            let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
                panic!("attached journal missing");
            };
            platform_owner::rebind_attached_exact(
                &journal_path,
                &attempt_id,
                journal.issuer,
                owner.clone(),
                ProcessIdentity {
                    start_time: owner.start_time + 1,
                    ..owner
                },
            )
            .unwrap();
        } else {
            platform_owner::delete_exact(&journal_path, &attempt_id, Some(owner)).unwrap();
        }
        let before = core.platform_vpn_event_revision();
        core.refresh_platform_vpn_owner_liveness_locked(&mut state)
            .unwrap();
        assert!(state.platform_vpn_running);
        assert_eq!(core.platform_vpn_event_revision(), before);
        drop(state);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn process_identity_requires_boot_and_process_filesystem_evidence() {
    let identity = ProcessIdentity {
        boot_id: "boot-a".to_owned(),
        pid: 42,
        start_time: 7,
    };
    assert_eq!(
        classify_process_identity(&identity, "boot-a", ProcessStartObservation::Found(7)),
        ProcessIdentityStatus::Alive
    );
    assert_eq!(
        classify_process_identity(&identity, "boot-a", ProcessStartObservation::Found(8)),
        ProcessIdentityStatus::Dead
    );
    assert_eq!(
        classify_process_identity(
            &identity,
            "boot-a",
            ProcessStartObservation::MissingWithProcessFsAvailable,
        ),
        ProcessIdentityStatus::Dead
    );
    assert_eq!(
        classify_process_identity(&identity, "boot-a", ProcessStartObservation::Unknown),
        ProcessIdentityStatus::Unknown
    );
    assert_eq!(
        classify_process_identity(&identity, "boot-b", ProcessStartObservation::Found(7)),
        ProcessIdentityStatus::Dead
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_process_identity_uses_the_stable_boot_session_uuid() {
    let first = read_boot_id().unwrap();
    let second = read_boot_id().unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 36);
    assert!(first.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    }));

    let identity = current_process_identity().unwrap();
    assert_eq!(identity.boot_id, first);
    assert_eq!(
        process_identity_status(&identity),
        ProcessIdentityStatus::Alive
    );
}

#[test]
fn vpn_intent_epoch_fences_queued_work_across_plugin_instances() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-intent-epoch-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let stale_start = core.advance_platform_vpn_intent().unwrap();
    let current_start = core.advance_platform_vpn_intent().unwrap();
    assert!(core
        .begin_platform_vpn_start_for_intent(stale_start)
        .unwrap_err()
        .to_string()
        .contains("superseded"));
    let attempt_id = core
        .begin_platform_vpn_start_for_intent(current_start)
        .unwrap();
    let stop_intent = core.advance_platform_vpn_intent().unwrap();
    assert!(core
        .claim_current_platform_vpn_stop(current_start)
        .unwrap_err()
        .to_string()
        .contains("superseded"));
    assert_eq!(
        core.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );
    let state = core.lock_state().unwrap();
    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Cancelled
    );
    assert!(state.platform_stop_requested);
    drop(state);
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    assert!(matches!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Present(PlatformVpnOwnerJournal {
            phase: PlatformVpnOwnerPhase::Stopping,
            extension: None,
            ..
        })
    ));
    let late_want = core
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string();
    assert!(late_want.contains("fenced by a stop intent"), "{late_want}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn os_stop_fence_survives_cleanup_ack_and_blocks_new_start_until_confirmation() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-os-stop-fence-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();

    let first_stop = core.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        core.claim_current_platform_vpn_stop(first_stop).unwrap(),
        attempt_id
    );
    assert!(core.complete_platform_vpn_cleanup(&attempt_id).unwrap());
    assert!(matches!(
        platform_owner::read(&root.join("runtime/platform-vpn-owner.json")).unwrap(),
        JournalRead::Missing
    ));

    let replacement = core.advance_platform_vpn_intent().unwrap();
    let blocked = core
        .begin_platform_vpn_start_for_intent(replacement)
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("OS stop"), "{blocked}");
    assert_eq!(
        core.claim_current_platform_vpn_stop(replacement).unwrap(),
        attempt_id
    );
    assert!(core
        .is_platform_vpn_stop_current(replacement, &attempt_id)
        .unwrap());
    assert!(core
        .begin_platform_vpn_os_stop(replacement, &attempt_id)
        .unwrap());

    let later_intent = core.advance_platform_vpn_intent().unwrap();
    assert!(core
        .claim_current_platform_vpn_stop(later_intent)
        .unwrap_err()
        .to_string()
        .contains("already in flight"));
    assert!(core
        .complete_platform_vpn_os_stop(replacement, &attempt_id)
        .unwrap());
    assert!(core
        .begin_platform_vpn_start_for_intent(later_intent)
        .is_ok());
    let _ = std::fs::remove_dir_all(root);
}

fn fail_next_platform_publish(core: &CoreHandle) {
    core.fail_next_platform_vpn_publish
        .store(true, Ordering::Release);
}

fn assert_replacement_start_rejects_late_want(core: &CoreHandle, old_attempt_id: &str) {
    let replacement_attempt_id = core.begin_platform_vpn_start().unwrap();
    assert_ne!(replacement_attempt_id, old_attempt_id);
    let error = core
        .validate_platform_owner_journal_for_want(old_attempt_id)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&format!(
            "owner journal belongs to {replacement_attempt_id}"
        )),
        "{error}"
    );
}

#[test]
fn exact_cleanup_publish_failure_releases_leases_before_returning_error() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-cleanup-publish-failure-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core
        .fail_platform_vpn_start(&attempt_id, "terminal before cleanup".to_owned())
        .unwrap());

    fail_next_platform_publish(&core);
    let error = core
        .complete_platform_vpn_cleanup(&attempt_id)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("injected platform publish failure"),
        "{error}"
    );
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert!(state.platform_vpn_issuer_lease.is_none());
    assert!(state.platform_vpn_extension_lease.is_none());
    drop(state);
    assert!(core
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string()
        .contains("journal is missing"));
    assert_replacement_start_rejects_late_want(&core, &attempt_id);
    drop(core);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirmed_recovery_publish_failure_releases_leases_before_returning_error() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-recovery-publish-failure-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    assert!(core.cancel_platform_vpn_start(&attempt_id).unwrap());
    let stop_intent = core.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        core.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );
    assert!(core
        .begin_platform_vpn_os_stop(stop_intent, &attempt_id)
        .unwrap());
    assert!(core
        .complete_platform_vpn_os_stop(stop_intent, &attempt_id)
        .unwrap());

    fail_next_platform_publish(&core);
    let error = core
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("injected platform publish failure"),
        "{error}"
    );
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert!(state.platform_vpn_issuer_lease.is_none());
    assert!(state.platform_vpn_extension_lease.is_none());
    drop(state);
    assert!(core
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string()
        .contains("journal is missing"));
    assert_replacement_start_rejects_late_want(&core, &attempt_id);
    drop(core);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unattached_failure_publish_error_releases_issuer_lease_and_fences_late_want() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-unattached-publish-failure-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    fail_next_platform_publish(&core);
    let error = core
        .fail_unattached_platform_vpn_start(&attempt_id, "system rejected".to_owned())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("injected platform publish failure"),
        "{error}"
    );
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert!(state.platform_vpn_issuer_lease.is_none());
    assert!(state.platform_vpn_extension_lease.is_none());
    drop(state);
    assert!(core
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string()
        .contains("journal is missing"));
    assert_replacement_start_rejects_late_want(&core, &attempt_id);
    drop(core);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn attached_owner_winning_stop_fence_resyncs_terminal_before_binding_publish() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-attach-stop-race-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
        panic!("pending owner journal missing");
    };
    let owner = current_process_identity().unwrap();
    let extension_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.extension.lease"),
        platform_owner_lease_record(
            &attempt_id,
            owner.clone(),
            PlatformVpnOwnerLeaseRole::Extension,
        ),
    )
    .unwrap();
    platform_owner::upgrade_attached_exact(
        &journal_path,
        &attempt_id,
        journal.issuer,
        owner.clone(),
    )
    .unwrap();

    let stop_intent = core.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        core.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );

    let mut state = core.lock_state().unwrap();
    state.platform_vpn_extension_lease = Some(extension_lease);
    core.publish_platform_vpn_binding_locked(&mut state, &attempt_id, &owner)
        .unwrap();
    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Cancelled
    );
    assert!(state.platform_stop_requested);
    assert!(state.platform_extension_attached);
    drop(state);
    assert_eq!(core.extension_tick(&attempt_id).unwrap(), "stopping");
    assert!(core.complete_platform_vpn_cleanup(&attempt_id).unwrap());

    let source = include_str!("lib.rs");
    let post_cas_publish = source
        .split_once("fn publish_platform_vpn_binding_locked")
        .unwrap()
        .1
        .split_once("pub async fn await_platform_vpn_start")
        .unwrap()
        .0;
    assert!(
        post_cas_publish
            .find("sync_platform_vpn_state_locked(state)")
            .unwrap()
            < post_cas_publish
                .find("state.platform_extension_owner_pid = owner_pid")
                .unwrap(),
        "the Attached winner must re-sync the Stop lane before publishing ownership"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn watchdog_orphan_cleanup_requires_confirmed_stop_and_dead_exact_owner() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-watchdog-recovery-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = "watchdog-owner".to_owned();
    let issuer = current_process_identity().unwrap();
    let dead_owner = ProcessIdentity {
        boot_id: read_boot_id().unwrap(),
        pid: u32::MAX,
        start_time: 1,
    };
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let issuer_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.issuer.lease"),
        platform_owner_lease_record(
            &attempt_id,
            issuer.clone(),
            PlatformVpnOwnerLeaseRole::Issuer,
        ),
    )
    .unwrap();
    platform_owner::create_pending_exact(
        &journal_path,
        PlatformVpnOwnerJournal {
            attempt_id: attempt_id.clone(),
            issuer: issuer.clone(),
            extension: None,
            phase: PlatformVpnOwnerPhase::Pending,
        },
    )
    .unwrap();
    let extension_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.extension.lease"),
        platform_owner_lease_record(
            &attempt_id,
            dead_owner.clone(),
            PlatformVpnOwnerLeaseRole::Extension,
        ),
    )
    .unwrap();
    platform_owner::upgrade_attached_exact(&journal_path, &attempt_id, issuer, dead_owner).unwrap();
    drop(extension_lease);
    drop(issuer_lease);
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_attempt_id = attempt_id.clone();
        state.platform_start_outcome = PlatformStartOutcome::Connected;
        state.platform_extension_attached = true;
        state.platform_extension_owner_pid = u32::MAX;
        state.platform_extension_owner_start_time = 1;
        state.platform_vpn_running = true;
        state.platform_vpn_cleanup_complete = false;
        state.platform_remote_state_updated_at = 7;
        let now = Instant::now();
        state.platform_remote_state_seen_at =
            Some(now - PLATFORM_HEARTBEAT_STALE_AFTER - PLATFORM_HEARTBEAT_WAKE_GRACE);
        state.platform_remote_stale_since = Some(now - PLATFORM_HEARTBEAT_WAKE_GRACE);
        core.apply_platform_envelope_locked(
            &mut state,
            true,
            None,
            Some(PlatformVpnState {
                start_attempt_id: attempt_id.clone(),
                start_outcome: PlatformStartOutcome::Connected,
                delivery_observed: true,
                extension_attached: true,
                stop_requested: false,
                extension_owner_pid: u32::MAX,
                extension_owner_start_time: 1,
                cleanup_complete: false,
                starting: false,
                running: true,
                network_protected: true,
                network_protect_error: None,
                updated_at: 7,
            }),
        );
        assert_eq!(state.platform_start_outcome, PlatformStartOutcome::Failed);
        assert!(state.platform_watchdog_cleanup_recoverable);
    }

    assert_eq!(
        core.current_recoverable_platform_vpn_session_id().unwrap(),
        attempt_id
    );
    assert!(core
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .unwrap());
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert!(!state.platform_extension_attached);
    assert!(!state.platform_watchdog_cleanup_recoverable);
    drop(state);
    assert!(core.begin_platform_vpn_start().is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirmed_os_stop_waits_for_a_hung_exact_owner_to_really_exit() {
    if let Some(marker) = std::env::var_os(PLATFORM_RECOVERY_OWNER_CHILD_MARKER) {
        std::fs::write(marker, b"owner-alive").unwrap();
        std::thread::sleep(Duration::from_secs(10));
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-live-owner-stop-{}",
        now_unix_nanos()
    ));
    let marker = root.join("owner-alive");
    std::fs::create_dir_all(&root).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tests::confirmed_os_stop_waits_for_a_hung_exact_owner_to_really_exit")
        .arg("--test-threads=1")
        .env(PLATFORM_RECOVERY_OWNER_CHILD_MARKER, &marker)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if marker.exists() {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "owner child exited before reaching its hold point"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(marker.exists(), "owner child did not reach its hold point");

    let owner = ProcessIdentity {
        boot_id: read_boot_id().unwrap(),
        pid: child.id(),
        start_time: read_process_start_time(child.id()).unwrap(),
    };
    assert_eq!(
        process_identity_status(&owner),
        ProcessIdentityStatus::Alive
    );
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGSTOP) }, 0);

    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
        panic!("pending owner journal missing");
    };
    let extension_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.extension.lease"),
        platform_owner_lease_record(
            &attempt_id,
            owner.clone(),
            PlatformVpnOwnerLeaseRole::Extension,
        ),
    )
    .unwrap();
    platform_owner::upgrade_attached_exact(
        &journal_path,
        &attempt_id,
        journal.issuer,
        owner.clone(),
    )
    .unwrap();
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_outcome = PlatformStartOutcome::Connected;
        state.platform_vpn_starting = false;
        state.platform_vpn_running = true;
        state.platform_extension_attached = true;
        state.platform_extension_owner_pid = owner.pid;
        state.platform_extension_owner_start_time = owner.start_time;
        state.platform_vpn_cleanup_complete = false;
    }
    let stop_intent = core.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        core.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );

    let killer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        child.kill().unwrap();
        let status = child.wait().unwrap();
        drop(extension_lease);
        status
    });
    let started = Instant::now();
    assert!(core
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .unwrap());
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "recovery must observe exact ownership release, not treat OS stop acknowledgement as cleanup"
    );
    assert!(!killer.await.unwrap().success());
    assert_eq!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Missing
    );
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert!(!state.platform_vpn_running);
    assert!(!state.platform_extension_attached);
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn begin_persists_the_pending_issuer_before_returning() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-pending-journal-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    let JournalRead::Present(journal) =
        platform_owner::read(&root.join("runtime/platform-vpn-owner.json")).unwrap()
    else {
        panic!("pending owner journal missing");
    };
    assert_eq!(journal.attempt_id, attempt_id);
    assert_eq!(journal.phase, PlatformVpnOwnerPhase::Pending);
    assert!(journal.extension.is_none());
    assert_eq!(
        platform_owner::observe_owner_lease_exact(
            &root.join("runtime/platform-vpn-owner.issuer.lease"),
            &platform_owner_lease_record(
                &attempt_id,
                journal.issuer,
                PlatformVpnOwnerLeaseRole::Issuer,
            ),
        )
        .unwrap(),
        PlatformVpnOwnerLeaseObservation::HeldExact
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cold_begin_does_not_rewrite_a_released_issuer_lease_with_an_old_journal() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-cold-begin-owner-{}",
        now_unix_nanos()
    ));
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let issuer_lease_path = root.join("runtime/platform-vpn-owner.issuer.lease");
    let attempt_id = "old-pending-owner";
    let issuer = current_process_identity().unwrap();
    let issuer_lease = platform_owner::acquire_owner_lease_exact(
        &issuer_lease_path,
        platform_owner_lease_record(
            attempt_id,
            issuer.clone(),
            PlatformVpnOwnerLeaseRole::Issuer,
        ),
    )
    .unwrap();
    platform_owner::create_pending_exact(
        &journal_path,
        PlatformVpnOwnerJournal {
            attempt_id: attempt_id.to_owned(),
            issuer,
            extension: None,
            phase: PlatformVpnOwnerPhase::Pending,
        },
    )
    .unwrap();
    drop(issuer_lease);
    let lease_header_before = std::fs::read(&issuer_lease_path).unwrap();

    let cold = CoreHandle::new_with_profile_root(&root);
    let error = cold.begin_platform_vpn_start().unwrap_err().to_string();
    assert!(
        error.contains("owner journal") && error.contains(attempt_id),
        "{error}"
    );
    assert_eq!(
        std::fs::read(&issuer_lease_path).unwrap(),
        lease_header_before,
        "a new issuer must not hold and rewrite the fixed lease inode while the old journal is observable"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn cold_ui_recovers_a_dead_pending_issuer_and_fences_its_late_want() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-cold-pending-{}",
        now_unix_nanos()
    ));
    let original = CoreHandle::new_with_profile_root(&root);
    let attempt_id = original.begin_platform_vpn_start().unwrap();
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
        panic!("pending owner journal missing");
    };
    assert!(platform_owner::delete_exact(&journal_path, &attempt_id, None).unwrap());
    drop(original);
    let dead_issuer = ProcessIdentity {
        boot_id: journal.issuer.boot_id,
        pid: u32::MAX,
        start_time: 1,
    };
    let dead_issuer_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.issuer.lease"),
        platform_owner_lease_record(
            &attempt_id,
            dead_issuer.clone(),
            PlatformVpnOwnerLeaseRole::Issuer,
        ),
    )
    .unwrap();
    platform_owner::create_pending_exact(
        &journal_path,
        PlatformVpnOwnerJournal {
            attempt_id: attempt_id.clone(),
            issuer: dead_issuer,
            extension: None,
            phase: PlatformVpnOwnerPhase::Pending,
        },
    )
    .unwrap();
    drop(dead_issuer_lease);
    let cold = CoreHandle::new_with_profile_root(&root);
    let rejected_before_bind = cold
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string();
    assert!(
        rejected_before_bind.contains("issuer lease")
            && rejected_before_bind.contains("was released"),
        "{rejected_before_bind}"
    );

    let stop_intent = cold.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        cold.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );
    assert!(cold
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .unwrap());
    assert_eq!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Missing
    );
    let late_want = cold
        .validate_platform_owner_journal_for_want(&attempt_id)
        .unwrap_err()
        .to_string();
    assert!(late_want.contains("journal is missing"), "{late_want}");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn cold_ui_recovers_only_the_exact_dead_attached_owner() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-cold-attached-{}",
        now_unix_nanos()
    ));
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let attempt_id = "cold-attached";
    let issuer = current_process_identity().unwrap();
    let dead_extension = ProcessIdentity {
        boot_id: issuer.boot_id.clone(),
        pid: u32::MAX,
        start_time: 1,
    };
    let issuer_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.issuer.lease"),
        platform_owner_lease_record(
            attempt_id,
            issuer.clone(),
            PlatformVpnOwnerLeaseRole::Issuer,
        ),
    )
    .unwrap();
    platform_owner::create_pending_exact(
        &journal_path,
        PlatformVpnOwnerJournal {
            attempt_id: attempt_id.to_owned(),
            issuer: issuer.clone(),
            extension: None,
            phase: PlatformVpnOwnerPhase::Pending,
        },
    )
    .unwrap();
    let extension_lease = platform_owner::acquire_owner_lease_exact(
        &root.join("runtime/platform-vpn-owner.extension.lease"),
        platform_owner_lease_record(
            attempt_id,
            dead_extension.clone(),
            PlatformVpnOwnerLeaseRole::Extension,
        ),
    )
    .unwrap();
    platform_owner::upgrade_attached_exact(&journal_path, attempt_id, issuer, dead_extension)
        .unwrap();
    drop(extension_lease);
    drop(issuer_lease);

    let cold = CoreHandle::new_with_profile_root(&root);
    let stop_intent = cold.advance_platform_vpn_intent().unwrap();
    assert_eq!(
        cold.claim_current_platform_vpn_stop(stop_intent).unwrap(),
        attempt_id
    );
    assert!(cold
        .recover_platform_vpn_cleanup_after_confirmed_stop(attempt_id)
        .await
        .unwrap());
    assert_eq!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Missing
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ordinary_failure_keeps_cleanup_barrier_while_exact_owner_is_alive() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-no-watchdog-recovery-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core
        .fail_platform_vpn_start(&attempt_id, "native failure".to_owned())
        .unwrap());

    assert_eq!(
        core.current_recoverable_platform_vpn_session_id().unwrap(),
        attempt_id
    );
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
        panic!("attached owner journal missing");
    };
    assert_eq!(
        platform_owner::observe_owner_lease_exact(
            &root.join("runtime/platform-vpn-owner.extension.lease"),
            &platform_owner_lease_record(
                &attempt_id,
                journal.extension.unwrap(),
                PlatformVpnOwnerLeaseRole::Extension,
            ),
        )
        .unwrap(),
        PlatformVpnOwnerLeaseObservation::HeldExact
    );
    assert!(!core.lock_state().unwrap().platform_vpn_cleanup_complete);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn delivered_unattached_terminal_owner_recovers_only_after_confirmed_stop() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-delivered-recovery-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    assert!(core.cancel_platform_vpn_start(&attempt_id).unwrap());
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_delivery_observed = true;
        state.platform_extension_attached = false;
        state.platform_remote_state_updated_at = 23;
    }

    assert_eq!(
        core.current_recoverable_platform_vpn_session_id().unwrap(),
        attempt_id
    );
    let intent_epoch = core.lock_state().unwrap().platform_vpn_intent_epoch;
    assert_eq!(
        core.claim_current_platform_vpn_stop(intent_epoch).unwrap(),
        attempt_id
    );
    assert!(core
        .begin_platform_vpn_os_stop(intent_epoch, &attempt_id)
        .unwrap());
    assert!(core.begin_platform_vpn_start().is_err());
    assert!(core
        .complete_platform_vpn_os_stop(intent_epoch, &attempt_id)
        .unwrap());
    assert!(core
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .unwrap());
    let state = core.lock_state().unwrap();
    assert!(state.platform_vpn_cleanup_complete);
    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Cancelled
    );
    assert!(!state.platform_extension_attached);
    drop(state);
    assert!(core.begin_platform_vpn_start().is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delivered_terminal_owner_that_attached_still_requires_extension_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-delivered-attached-cleanup-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core.cancel_platform_vpn_start(&attempt_id).unwrap());

    assert_eq!(
        core.current_recoverable_platform_vpn_session_id().unwrap(),
        attempt_id
    );
    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let JournalRead::Present(journal) = platform_owner::read(&journal_path).unwrap() else {
        panic!("attached owner journal missing");
    };
    assert_eq!(
        platform_owner::observe_owner_lease_exact(
            &root.join("runtime/platform-vpn-owner.extension.lease"),
            &platform_owner_lease_record(
                &attempt_id,
                journal.extension.unwrap(),
                PlatformVpnOwnerLeaseRole::Extension,
            ),
        )
        .unwrap(),
        PlatformVpnOwnerLeaseObservation::HeldExact
    );
    assert!(!core.lock_state().unwrap().platform_vpn_cleanup_complete);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cleanup_recovery_requires_exact_owner_death_not_heartbeat_silence() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-recovery-progress-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = "watchdog-progress-owner".to_owned();
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_attempt_id = attempt_id.clone();
        state.platform_start_outcome = PlatformStartOutcome::Failed;
        state.platform_extension_attached = true;
        state.platform_vpn_cleanup_complete = false;
        state.platform_stop_requested = true;
        state.platform_extension_owner_pid = 42;
        state.platform_extension_owner_start_time = 7;
        state.platform_watchdog_cleanup_recoverable = true;
        assert_eq!(
            platform_cleanup_recovery_proof_with_owner_status(&state, ProcessIdentityStatus::Alive,),
            CleanupRecoveryProof::OwnerAlive
        );
        assert_eq!(
            platform_cleanup_recovery_proof_with_owner_status(
                &state,
                ProcessIdentityStatus::Unknown,
            ),
            CleanupRecoveryProof::OwnerLivenessUnknown
        );
        assert_eq!(
            platform_cleanup_recovery_proof_with_owner_status(&state, ProcessIdentityStatus::Dead,),
            CleanupRecoveryProof::Proven
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn older_platform_frame_does_not_refresh_remote_liveness() {
    let root =
        std::env::temp_dir().join(format!("paws-platform-vpn-old-frame-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let mut state = core.lock_state().unwrap();
    state.platform_start_attempt_id = "attempt-current".to_owned();
    state.platform_start_outcome = PlatformStartOutcome::Connected;
    state.platform_vpn_running = true;
    state.platform_remote_state_updated_at = 20;
    let last_seen = Instant::now();
    state.platform_remote_state_seen_at = Some(last_seen);

    core.apply_platform_envelope_locked(
        &mut state,
        true,
        None,
        Some(PlatformVpnState {
            start_attempt_id: "attempt-current".to_owned(),
            start_outcome: PlatformStartOutcome::Connected,
            delivery_observed: true,
            extension_attached: true,
            stop_requested: false,
            extension_owner_pid: 0,
            extension_owner_start_time: 0,
            cleanup_complete: false,
            starting: false,
            running: true,
            network_protected: false,
            network_protect_error: None,
            updated_at: 19,
        }),
    );

    assert_eq!(state.platform_remote_state_updated_at, 20);
    assert_eq!(state.platform_remote_state_seen_at, Some(last_seen));
    assert!(state.platform_vpn_running);
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn new_platform_attempt_resets_remote_liveness_revision() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-new-attempt-liveness-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_attempt_id = "finished-attempt".to_owned();
        state.platform_start_outcome = PlatformStartOutcome::Failed;
        state.platform_vpn_cleanup_complete = true;
        state.platform_remote_state_updated_at = u128::MAX;
        state.platform_remote_state_seen_at = Some(Instant::now());
        state.platform_remote_stale_since = Some(Instant::now());
    }

    core.begin_platform_vpn_start().unwrap();

    let state = core.lock_state().unwrap();
    assert_eq!(state.platform_remote_state_updated_at, 0);
    assert!(state.platform_remote_state_seen_at.is_none());
    assert!(state.platform_remote_stale_since.is_none());
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_platform_frame_cannot_ack_connection_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-live-cleanup-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let mut state = core.lock_state().unwrap();
    state.platform_start_attempt_id = "attempt-live".to_owned();
    state.platform_start_outcome = PlatformStartOutcome::Pending;
    state.platform_extension_attached = true;

    core.apply_platform_envelope_locked(
        &mut state,
        true,
        None,
        Some(PlatformVpnState {
            start_attempt_id: "attempt-live".to_owned(),
            start_outcome: PlatformStartOutcome::Connected,
            delivery_observed: true,
            extension_attached: true,
            stop_requested: false,
            extension_owner_pid: 0,
            extension_owner_start_time: 0,
            cleanup_complete: true,
            starting: false,
            running: true,
            network_protected: true,
            network_protect_error: None,
            updated_at: 1,
        }),
    );

    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Connected
    );
    assert!(state.platform_vpn_running);
    assert!(!state.platform_vpn_cleanup_complete);
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn platform_want_validation_rejects_idle_owner_frame() {
    let envelope = platform_ipc::PlatformEnvelope {
        state: Some(PlatformVpnState {
            start_attempt_id: "attempt-idle".to_owned(),
            start_outcome: PlatformStartOutcome::Idle,
            ..PlatformVpnState::default()
        }),
        ..platform_ipc::PlatformEnvelope::default()
    };

    assert!(validate_platform_start_envelope(&envelope, "attempt-idle").is_err());
    assert!(validate_platform_start_envelope(&envelope, "").is_err());
}

#[test]
fn terminal_platform_attempt_cannot_be_revived_by_a_late_running_frame() {
    let root =
        std::env::temp_dir().join(format!("paws-platform-vpn-terminal-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    {
        let mut state = core.lock_state().unwrap();
        state.platform_start_attempt_id = "attempt-terminal".to_owned();
        state.platform_start_outcome = PlatformStartOutcome::Failed;
        state.platform_vpn_running = false;
        core.apply_platform_envelope_locked(
            &mut state,
            true,
            None,
            Some(PlatformVpnState {
                start_attempt_id: "attempt-terminal".to_owned(),
                start_outcome: PlatformStartOutcome::Connected,
                delivery_observed: true,
                extension_attached: true,
                stop_requested: false,
                extension_owner_pid: 0,
                extension_owner_start_time: 0,
                cleanup_complete: false,
                starting: false,
                running: true,
                network_protected: true,
                network_protect_error: None,
                updated_at: 1,
            }),
        );
        assert_eq!(state.platform_start_outcome, PlatformStartOutcome::Failed);
        assert!(!state.platform_vpn_running);
        assert!(!state.platform_network_protected);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn platform_start_completes_only_on_matching_connected_terminal() {
    let root =
        std::env::temp_dir().join(format!("paws-platform-vpn-connected-{}", now_unix_nanos()));
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
async fn platform_attach_wait_is_exact_and_wakes_before_start_dispatch_completion() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-attach-wait-{}",
        now_unix_nanos()
    ));
    let core = Arc::new(CoreHandle::new_with_profile_root(&root));
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    assert_eq!(
        core.await_platform_vpn_attach("older-attempt")
            .await
            .unwrap(),
        PlatformAttachOutcome::Superseded
    );

    let wait_core = Arc::clone(&core);
    let wait_attempt = attempt_id.clone();
    let waiter =
        tokio::spawn(async move { wait_core.await_platform_vpn_attach(&wait_attempt).await });
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("extension attachment did not wake the waiter")
            .unwrap()
            .unwrap(),
        PlatformAttachOutcome::Attached
    );

    let terminal_root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-attach-terminal-{}",
        now_unix_nanos()
    ));
    let terminal_core = CoreHandle::new_with_profile_root(&terminal_root);
    let terminal_attempt = terminal_core.begin_platform_vpn_start().unwrap();
    assert!(terminal_core
        .fail_unattached_platform_vpn_start(&terminal_attempt, "rejected".to_owned())
        .unwrap());
    assert_eq!(
        terminal_core
            .await_platform_vpn_attach(&terminal_attempt)
            .await
            .unwrap(),
        PlatformAttachOutcome::Terminal
    );

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(terminal_root);
}

#[test]
fn terminal_delivery_acknowledges_only_the_exact_terminal_attempt() {
    let mut cancelled = PlatformVpnState {
        start_attempt_id: "attempt-a".to_owned(),
        start_outcome: PlatformStartOutcome::Cancelled,
        extension_attached: true,
        cleanup_complete: false,
        starting: true,
        running: true,
        network_protected: true,
        updated_at: 1,
        ..PlatformVpnState::default()
    };

    assert!(acknowledge_terminal_delivery_state(
        &mut cancelled,
        "attempt-a"
    ));
    assert_eq!(cancelled.start_attempt_id, "attempt-a");
    assert_eq!(cancelled.start_outcome, PlatformStartOutcome::Cancelled);
    assert!(cancelled.delivery_observed);
    assert!(!cancelled.extension_attached);
    assert!(!cancelled.cleanup_complete);
    assert!(!cancelled.starting);
    assert!(!cancelled.running);
    assert!(!cancelled.network_protected);

    let mut new_owner = PlatformVpnState {
        start_attempt_id: "attempt-b".to_owned(),
        start_outcome: PlatformStartOutcome::Pending,
        starting: true,
        updated_at: 9,
        ..PlatformVpnState::default()
    };
    let before = new_owner.clone();
    assert!(!acknowledge_terminal_delivery_state(
        &mut new_owner,
        "attempt-a"
    ));
    assert_eq!(new_owner.start_attempt_id, before.start_attempt_id);
    assert_eq!(new_owner.start_outcome, before.start_outcome);
    assert_eq!(new_owner.delivery_observed, before.delivery_observed);
    assert_eq!(new_owner.starting, before.starting);
    assert_eq!(new_owner.updated_at, before.updated_at);
}

#[tokio::test]
async fn platform_vpn_state_changes_are_delivered_by_revisioned_events() {
    let root = std::env::temp_dir().join(format!("paws-platform-vpn-events-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let initial_revision = core.platform_vpn_event_revision();

    core.begin_platform_vpn_start().unwrap();
    let starting_revision = core
        .await_platform_vpn_event(initial_revision)
        .await
        .unwrap();
    assert!(starting_revision > initial_revision);
    assert_eq!(
        core.snapshot().unwrap().vpn_lifecycle,
        VpnLifecycle::Starting
    );

    core.set_platform_vpn_running(true).unwrap();
    let connected_revision = core
        .await_platform_vpn_event(starting_revision)
        .await
        .unwrap();
    assert!(connected_revision > starting_revision);
    assert!(core.snapshot().unwrap().vpn_running);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn platform_start_failure_is_exactly_once() {
    let root = std::env::temp_dir().join(format!("paws-platform-vpn-failed-{}", now_unix_nanos()));
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
async fn platform_stop_waits_for_exact_connection_cleanup_ack() {
    let root = std::env::temp_dir().join(format!("paws-platform-vpn-cleanup-{}", now_unix_nanos()));
    let core = Arc::new(CoreHandle::new_with_profile_root(&root));
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core.cancel_platform_vpn_start(&attempt_id).unwrap());

    let journal_path = root.join("runtime/platform-vpn-owner.json");
    let journal = match platform_owner::read(&journal_path).unwrap() {
        JournalRead::Present(journal) => journal,
        JournalRead::Missing => panic!("attached owner journal is missing"),
    };
    let current_owner = journal.extension.clone().unwrap();
    let mut foreign_boot_owner = current_owner.clone();
    foreign_boot_owner.boot_id.push_str("-foreign");
    platform_owner::rebind_attached_exact(
        &journal_path,
        &attempt_id,
        journal.issuer.clone(),
        current_owner.clone(),
        foreign_boot_owner.clone(),
    )
    .unwrap();
    assert!(core
        .complete_platform_vpn_cleanup(&attempt_id)
        .unwrap_err()
        .to_string()
        .contains("does not match the Extension acknowledging cleanup"));
    assert!(matches!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Present(PlatformVpnOwnerJournal {
            extension: Some(extension),
            ..
        }) if extension == foreign_boot_owner
    ));
    platform_owner::rebind_attached_exact(
        &journal_path,
        &attempt_id,
        journal.issuer,
        foreign_boot_owner,
        current_owner,
    )
    .unwrap();

    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            core.await_platform_vpn_stop(&attempt_id),
        )
        .await
        .is_err(),
        "native terminal state must not stand in for VpnConnection.destroy"
    );
    assert!(core
        .begin_platform_vpn_start()
        .unwrap_err()
        .to_string()
        .contains("cleanup is still pending"));

    let wait_core = Arc::clone(&core);
    let wait_attempt = attempt_id.clone();
    let waiter =
        tokio::spawn(async move { wait_core.await_platform_vpn_stop(&wait_attempt).await });
    tokio::task::yield_now().await;
    assert!(core.complete_platform_vpn_cleanup(&attempt_id).unwrap());
    assert!(tokio::time::timeout(Duration::from_millis(100), waiter)
        .await
        .expect("cleanup acknowledgement did not wake the waiter")
        .unwrap()
        .unwrap());
    assert_eq!(
        platform_owner::read(&journal_path).unwrap(),
        JournalRead::Missing
    );
    assert!(core.begin_platform_vpn_start().is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn attempt_scoped_callbacks_reject_stale_and_post_terminal_updates() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-attempt-callbacks-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();

    assert!(!core
        .set_platform_vpn_starting_for_attempt("stale-attempt", true)
        .unwrap());
    assert!(!core
        .set_platform_vpn_starting_for_attempt(&attempt_id, true)
        .unwrap());
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    assert!(core
        .set_platform_vpn_failed_for_attempt(&attempt_id, "native failed".to_owned())
        .unwrap());
    assert!(!core
        .set_platform_vpn_starting_for_attempt(&attempt_id, true)
        .unwrap());
    assert!(!core
        .set_platform_network_protected_for_attempt(&attempt_id, true, None)
        .unwrap());

    let state = core.lock_state().unwrap();
    assert_eq!(state.platform_start_outcome, PlatformStartOutcome::Failed);
    assert!(!state.platform_vpn_running);
    assert_eq!(
        state.platform_network_protect_error.as_deref(),
        Some("native failed")
    );
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn connected_attempt_cannot_regress_to_starting() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-connected-callback-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    core.set_platform_vpn_running(true).unwrap();

    assert!(!core
        .set_platform_vpn_starting_for_attempt(&attempt_id, true)
        .unwrap());
    let state = core.lock_state().unwrap();
    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Connected
    );
    assert!(state.platform_vpn_running);
    assert!(!state.platform_vpn_starting);
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bound_pending_attempt_reports_starting_before_the_native_worker_exists() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-bound-before-native-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();

    assert_eq!(core.vpn.lifecycle(), NativeVpnLifecycle::Stopped);
    assert_eq!(core.extension_tick(&attempt_id).unwrap(), "starting");
    let state = core.lock_state().unwrap();
    assert_eq!(state.platform_start_outcome, PlatformStartOutcome::Pending);
    assert!(state.platform_vpn_starting);
    assert!(!state.platform_vpn_running);
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fresh_extension_process_can_recover_the_same_connected_attempt() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-process-recovery-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    core.set_platform_vpn_running(true).unwrap();
    assert!(!core.vpn.is_running());

    core.bind_platform_vpn_start(&attempt_id).unwrap();

    let state = core.lock_state().unwrap();
    assert_eq!(state.platform_start_outcome, PlatformStartOutcome::Pending);
    assert!(state.platform_vpn_starting);
    assert!(!state.platform_vpn_running);
    assert!(!state.platform_vpn_cleanup_complete);
    drop(state);
    assert!(core
        .set_platform_vpn_starting_for_attempt(&attempt_id, true)
        .unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn heartbeat_does_not_observe_a_transient_native_restart_state() {
    let root = std::env::temp_dir().join(format!(
        "paws-platform-vpn-heartbeat-operation-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let attempt_id = core.begin_platform_vpn_start().unwrap();
    core.bind_platform_vpn_start(&attempt_id).unwrap();
    core.set_platform_vpn_running(true).unwrap();

    let _operation_guard = core.vpn_operation_lock.try_lock().unwrap();
    assert_eq!(core.extension_tick(&attempt_id).unwrap(), "connected");
    let state = core.lock_state().unwrap();
    assert_eq!(
        state.platform_start_outcome,
        PlatformStartOutcome::Connected
    );
    assert!(state.platform_vpn_running);
    drop(state);
    drop(_operation_guard);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn system_rejection_only_fails_before_extension_attachment() {
    let root =
        std::env::temp_dir().join(format!("paws-platform-vpn-attachment-{}", now_unix_nanos()));
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
        std::env::temp_dir().join(format!("paws-platform-vpn-deadline-{}", now_unix_nanos()));
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
  - name: VLESS H2Mux
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: 00000000-0000-0000-0000-000000000002
    smux:
      enabled: true
      protocol: h2mux
  - name: VLESS Yamux
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: 00000000-0000-0000-0000-000000000003
    smux:
      enabled: true
      protocol: yamux
  - name: VLESS MuxCool
    type: vless
    server: 127.0.0.1
    port: 443
    uuid: 00000000-0000-0000-0000-000000000004
    smux:
      enabled: true
      protocol: muxcool
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
    proxies: [SS, Trojan, VLESS, VLESS H2Mux, VLESS Yamux, VLESS MuxCool, AnyTLS, VMess, Snell, Hysteria2, HTTP, SOCKS5, DIRECT]
rules:
  - MATCH,Proxy
"#;
    let config = load_meow_config(yaml).await.unwrap();
    for proxy in [
        "SS",
        "Trojan",
        "VLESS",
        "VLESS H2Mux",
        "VLESS Yamux",
        "VLESS MuxCool",
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
    let root = std::env::temp_dir().join(format!("paws-runtime-log-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let message = format!(
        "paws runtime log test {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    tracing::warn!(target: "paws_core_test", "{}", message);

    core.refresh_telemetry().unwrap();
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
    let root = std::env::temp_dir().join(format!("paws-runtime-filter-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let message = format!("arkit framework log {}", now_unix_nanos());

    tracing::warn!(target: "arkit::renderer", "{}", message);

    assert!(!core
        .snapshot()
        .unwrap()
        .logs
        .iter()
        .any(|log| log.message.contains(&message)));
    assert!(is_vpn_log_target("paws_vpn::tun"));
    assert!(is_vpn_log_target("meow_tunnel::dispatcher"));
    assert!(!is_vpn_log_target("arkit::renderer"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn clear_logs_removes_state_and_runtime_logs() {
    let _guard = TEST_LOG_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("paws-clear-log-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    {
        let mut state = core.lock_state().unwrap();
        state.logs.push(warning_log("state log to clear"));
    }
    tracing::warn!(target: "paws_core_test", "runtime log to clear");
    let before = core.telemetry_projection().unwrap();
    assert!(!before.logs.is_empty());

    core.clear_logs().unwrap();

    let after = core.telemetry_projection().unwrap();
    assert!(after.logs.is_empty());
    assert!(after.revisions.telemetry_revision > before.revisions.telemetry_revision);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_loads_engine_without_marking_vpn_connected() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-reload-state-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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

#[tokio::test]
async fn reloading_the_same_profile_does_not_advance_config_revision() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-reload-domain-revision-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Revision domains",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
    let before = core.runtime_status_projection().unwrap().revisions;

    core.reload_config(&profile_id).await.unwrap();

    let after = core.runtime_status_projection().unwrap().revisions;
    assert_eq!(after.config_revision, before.config_revision);
    assert!(after.status_revision > before.status_revision);
    assert!(after.resource_revision > before.resource_revision);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_activity_rules_persist_and_hot_update_the_existing_tunnel() {
    let root =
        std::env::temp_dir().join(format!("paws-core-manual-rule-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(root.clone());
    let profile_id = core
        .import_profile_from_content(
            "Manual rule",
            "test",
            &paws_profile::default_runtime_yaml(),
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
                match_kind: paws_model::ManualRuleMatchKind::Domain,
                value: "API.Example.COM.".to_owned(),
                target: "Proxy".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        added.mutation.kind,
        paws_model::ManualRuleMutationKind::Added
    );
    assert_eq!(added.mutation.line, "DOMAIN,api.example.com,Proxy");
    assert!(added.live_updated);
    assert!(added.rule_mode_active);

    let updated = core
        .apply_manual_rule(
            &profile_id,
            &ManualRuleSpec {
                match_kind: paws_model::ManualRuleMatchKind::Domain,
                value: "api.example.com".to_owned(),
                target: "DIRECT".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.mutation.rule_id, added.mutation.rule_id);
    assert_eq!(
        updated.mutation.kind,
        paws_model::ManualRuleMutationKind::Updated
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
        "paws-core-vpn-prepare-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = Arc::new(CoreHandle::new_with_profile_root(root));
    core.import_profile_from_content(
        "Direct",
        "test",
        &paws_profile::default_runtime_yaml(),
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
    let root = std::env::temp_dir().join(format!("paws-core-ui-cache-test-{}", now_unix_nanos()));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Cached dashboard",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-ui-cache-writer-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Cache writer",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-vpn-cache-writer-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "VPN cache isolation",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-ui-cache-revision-test-{}",
        now_unix_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(&root);
    let profile_id = core
        .import_profile_from_content(
            "Changed dashboard",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        paws_profile::default_runtime_yaml()
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
        "paws-core-geodata-clean-test-{}",
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
        "paws-core-managed-validation-test-{}",
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
        "paws-core-vpn-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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

    core.stop_vpn().await.unwrap();
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
        "paws-core-dns-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content("DNS", "test", &paws_profile::default_runtime_yaml(), None)
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
        "paws-core-vpn-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let profile_id = core
        .import_profile_from_content("VPN", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    core.reload_config(&profile_id).await.unwrap();

    let previous_revision = core.snapshot().unwrap().config_revision;
    let error = core
        .set_profile_vpn_config(&profile_id, true, false, true, "lwip".to_owned())
        .await
        .expect_err("unsupported application bypass must not be persisted");
    assert!(error.to_string().contains("not supported"));
    assert_eq!(core.snapshot().unwrap().config_revision, previous_revision);

    let error = core
        .set_profile_vpn_config(&profile_id, true, false, false, "lwip".to_owned())
        .await
        .expect_err("unsupported system proxy management must not be persisted");
    assert!(error.to_string().contains("not supported"));
    assert_eq!(core.snapshot().unwrap().config_revision, previous_revision);

    core.set_profile_vpn_config(&profile_id, false, false, false, "lwip".to_owned())
        .await
        .unwrap();

    let snapshot = core.snapshot().unwrap();
    assert!(!snapshot.vpn_options.system_proxy);
    assert!(!snapshot.vpn_options.dns_hijacking);
    assert!(!snapshot.vpn_options.allow_bypass);
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
        "paws-core-direct-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-direct-echo-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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
        let mut payload = vec![0_u8; "paws-echo-payload".len()];
        stream.read_exact(&mut payload).await.unwrap();
        stream.write_all(&payload).await.unwrap();
    });

    let payload = "paws-echo-payload";
    let echoed = core
        .test_proxy_echo("DIRECT", &format!("http://{addr}"), payload, Some(1000))
        .await
        .unwrap();
    assert_eq!(echoed, payload);
    accept_handle.await.unwrap();
    let snapshot = core.snapshot().unwrap();
    assert!(snapshot.logs.iter().any(|log| {
        log.message
            .contains(&format!("DIRECT echo roundtrip: {} bytes", payload.len()))
    }));
}

#[test]
fn proxy_echo_metadata_uses_an_opaque_tcp_tunnel() {
    let metadata = proxy_test_metadata("http://127.0.0.1:8080", "paws-echo").unwrap();
    assert_eq!(metadata.conn_type, ConnType::Inner);
    assert_eq!(metadata.host.as_str(), "127.0.0.1");
    assert_eq!(metadata.dst_port, 8080);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_uses_meow_tunnel_statistics_for_connections_and_traffic() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-meow-stats-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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

    core.refresh_telemetry().unwrap();
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

    let before_clear_revision = snapshot.config_revision;
    let before_clear_runtime_revision = snapshot.revision;
    core.clear_request_history().unwrap();
    let cleared = core.snapshot().unwrap();
    assert!(cleared.request_history.is_empty());
    assert_eq!(cleared.config_revision, before_clear_revision);
    assert!(cleared.revision > before_clear_runtime_revision);

    let first = track_test_connection(&tunnel, "one.example");
    let second = track_test_connection(&tunnel, "two.example");
    core.refresh_telemetry().unwrap();
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
    let root =
        std::env::temp_dir().join(format!("paws-core-tun-direction-test-{}", now_unix_nanos()));
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
        "paws-core-traffic-stop-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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

    core.refresh_telemetry().unwrap();
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
        "paws-core-profile-switch-tun-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-profile-switch-meow-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-profile-delete-traffic-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let first_id = core
        .import_profile_from_content("First", "test", &paws_profile::default_runtime_yaml(), None)
        .await
        .unwrap();
    let second_id = core
        .import_profile_from_content(
            "Second",
            "test",
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-platform-stop-traffic-test-{}",
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
            &paws_profile::default_runtime_yaml(),
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
        "paws-core-controller-test-{}",
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
        "paws controller ws log test {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/logs?level=warning"))
        .await
        .unwrap();
    tracing::warn!(target: "paws_core_controller_test", "{}", warning);
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
        "paws-core-controller-sync-test-{}",
        now_unix_nanos()
    ));
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let core = CoreHandle::new_with_profile_root_and_controller(root, addr);
    let original = format!(
        r#"mixed-port: 7890
paws:
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
    assert!(persisted.contains("paws:"));
    assert!(persisted.contains("mtu: 1410"));
    assert!(!persisted.contains("external-controller:"));
    core.stop_vpn().await.unwrap();
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_exposes_loaded_provider_registries() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-provider-controller-test-{}",
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
    let runtime_provider_dir = root.join("runtime/providers/proxy").join(&profile_id);
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
        "paws-core-provider-duplicate-test-{}",
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
    let runtime_provider_dir = root.join("runtime/providers/proxy").join(&profile_id);
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
        "paws-core-inline-rule-provider-test-{}",
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
        "paws-core-provider-stale-cache-test-{}",
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
        "paws-core-provider-empty-refresh-test-{}",
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
        "paws-core-provider-inline-refresh-test-{}",
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
        "paws-core-selected-proxy-test-{}",
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
        "paws-core-automatic-group-test-{}",
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
        "paws-core-sniffer-test-{}",
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
        "paws-core-profile-edit-test-{}",
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
        "paws-core-profile-delete-active-test-{}",
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
        "paws-core-refresh-all-test-{}",
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
        "paws-core-sub-userinfo-test-{}",
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
        "paws-core-sub-comment-metadata-test-{}",
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
        "paws-core-protocol-feature-test-{}",
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
        "paws-core-share-subscription-test-{}",
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
        "paws-core-share-transport-test-{}",
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
http://user:pass@127.0.0.1:8080?headers=User-Agent%3DPaws%3BProxy-Authorization%3DBearer%20token#HTTP-SHARE
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
async fn unsupported_transport_options_are_rejected_instead_of_ignored() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-transport-contract-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let core = CoreHandle::new_with_profile_root(root);
    let trojan_grpc = "trojan://password@example.test:443?type=grpc&serviceName=svc&sni=edge.example.test#TROJAN-GRPC";
    let error = core
        .import_profile_from_content("unsupported", "clipboard", trojan_grpc, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Trojan network 'grpc' is not implemented"));

    let ws_h2 = r#"
proxies:
  - name: WS-H2
    type: vless
    server: edge.example.test
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    tls: true
    network: ws
    alpn: [h2, http/1.1]
    ws-opts:
      headers:
        Host: cdn.example.test
proxy-groups:
  - name: Proxy
    type: select
    proxies: [WS-H2]
rules: [MATCH,Proxy]
"#;
    let error = core
        .validate_profile_content(ws_h2)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("WebSocket ALPN must be exactly http/1.1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_local_protocol_profiles_import_and_populate_proxy_groups() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-generated-profiles-test-{}",
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
        "paws-core-ss-echo-test-{}",
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
            "paws-ss-echo",
            Some(1000),
        )
        .await
        .unwrap();
    assert_eq!(echoed, "paws-ss-echo");

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
            "paws-ss-echo",
            Some(300),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("echo test"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_and_socks5_auth_echo_roundtrip_and_bad_credentials_fail() {
    let root = std::env::temp_dir().join(format!(
        "paws-core-http-socks-echo-test-{}",
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
            "paws-http-echo",
            Some(1000),
        )
        .await
        .unwrap(),
        "paws-http-echo"
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
            "paws-http-echo",
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
            "paws-socks-echo",
            Some(1000),
        )
        .await
        .unwrap(),
        "paws-socks-echo"
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
            "paws-socks-echo",
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
                        || !request.contains("X-Paws-Test: local-protocol")
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
