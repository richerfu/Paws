const VPN_PLUGIN: &str = include_str!("../../../entry/src/main/ets/plugins/VpnPlugin.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/HMetaVpnExtensionAbility.ets");

#[test]
fn vpn_start_requests_notification_access_without_blocking_vpn() {
    assert!(VPN_PLUGIN.contains("ensureSpeedNotificationPermission"));
    assert!(VPN_PLUGIN.contains("notificationManager.isNotificationEnabled"));
    assert!(VPN_PLUGIN
        .contains("notificationManager.requestEnableNotification(context.abilityContext)"));
    assert!(VPN_PLUGIN.contains("Notification permission must never prevent"));
}

#[test]
fn vpn_notification_uses_live_download_and_upload_speeds() {
    assert!(VPN_ABILITY.contains("hmetaUi.persistVpnTelemetry()"));
    assert!(VPN_ABILITY.contains("snapshot.traffic?.downloadSpeed"));
    assert!(VPN_ABILITY.contains("snapshot.traffic?.uploadSpeed"));
    assert!(VPN_ABILITY.contains("↓ ${formatSpeed"));
    assert!(VPN_ABILITY.contains("↑ ${formatSpeed"));
}

#[test]
fn vpn_notification_is_updated_in_place_and_removed_on_stop() {
    assert!(VPN_ABILITY.contains("id: VPN_SPEED_NOTIFICATION_ID"));
    assert!(VPN_ABILITY.contains("isOngoing: true"));
    assert!(VPN_ABILITY.contains("isUnremovable: true"));
    assert!(VPN_ABILITY.contains("notificationManager.publish(request)"));
    assert!(VPN_ABILITY.contains("notificationManager.cancel(VPN_SPEED_NOTIFICATION_ID)"));
    assert!(VPN_ABILITY.contains("this.stopSpeedNotification()"));
    assert!(VPN_ABILITY.contains("this.persistTelemetry(false)"));
}

#[test]
fn vpn_notification_click_opens_the_entry_ability() {
    assert!(VPN_ABILITY.contains("wantAgent.getWantAgent"));
    assert!(VPN_ABILITY.contains("actionType: wantAgent.OperationType.START_ABILITY"));
    assert!(VPN_ABILITY.contains("moduleName: HMETA_MODULE_NAME"));
    assert!(VPN_ABILITY.contains("abilityName: HMETA_ENTRY_ABILITY"));
    assert!(VPN_ABILITY.contains("wantAgent: notificationWantAgent"));
    assert!(VPN_ABILITY.contains("WantAgentFlags.UPDATE_PRESENT_FLAG"));
}
