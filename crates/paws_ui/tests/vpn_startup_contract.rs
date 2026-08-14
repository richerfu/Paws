const UI: &str = concat!(
    include_str!("../src/ui.rs"),
    include_str!("../src/ui/tasks.rs")
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
        "async fn start_vpn_command_and_snapshot",
        "async fn stop_vpn_command_and_snapshot",
    );

    assert!(start.contains("active_profile.as_deref() != Some(profile_id.as_str())"));
    assert!(!start.contains("Duration::from_millis(350)"));
    assert!(UI.contains("await_vpn_state_event"));
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
    assert!(NAPI_TYPES.contains("beginPlatformVpnStart(): string"));
    assert!(NAPI_TYPES.contains("bindPlatformVpnStart(attemptId: string): void"));
    assert!(NAPI_TYPES.contains("awaitPlatformVpnStart(attemptId: string): Promise<string>"));
    assert!(NAPI_TYPES.contains("failUnattachedPlatformVpnStart("));
    assert!(!NAPI_TYPES.contains("awaitPlatformVpnStartAttachment"));

    let request = section(
        VPN_PLUGIN,
        "async requestStartVpn",
        "private async requestStopVpnWithContext",
    );
    assert!(request.contains("beginPlatformVpnStart()"));
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
        "private recordVpnFailure",
    );
    let attach = extension
        .find("attachPlatformSharedMemory")
        .expect("ashmem attachment");
    let bind = extension
        .find("bindPlatformVpnStart")
        .expect("attempt binding");
    let running = extension
        .find("setPlatformVpnRunning")
        .expect("terminal state");
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
        "private startFromWant",
        "private recordVpnFailure",
    );
    let bootstrap = section(
        start,
        "const sharedMemory = readPlatformSharedMemoryFds(want)",
        "try {\n      pawsUi.attachPlatformSharedMemory",
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
    assert!(NAPI_TYPES.contains("prepareVpn(): Promise<boolean>"));
    let start = section(
        VPN_ABILITY,
        "private startFromWant",
        "private recordVpnFailure",
    );
    let prepare = start.find("pawsUi.prepareVpn()").expect("native prepare");
    let create = start.find(".create(config)").expect("TUN creation");
    let await_prepare = start
        .find("await nativePrepare")
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
    assert!(!create.contains("prepareVpn"));
    assert!(!create.contains("prepareNativeProfileForFirstFrame"));

    let bootstrap = section(
        UI,
        "async fn bootstrap_active_profile",
        "fn reconcile_vpn_command",
    );
    assert!(bootstrap.contains("core.prepare_active_vpn().await"));
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
    assert!(!window.contains("prepareVpn"));
    assert!(!window.contains("firstFramePreparation"));
}

#[test]
fn vpn_restart_waits_for_platform_stop_before_starting_with_new_options() {
    let restart = section(
        UI,
        "async fn request_vpn_restart_if_running",
        "fn dns_draft_from_snapshot",
    );
    let stop = restart
        .find("request_stop_vpn().await")
        .expect("awaited VPN stop");
    let start = restart
        .find("request_start_vpn(options_json).await")
        .expect("awaited VPN start");

    assert!(stop < start);
    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_start_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("pub(crate) async fn request_stop_vpn"));
    assert!(PLATFORM_CALLBACKS.contains("\"start-vpn\""));
    assert!(PLATFORM_CALLBACKS.contains("VpnStartRequest { options_json }"));
    assert!(PLATFORM_CALLBACKS.contains("\"stop-vpn\""));
    assert!(PLATFORM_CALLBACKS.contains("VpnStopRequest"));
    assert!(VPN_ABILITY.contains("new PawsVpnConfig(options)"));
    assert!(!VPN_ABILITY.contains("trustedApplications"));
    assert!(!VPN_ABILITY.contains("blockedApplications"));
}
