const UI: &str = concat!(
    include_str!("../src/ui.rs"),
    include_str!("../src/ui/tasks.rs"),
    include_str!("../src/ui/operations.rs"),
    include_str!("../src/view.rs")
);
const VPN_PLUGIN: &str = include_str!("../../../entry/src/main/ets/plugins/VpnPlugin.ets");
const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets");
const VPN_CONFIG: &str = include_str!("../../../entry/src/main/ets/vpnability/VpnConfig.ets");
const NAPI_TYPES: &str = include_str!("../../../entry/src/main/cpp/types/libpaws_ui/Index.d.ts");
const PLATFORM_CALLBACKS: &str = include_str!("../src/bridge/mod.rs");
const CORE: &str = include_str!("../../paws_core/src/lib.rs");
const PLATFORM_IPC: &str = include_str!("../../paws_core/src/platform_ipc.rs");
const NETSTACK: &str = include_str!("../../paws_vpn/src/netstack.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn vpn_start_does_not_reload_the_already_active_profile_or_poll_state() {
    let start = section(
        UI,
        "async fn start_vpn_command",
        "async fn stop_vpn_command",
    );

    assert!(start.contains("active_profile.as_deref() != Some(profile_id.as_str())"));
    assert!(!start.contains("Duration::from_millis(350)"));
    assert!(UI.contains("subscribe_runtime_revisions"));
    assert!(!UI.contains("delayed_vpn_snapshot"));
    assert!(!UI.contains("Duration::from_millis(200)"));
}

#[test]
fn notification_permission_is_deferred_until_after_the_vpn_ability_launches() {
    let request = section(
        VPN_PLUGIN,
        "async requestStartVpn",
        "private async requestStopVpnWithContext",
    );
    let launch = request
        .find("vpnExtension.startVpnExtensionAbility")
        .expect("VPN ability launch");
    let permission = request
        .find("this.ensureSpeedNotificationPermission(context)")
        .expect("deferred notification permission");

    assert!(launch < permission);
    assert!(request.contains("reusing pending VPN start request"));
}

#[test]
fn first_authorization_start_is_coordinated_by_the_extension_terminal_state() {
    assert!(NAPI_TYPES.contains("beginPlatformVpnStartForIntent(intentEpoch: string): string"));
    assert!(!NAPI_TYPES.contains("beginPlatformVpnStart(): string"));
    assert!(NAPI_TYPES.contains("bindPlatformVpnStart(attemptId: string): string"));
    assert!(NAPI_TYPES.contains("awaitPlatformVpnStart(attemptId: string): Promise<string>"));
    assert!(NAPI_TYPES.contains("failUnattachedPlatformVpnStart("));
    assert!(!NAPI_TYPES.contains("awaitPlatformVpnStartAttachment"));

    let request = section(
        VPN_PLUGIN,
        "async requestStartVpn",
        "private async requestStopVpnWithContext",
    );
    assert!(request.contains("advancePlatformVpnIntent()"));
    assert!(request.contains("claimCurrentPlatformVpnStop(intentEpoch)"));
    assert!(request.contains("beginPlatformVpnStartForIntent(intentEpoch)"));
    assert!(request.contains("awaitPlatformVpnStart(attemptId)"));
    assert!(request.contains("failUnattachedPlatformVpnStart(attemptId, message)"));
    assert!(request.contains("buildVpnWant(optionsJson, this.platformSharedMemory, attemptId)"));
    // The original Want is redispatch unconditionally after the system
    // acknowledgement; no timed attach probing participates in the flow.
    assert!(request.contains("system acknowledged VPN start attempt"));
    assert!(request.contains("redispatching VPN start attempt"));
    assert!(request.contains("redispatched VPN start attempt"));
    assert!(!request.contains("awaitPlatformVpnStartAttachment"));
    assert!(!request.contains("VPN_EXTENSION_ATTACH_GRACE_MS"));
    assert!(!request.contains("Promise.race"));
    assert!(!request.contains("15000"));

    let extension = section(
        VPN_ABILITY,
        "private startFromWant",
        "private recordVpnFailureForAttempt",
    );
    let attach = extension
        .find("attachPlatformSharedMemory")
        .expect("ashmem attachment");
    let bind = extension
        .find("bindPlatformVpnStart")
        .expect("attempt binding");
    let running = extension
        .find("extensionTick(attemptId)")
        .expect("native lifecycle confirmation");
    assert!(attach < bind);
    assert!(bind < running);
}

#[test]
fn vpn_extension_subscription_is_event_driven_without_polling() {
    assert!(NAPI_TYPES.contains("waitForPlatformChangeEvent(): Promise<boolean>"));
    assert!(NAPI_TYPES.contains("cancelPlatformChangeWait(): void"));
    assert!(!NAPI_TYPES.contains("waitForPlatformChange(timeoutMs"));

    let subscription = section(
        VPN_ABILITY,
        "private async runPlatformSubscription",
        "private startTelemetry",
    );
    assert!(subscription.contains("await pawsUi.waitForPlatformChangeEvent()"));
    assert!(subscription.contains("pawsUi.syncPlatformChanges()"));
    assert!(!subscription.contains("waitForPlatformChange(1000)"));
    // Stopping the subscription wakes the parked waiter so the loop unwinds
    // without waiting for the next peer frame.
    assert!(VPN_ABILITY.contains("cancelPlatformChangeWait"));

    assert!(CORE.contains("wait_for_platform_change_event"));
    assert!(CORE.contains("cancel_platform_change_wait"));
    assert!(PLATFORM_IPC.contains("wait_event_cancellable"));
    assert!(PLATFORM_IPC.contains("cancel_event_waits"));
}

#[test]
fn tun_reader_parks_on_readiness_instead_of_busy_polling() {
    assert!(NETSTACK.contains("AsyncFd"));
    assert!(NETSTACK.contains("reader_tun.readable()"));
    assert!(NETSTACK.contains("reader_shutdown.notified()"));
    assert!(NETSTACK.contains("guard.clear_ready()"));
    assert!(!NETSTACK.contains("from_micros(200)"));
    assert!(!NETSTACK.contains("yield_now"));
}

#[test]
fn descriptor_free_authorization_bootstrap_waits_for_the_rebound_want() {
    let start = section(
        VPN_ABILITY,
        "private enqueueStartFromWant",
        "private async handleRequest",
    );
    let bootstrap = section(
        start,
        "const sharedMemory = readPlatformSharedMemoryFds(want)",
        "if (!this.isLatestWant",
    );

    assert!(bootstrap.contains("authorization bootstrap"));
    assert!(bootstrap.contains("waiting for rebound request"));
    assert!(!bootstrap.contains("recordVpnFailure"));
}

#[test]
fn first_authorization_want_unwraps_nested_parameters_and_descriptors() {
    assert!(VPN_CONFIG.contains("const PAWS_SYSTEM_PARAMETERS_KEY = 'myParams'"));
    assert!(VPN_CONFIG.contains("function readVpnParameters"));
    assert!(VPN_CONFIG.contains("readFileDescriptorParameter"));
    assert!(VPN_CONFIG.contains("readPlatformStartAttemptId"));
}

#[test]
fn platform_vpn_state_uses_one_event_pump_and_in_process_subscribers() {
    assert!(CORE.contains("start_platform_vpn_event_pump"));
    assert!(CORE.contains("platform.wait_for_change_event()"));
    assert!(CORE.contains("platform_vpn_event_tx"));
    assert!(CORE.contains("await_platform_vpn_event"));
    assert!(PLATFORM_IPC.contains("self.notification.wait(None)"));

    let terminal_wait = section(
        CORE,
        "async fn await_platform_vpn_start_with_deadline",
        "pub fn fail_unattached_platform_vpn_start",
    );
    assert!(terminal_wait.contains("receiver.changed()"));
    assert!(!terminal_wait.contains("wait_for_platform_change"));

    assert!(ENTRY_ABILITY.contains("new LazyPlugin(() => this.createVpnPlugin())"));
    assert!(!UI.contains("delayed_vpn_snapshot"));
}

#[test]
fn native_profile_prepare_overlaps_tun_creation() {
    assert!(NAPI_TYPES.contains("prepareVpn(attemptId: string): Promise<boolean>"));
    let start = section(
        VPN_ABILITY,
        "private async performStart",
        "private recordVpnFailureForAttempt",
    );
    let prepare = start
        .find("pawsUi.prepareVpn(attemptId)")
        .expect("native prepare");
    let create = start.find(".create(config)").expect("TUN creation");
    let await_prepare = start
        .find("nativePrepare, 'native VPN prepare'")
        .expect("native prepare await");
    let native_start = start.find("pawsUi.startVpn").expect("native VPN start");

    assert!(prepare < create);
    assert!(create < await_prepare);
    assert!(await_prepare < native_start);
}

#[test]
fn dashboard_mount_does_not_wait_for_profile_parsing() {
    let create = section(ENTRY_ABILITY, "public async onCreate", "public onNewWant");
    assert!(create.contains("this.configureNativeHome()"));
    assert!(create.contains(
        "throw new Error(describeError(err) || 'critical native initialization failed')"
    ));
    let critical = create.find("this.configureNativeHome()").unwrap();
    let optional = create.find("this.configureNativeLocale()").unwrap();
    let native_create = create.find("await super.onCreate").unwrap();
    assert!(critical < optional);
    assert!(optional < native_create);
    assert!(create.contains("this.runOptionalDebugAutomation(want)"));
    assert!(ENTRY_ABILITY.contains("private runOptionalDebugAutomation"));
    assert!(ENTRY_ABILITY.contains("debug automation terminated"));
    assert!(!create.contains("prepareVpn"));
    assert!(!create.contains("prepareNativeProfileForFirstFrame"));

    let bootstrap = section(
        UI,
        "async fn bootstrap_active_profile",
        "async fn lookup_rule",
    );
    assert!(bootstrap.contains("core.prepare_active_vpn()"));
    assert!(bootstrap.contains(".await"));
    assert!(!bootstrap.contains("core.reload_config"));

    let loader = section(
        include_str!("../../paws_core/src/controller.rs"),
        "async fn load_meow_config",
        "fn tunnel_from_config",
    );
    assert!(loader.contains("tokio::task::spawn_blocking"));

    let window = section(
        ENTRY_ABILITY,
        "protected async loadWindowStageContent",
        "\n  }\n}",
    );
    assert!(window.contains("win.setUIContent('pages/Index')"));
    assert!(window.contains("full-screen layout skipped"));
    assert!(window.contains("throw new Error(message)"));
    assert!(
        window.find("win.setWindowLayoutFullScreen(true)").unwrap()
            < window.find("win.setUIContent('pages/Index')").unwrap()
    );
    assert!(!window.contains("prepareVpn"));
    assert!(!window.contains("firstFramePreparation"));
}

#[test]
fn vpn_restart_is_an_owned_revision_checked_platform_transaction() {
    let restart = section(
        UI,
        "async fn restart_owned_session",
        "pub(crate) fn toggle_vpn",
    );
    assert!(restart.contains("request_restart_vpn"));
    assert!(restart.contains("config.revisions.config_revision"));
    assert!(!restart.contains("request_stop_vpn"));
    assert!(!restart.contains("request_start_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_restart_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_start_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_stop_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("\"start-vpn\""));
    assert!(PLATFORM_CALLBACKS.contains("request_id: request_id.clone()"));
    assert!(PLATFORM_CALLBACKS.contains("options_json,"));
    assert!(PLATFORM_CALLBACKS.contains("\"stop-vpn\""));
    assert!(PLATFORM_CALLBACKS.contains("VpnStopRequest"));
    assert!(VPN_ABILITY.contains("new PawsVpnConfig(options)"));
    assert!(!VPN_ABILITY.contains("trustedApplications"));
    assert!(!VPN_ABILITY.contains("blockedApplications"));
}

#[test]
fn vpn_bridge_submits_once_and_observes_operations_below_transport_limit() {
    assert!(PLATFORM_CALLBACKS.contains("VPN_OPERATION_WAIT_SLICE_MS: u32 = 45_000"));
    assert!(PLATFORM_CALLBACKS.contains("VPN_OPERATION_TRANSPORT_TIMEOUT_MS: u32 = 55_000"));
    assert!(PLATFORM_CALLBACKS.contains("\"await-vpn-operation\""));
    assert!(PLATFORM_CALLBACKS.contains("\"lookup-vpn-operation\""));
    assert!(PLATFORM_CALLBACKS.contains("VpnOperationLookupRequest"));
    assert!(PLATFORM_CALLBACKS.contains("VPN_REQUEST_NONCE"));
    assert!(PLATFORM_CALLBACKS.contains("VpnOperationOutcome::Unconfirmed"));
    assert!(!PLATFORM_CALLBACKS.contains("const VPN_START_TIMEOUT_MS: u32 = 150_000"));
    assert!(NAPI_TYPES.matches("requestId: string").count() >= 5);
    assert!(NAPI_TYPES.contains("export interface VpnOperationLookupRequest"));
    assert!(NAPI_TYPES.contains("export interface VpnOperationLookupResponse"));
    assert!(VPN_PLUGIN.contains("MAX_RETAINED_VPN_OPERATIONS"));
    assert!(VPN_PLUGIN.contains("bridgeSessionNonce"));
    assert!(VPN_PLUGIN.contains("requestOperations"));
    assert!(VPN_PLUGIN.contains("was reused with a different request"));
    assert!(VPN_PLUGIN.contains("status: 'pending'"));
    assert!(VPN_PLUGIN.contains("status: 'unavailable'"));
    assert!(VPN_PLUGIN.contains("status: 'unknown-session'"));
    let await_operation = section(
        VPN_PLUGIN,
        "private async awaitBridgeOperation",
        "private lookupBridgeOperation",
    );
    assert!(!await_operation.contains("bridgeOperations.delete"));
}

#[test]
fn deleting_the_final_profile_uses_an_owned_revision_checked_stop() {
    let operations = include_str!("../src/ui/operations.rs");
    let delete = section(
        operations,
        "pub(crate) fn delete_profile",
        "pub(crate) fn refresh_profile",
    );
    assert!(delete.contains("request_stop_vpn_if_current"));
    assert!(delete.contains("config.revisions.config_revision"));
    assert!(!delete.contains("request_stop_vpn()"));
    assert!(!delete.contains("runtime_status_projection()"));

    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_stop_vpn_if_current"));
    assert!(PLATFORM_CALLBACKS.contains("\"stop-vpn-if-current\""));
    assert!(PLATFORM_CALLBACKS.contains("VpnOwnedStopRequest"));
}

#[test]
fn cleanup_recovery_requires_confirmed_os_stop_and_released_exact_lease() {
    assert!(NAPI_TYPES.contains("recoverPlatformVpnCleanupAfterConfirmedStop("));
    assert!(NAPI_TYPES.contains("beginPlatformVpnOsStop("));
    assert!(NAPI_TYPES.contains("completePlatformVpnOsStop("));
    assert!(NAPI_TYPES.contains("failPlatformVpnOsStop("));
    assert!(!NAPI_TYPES.contains("currentRecoverablePlatformVpnSessionId(): string"));

    let core_recovery = section(
        CORE,
        "pub async fn recover_platform_vpn_cleanup_after_confirmed_stop",
        "async fn await_platform_vpn_start_with_deadline",
    );
    let compact_recovery = core_recovery
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(core_recovery.contains("PLATFORM_OS_STOP_RECOVERY_DEADLINE"));
    assert!(core_recovery.contains("PLATFORM_OS_STOP_RECOVERY_POLL_INTERVAL"));
    assert!(
        core_recovery
            .matches("platform_owner::observe_owner_lease_exact")
            .count()
            >= 2
    );
    assert!(core_recovery.contains("PlatformVpnOwnerLeaseRole::Issuer"));
    assert!(core_recovery.contains("PlatformVpnOwnerLeaseRole::Extension"));
    assert!(compact_recovery.contains(
        "PlatformVpnOwnerLeaseObservation::Released => { CleanupRecoveryProof::Proven }"
    ));
    assert!(compact_recovery.contains(
        "PlatformVpnOwnerLeaseObservation::HeldExact => { CleanupRecoveryProof::OwnerAlive }"
    ));
    assert!(compact_recovery.contains(
        "PlatformVpnOwnerLeaseObservation::HeldOther => { CleanupRecoveryProof::OwnerLivenessUnknown }"
    ));
    assert!(core_recovery.contains("platform_owner::delete_exact"));

    let held = section(
        &compact_recovery,
        "CleanupRecoveryProof::OwnerAlive => {",
        "CleanupRecoveryProof::OwnerLivenessUnknown => {",
    );
    assert!(held.contains("return Ok(false)"));
    assert!(!held.contains("return Ok(true)"));
    let unknown = compact_recovery
        .rsplit_once("CleanupRecoveryProof::OwnerLivenessUnknown => {")
        .expect("unknown exact-lease observation branch")
        .1;
    assert!(unknown.contains("return Err("));
    assert!(!unknown.contains("return Ok(true)"));
    assert!(!core_recovery.contains("cleanup_recovery_proof_for_identity"));

    let stop = section(VPN_PLUGIN, "private async performStopVpn", "\n  }\n}");
    let cooperative = stop
        .find("pawsUi.awaitPlatformVpnStop(sessionId)")
        .expect("cooperative exact cleanup observation");
    let begin_os_stop = stop
        .find("pawsUi.beginPlatformVpnOsStop(intentEpoch, sessionId)")
        .expect("exact OS-stop dispatch fence");
    let os_stop = stop
        .find("await vpnExtension.stopVpnExtensionAbility")
        .expect("HarmonyOS stop acknowledgement");
    let complete_os_stop = stop
        .find("pawsUi.completePlatformVpnOsStop(intentEpoch, sessionId)")
        .expect("confirmed OS-stop fence completion");
    let recovery = stop
        .find("recoverPlatformVpnCleanupAfterConfirmedStop(sessionId)")
        .expect("exact-lease cleanup recovery");
    assert!(stop.contains("waitForCooperativeCleanup(cleanupObservation)"));
    assert!(stop.contains("pawsUi.failPlatformVpnOsStop(intentEpoch, sessionId)"));
    assert!(cooperative < os_stop);
    assert!(begin_os_stop < os_stop);
    assert!(os_stop < complete_os_stop);
    assert!(complete_os_stop < recovery);

    let start = section(
        VPN_PLUGIN,
        "private async performStartVpn",
        "private async ensureSpeedNotificationPermission",
    );
    let orphan = start
        .find("claimCurrentPlatformVpnStop(intentEpoch)")
        .expect("durable owner claim");
    let cleanup = start
        .find("performStopVpn(recoverableSessionId, intentEpoch)")
        .expect("orphan OS cleanup");
    let begin = start
        .find("beginPlatformVpnStartForIntent(intentEpoch)")
        .expect("replacement start");
    assert!(orphan < cleanup);
    assert!(cleanup < begin);
}
