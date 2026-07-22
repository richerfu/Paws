const UI: &str = include_str!("../src/ui.rs");
const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/HMetaVpnExtensionAbility.ets");
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
