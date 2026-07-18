const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/HMetaVpnExtensionAbility.ets");

#[test]
fn vpn_start_requests_notification_access_without_blocking_vpn() {
    assert!(ENTRY_ABILITY.contains("ensureSpeedNotificationPermission"));
    assert!(ENTRY_ABILITY.contains("notificationManager.isNotificationEnabled"));
    assert!(ENTRY_ABILITY.contains("notificationManager.requestEnableNotification(this.context)"));
    assert!(ENTRY_ABILITY.contains("Notification permission must never prevent"));
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
