const APP_PROFILE: &str = include_str!("../../../AppScope/app.json5");
const VPN_CONFIG: &str = include_str!("../../../entry/src/main/ets/vpnability/VpnConfig.ets");
const ENTRY_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/entryability/EntryAbility.ets");
const VPN_ABILITY: &str =
    include_str!("../../../entry/src/main/ets/vpnability/HMetaVpnExtensionAbility.ets");
const BACKUP_PROFILE: &str =
    include_str!("../../../entry/src/main/resources/base/profile/backup_config.json");

#[test]
fn harmony_bundle_and_private_storage_use_paws_branding() {
    assert!(APP_PROFILE.contains("com.richerfu.paws"));
    assert!(VPN_CONFIG.contains("com.richerfu.paws"));
    assert!(ENTRY_ABILITY.contains("filesDir}/paws"));
    assert!(VPN_ABILITY.contains("filesDir}/paws"));
    assert!(BACKUP_PROFILE.contains("base/files/paws"));

    for source in [
        APP_PROFILE,
        VPN_CONFIG,
        ENTRY_ABILITY,
        VPN_ABILITY,
        BACKUP_PROFILE,
    ] {
        assert!(!source.contains("com.richerfu.clash_hmeta"));
        assert!(!source.contains("filesDir}/hmeta"));
        assert!(!source.contains("base/files/hmeta"));
    }
}
