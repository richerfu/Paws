const VPN_PLUGIN: &str = include_str!("../../../entry/src/main/ets/plugins/VpnPlugin.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets");
const CONTINUOUS_TASK: &str =
    include_str!("../../../entry/src/main/ets/vpnability/VpnContinuousTask.ets");

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
    assert!(VPN_ABILITY.contains("pawsUi.persistVpnTelemetry()"));
    assert!(CONTINUOUS_TASK.contains("traffic.downloadSpeed"));
    assert!(CONTINUOUS_TASK.contains("traffic.uploadSpeed"));
    assert!(CONTINUOUS_TASK.contains("↓ ${formatSpeed"));
    assert!(CONTINUOUS_TASK.contains("↑ ${formatSpeed"));
    assert!(CONTINUOUS_TASK.contains("pawsUi.queryVpnSpeed()"));
}

#[test]
fn vpn_has_one_system_owned_background_notification_path() {
    assert!(!VPN_ABILITY.contains("@kit.NotificationKit"));
    assert!(!VPN_ABILITY.contains("notificationManager.publish"));
    assert!(CONTINUOUS_TASK.contains("id: task.notificationId"));
    assert!(CONTINUOUS_TASK.contains("isOngoing: true"));
    assert!(CONTINUOUS_TASK.contains("notificationManager.publish(request)"));
    assert!(CONTINUOUS_TASK.contains("stopBackgroundRunning(context)"));
    assert!(VPN_PLUGIN.contains("continuous task protection ended; VPN remains active"));
    assert!(!VPN_PLUGIN.contains("system cancelled continuous VPN task; stopping VPN"));
}

#[test]
fn vpn_notification_click_opens_the_entry_ability() {
    assert!(CONTINUOUS_TASK.contains("wantAgent.getWantAgent"));
    assert!(CONTINUOUS_TASK.contains("actionType: wantAgent.OperationType.START_ABILITY"));
    assert!(CONTINUOUS_TASK.contains("moduleName: PAWS_MODULE_NAME"));
    assert!(CONTINUOUS_TASK.contains("abilityName: PAWS_ENTRY_ABILITY"));
    assert!(CONTINUOUS_TASK.contains("wantAgent: agent"));
    assert!(CONTINUOUS_TASK.contains("WantAgentFlags.UPDATE_PRESENT_FLAG"));
}
