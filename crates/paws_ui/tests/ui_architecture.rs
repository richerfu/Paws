const UI: &str = include_str!("../src/ui.rs");
const STORE: &str = include_str!("../src/ui_store.rs");
const VIEW: &str = include_str!("../src/view.rs");
const OPERATIONS: &str = include_str!("../src/ui/operations.rs");
const TASKS: &str = include_str!("../src/ui/tasks.rs");
const PROFILES: &str = include_str!("../src/view/pages/profiles.rs");
const SETTINGS: &str = include_str!("../src/view/pages/settings.rs");
const RESOURCES: &str = include_str!("../src/view/pages/resources.rs");
const ACTIVITY: &str = include_str!("../src/view/pages/activity.rs");
const REACTIVE: &str = include_str!("../src/reactive_signal.rs");
const VPN_OPERATION: &str = include_str!("../src/vpn_operation.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn production_ui_has_no_universal_state_action_or_command_bus() {
    let production = [UI, STORE, VIEW, OPERATIONS, TASKS, VPN_OPERATION].join("\n");
    for removed in [
        "struct State",
        "enum Action",
        "struct Command",
        "Signal<State>",
        "dispatch(",
        "content_key",
        "Duration::from_millis(40)",
    ] {
        assert!(
            !production.contains(removed),
            "obsolete UI architecture returned: {removed}"
        );
    }
    assert!(
        !production.contains("RuntimeSnapshot"),
        "the UI must consume typed projections, not the aggregate snapshot"
    );
}

#[test]
fn store_exposes_independent_domains_and_pages_own_their_drafts() {
    for domain in [
        "Signal<PreferencesProjection>",
        "Signal<SessionProjection>",
        "Signal<ProfilesProjection>",
        "Signal<ProxiesProjection>",
        "Signal<ActivityProjection>",
        "Signal<TelemetryProjection>",
        "Signal<ResourcesProjection>",
        "Signal<DiagnosticsProjection>",
        "Signal<LogsProjection>",
        "Signal<SettingsProjection>",
        "Signal<AboutProjection>",
    ] {
        assert!(STORE.contains(domain), "missing typed domain: {domain}");
    }

    assert!(PROFILES.contains("use_signal(|| None::<YamlEditorDraft>)"));
    assert!(SETTINGS.contains("use_signal(move || SettingsDraft::new(initial))"));
    assert!(RESOURCES.contains("use_local_rule_editors()"));
    assert!(ACTIVITY.contains("use_local_rule_editors()"));

    let profiles = section(
        STORE,
        "struct ProfilesProjection",
        "struct ProxiesProjection",
    );
    let proxies = section(
        STORE,
        "struct ProxiesProjection",
        "struct ActivityProjection",
    );
    let logs = section(STORE, "struct LogsProjection", "struct ProfileImportState");
    let session = section(
        STORE,
        "struct SessionProjection",
        "struct ProfilesProjection",
    );
    assert!(!profiles.contains("loading"));
    assert!(!profiles.contains("succeeded"));
    assert!(!proxies.contains("pending"));
    assert!(!logs.contains("recording_pending"));
    assert!(!session.contains("pending"));
    assert!(STORE.contains("struct UiOperationStores"));
    assert!(STORE.contains("Signal<ProfileImportState>"));
    assert!(STORE.contains("Signal<ProxyOperationState>"));
    assert!(STORE.contains("Signal<LogOperationState>"));
    assert!(STORE.contains("Signal<VpnOperationState>"));
    assert!(STORE.contains("bootstrap_error: Option<String>"));
    assert!(STORE.contains("runtime_error: Option<String>"));
    assert!(VIEW.contains("stores.set_bootstrap_error"));
    assert!(VIEW.contains("RuntimeProjectionUpdate::ClearError(error)"));
}

#[test]
fn vpn_operation_ownership_is_not_derived_from_runtime_status() {
    let status_apply = section(
        STORE,
        "fn apply_status_projection",
        "fn apply_resource_projection",
    );
    assert!(!status_apply.contains("operations"));
    assert!(!status_apply.contains("pending"));
    assert!(!status_apply.contains("VpnOperationState"));

    assert!(VPN_OPERATION.contains("struct ActiveVpnOperation"));
    assert!(VPN_OPERATION.contains("id: u64"));
    assert!(VPN_OPERATION.contains("owner: Option<String>"));
    assert!(VPN_OPERATION.contains("fn begin_recovery"));
    assert!(VPN_OPERATION.contains("RecoveryStillBlocked"));
    assert!(OPERATIONS.contains("confirm_vpn_operation(operation.receipt)"));
    assert!(OPERATIONS.contains("finish_vpn_command(operation_id, result)"));
}

#[test]
fn every_vpn_bridge_outcome_retains_its_receipt_until_global_settlement() {
    let bridge = include_str!("../src/bridge/mod.rs");
    for request in [
        "pub(crate) async fn request_start_vpn",
        "pub(crate) async fn request_stop_vpn()",
        "pub(crate) async fn request_stop_vpn_if_current",
        "pub(crate) async fn request_restart_vpn",
    ] {
        let tail = &bridge[bridge.find(request).expect("VPN request function")..];
        let body = &tail[..tail.find("\n}\n").expect("VPN request function end")];
        assert!(body.contains("request_id"), "{request} lacks a request id");
        assert!(
            body.contains("submitted_receipt"),
            "{request} does not preserve a receipt after submission"
        );
        assert!(
            body.contains("await_vpn_operation"),
            "{request} does not observe the submitted receipt"
        );
    }

    assert!(OPERATIONS.contains("crate::bridge::VpnOperationOutcome::Unconfirmed(operation)"));
    assert!(OPERATIONS.contains("state.mark_unconfirmed(operation_id, operation)"));
    assert!(OPERATIONS.contains("vpn_operation_confirmed_not_applied"));
    let import_commit = section(
        OPERATIONS,
        "fn commit_profile_import",
        "fn finish_profile_import_with_error",
    );
    assert!(
        import_commit.find("finish_vpn_followup").unwrap()
            < import_commit.find("if !import_is_current").unwrap()
    );
}

#[test]
fn runtime_updates_are_event_driven_and_domain_versioned() {
    assert!(VIEW.contains("subscribe_runtime_revisions()"));
    assert!(!VIEW.contains("tokio::time::interval"));
    for projection in [
        "config_projection()",
        "telemetry_projection()",
        "runtime_status_projection()",
        "resource_projection()",
    ] {
        assert!(VIEW.contains(projection), "provider omits {projection}");
    }
    for revision in [
        "config_revision < versions.config_revision",
        "telemetry_revision < versions.telemetry_revision",
        "status_revision < versions.status_revision",
        "resource_revision < versions.resource_revision",
    ] {
        assert!(STORE.contains(revision), "missing stale guard: {revision}");
    }
    assert!(STORE.contains("replace_if_changed"));
}

#[test]
fn query_and_mutation_lifetimes_are_explicit() {
    assert!(VIEW.contains("fn use_page_tasks() -> PageTasks"));
    assert!(OPERATIONS.contains("pub(crate) fn run_durable_mutation"));
    assert!(STORE.contains("query_task: std::sync::Arc"));
    assert!(STORE.contains("task.abort()"));
    assert!(OPERATIONS.contains("if !local.is_alive()"));
    assert!(TASKS.contains("tokio::select!"));
    assert!(TASKS.contains("ProfileImportPreparation::Cancelled"));
    assert!(TASKS.contains("commit_prepared_profile_import_and_activate_checked"));
    assert!(OPERATIONS.contains("commit_prepared_profile_import(prepared).await"));
    assert!(OPERATIONS.contains("commit_prepared_rule_import_checked"));
    assert!(OPERATIONS.contains("current_revision != expected_config_revision"));
    assert!(VIEW.contains("let task = bootstrap_tokio.spawn(bootstrap_active_profile())"));
    assert!(REACTIVE.contains("tokio::sync::mpsc::channel(4)"));
    assert!(VIEW.contains("queries.retain(|task| !task.is_finished())"));
    assert!(!TASKS.contains("contains(\"cancel\")"));
}
