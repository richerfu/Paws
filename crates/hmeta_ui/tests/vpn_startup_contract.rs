const UI: &str = concat!(
    include_str!("../src/ui.rs"),
    include_str!("../src/ui/tasks.rs")
);
const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/HMetaVpnExtensionAbility.ets");
const VPN_CONFIG: &str = include_str!("../../../entry/src/main/ets/vpnability/VpnConfig.ets");
const NAPI_TYPES: &str = include_str!("../../../entry/src/main/cpp/types/libhmeta_ui/Index.d.ts");
const PLATFORM_CALLBACKS: &str = include_str!("../src/platform_callbacks.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn vpn_start_does_not_reload_the_already_active_profile_or_wait_a_fixed_delay() {
    let start = section(
        UI,
        "async fn start_vpn_command_and_snapshot",
        "async fn stop_vpn_command_and_snapshot",
    );

    assert!(start.contains("active_profile.as_deref() != Some(profile_id.as_str())"));
    assert!(!start.contains("Duration::from_millis(350)"));
    assert!(UI.contains("delayed_vpn_snapshot"));
    assert!(UI.contains("Duration::from_millis(200)"));
}

#[test]
fn notification_permission_is_deferred_until_after_the_vpn_ability_launches() {
    let request = section(
        ENTRY_ABILITY,
        "private async requestStartVpn",
        "private async ensureSpeedNotificationPermission",
    );
    let launch = request
        .find("vpnExtension.startVpnExtensionAbility")
        .expect("VPN ability launch");
    let permission = request
        .find("this.ensureSpeedNotificationPermission()")
        .expect("deferred notification permission");

    assert!(launch < permission);
    assert!(request.contains("reusing pending VPN start request"));
}

#[test]
fn first_authorization_start_is_coordinated_by_the_extension_terminal_state() {
    assert!(NAPI_TYPES.contains("beginPlatformVpnStart(): string"));
    assert!(NAPI_TYPES.contains("bindPlatformVpnStart(attemptId: string): void"));
    assert!(NAPI_TYPES.contains("awaitPlatformVpnStartAttachment("));
    assert!(NAPI_TYPES.contains("awaitPlatformVpnStart(attemptId: string): Promise<string>"));
    assert!(NAPI_TYPES.contains("failUnattachedPlatformVpnStart("));

    let request = section(
        ENTRY_ABILITY,
        "private async requestStartVpn",
        "private async ensureSpeedNotificationPermission",
    );
    assert!(request.contains("beginPlatformVpnStart()"));
    assert!(request.contains("awaitPlatformVpnStart(attemptId)"));
    assert!(request.contains("failUnattachedPlatformVpnStart(attemptId, message)"));
    assert!(request.contains("buildVpnWant(optionsJson, this.platformSharedMemory, attemptId)"));
    assert!(request.contains("awaitPlatformVpnStartAttachment"));
    assert!(request.contains("redispatching attempt"));
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
fn descriptor_free_authorization_bootstrap_waits_for_the_rebound_want() {
    let start = section(
        VPN_ABILITY,
        "private startFromWant",
        "private recordVpnFailure",
    );
    let bootstrap = section(
        start,
        "const sharedMemory = readPlatformSharedMemoryFds(want)",
        "try {\n      hmetaUi.attachPlatformSharedMemory",
    );

    assert!(bootstrap.contains("authorization bootstrap"));
    assert!(bootstrap.contains("waiting for rebound request"));
    assert!(!bootstrap.contains("recordVpnFailure"));
}

#[test]
fn first_authorization_want_unwraps_nested_parameters_and_descriptors() {
    assert!(VPN_CONFIG.contains("const HMETA_SYSTEM_PARAMETERS_KEY = 'myParams'"));
    assert!(VPN_CONFIG.contains("function readVpnParameters"));
    assert!(VPN_CONFIG.contains("readFileDescriptorParameter"));
    assert!(VPN_CONFIG.contains("readPlatformStartAttemptId"));
}

#[test]
fn routine_ui_refresh_does_not_compete_for_the_start_notification_waiter() {
    let delayed = section(
        UI,
        "async fn delayed_snapshot",
        "async fn delayed_vpn_snapshot",
    );
    assert!(delayed.contains("tokio::time::sleep"));
    assert!(!delayed.contains("wait_for_platform_change"));
}

#[test]
fn native_profile_prepare_overlaps_tun_creation() {
    assert!(NAPI_TYPES.contains("prepareVpn(): Promise<boolean>"));
    let start = section(
        VPN_ABILITY,
        "private startFromWant",
        "private recordVpnFailure",
    );
    let prepare = start.find("hmetaUi.prepareVpn()").expect("native prepare");
    let create = start.find(".create(config)").expect("TUN creation");
    let await_prepare = start
        .find("await nativePrepare")
        .expect("native prepare await");
    let native_start = start.find("hmetaUi.startVpn").expect("native VPN start");

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
        include_str!("../../hmeta_core/src/controller.rs"),
        "async fn load_meow_config",
        "fn tunnel_from_config",
    );
    assert!(loader.contains("tokio::task::spawn_blocking"));

    let window = section(
        ENTRY_ABILITY,
        "public async onWindowStageCreate",
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
    assert!(PLATFORM_CALLBACKS
        .contains("invoke_string_void_callback(tsfn, options_json, \"VPN start\").await"));
    assert!(PLATFORM_CALLBACKS.contains("invoke_void_callback(tsfn, \"VPN stop\").await"));
    assert!(VPN_ABILITY.contains("new HMetaVpnConfig(options)"));
    assert!(!VPN_ABILITY.contains("trustedApplications"));
    assert!(!VPN_ABILITY.contains("blockedApplications"));
}
