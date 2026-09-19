#[path = "../src/settings_draft.rs"]
mod settings_draft;
use settings_draft::*;

fn baseline(profile: &str, revision: u64) -> SettingsBaseline {
    SettingsBaseline {
        profile_id: Some(profile.into()),
        revision,
        values: SettingsValues {
            dns: DnsDraft {
                servers: "1.1.1.1".into(),
                fallbacks: String::new(),
                policy: String::new(),
            },
            vpn: VpnDraft {
                system_proxy: false,
                dns_hijacking: true,
                allow_bypass: false,
                stack: "lwip".into(),
            },
            network: NetworkDraft {
                mixed_port: "7890".into(),
                controller_port: "9090".into(),
                mixed_enabled: false,
                controller_enabled: false,
                allow_lan: false,
            },
        },
    }
}

#[test]
fn unedited_form_follows_external_changes() {
    let mut form = SettingsDraft::new(baseline("a", 1));
    let mut next = baseline("a", 2);
    next.values.network.mixed_port = "7900".into();
    form.observe(next.clone());
    assert_eq!(form.values, next.values);
    assert!(!form.dirty(SettingsSection::Network));
}

#[test]
fn profile_switch_cannot_redirect_an_existing_draft() {
    let mut form = SettingsDraft::new(baseline("a", 1));
    form.values.dns.servers = "8.8.8.8".into();
    form.observe(baseline("b", 2));
    assert_eq!(form.baseline.profile_id.as_deref(), Some("a"));
    assert_eq!(form.values.dns.servers, "8.8.8.8");
    assert!(form.begin(SettingsSection::Dns).is_none());
    form.reload();
    assert_eq!(form.baseline.profile_id.as_deref(), Some("b"));
    assert_eq!(form.values.dns.servers, "1.1.1.1");
}

#[test]
fn pending_save_is_exclusive_and_preserves_other_section_edits() {
    let mut form = SettingsDraft::new(baseline("a", 1));
    form.values.dns.servers = "8.8.8.8".into();
    form.values.network.allow_lan = true;
    let (profile, revision, _) = form.begin(SettingsSection::Dns).unwrap();
    assert_eq!((profile.as_str(), revision), ("a", 1));
    assert!(form.begin(SettingsSection::Network).is_none());
    let mut applied = baseline("a", 2);
    applied.values.dns.servers = "8.8.8.8".into();
    form.observe(applied.clone());
    form.finish(SettingsSection::Dns, applied);
    assert!(!form.dirty(SettingsSection::Dns));
    assert!(form.dirty(SettingsSection::Network));
    assert!(form.values.network.allow_lan);
    assert_eq!(form.begin(SettingsSection::Network).unwrap().1, 2);
}

#[test]
fn failed_save_keeps_edits_and_releases_pending() {
    let mut form = SettingsDraft::new(baseline("a", 1));
    form.values.vpn.system_proxy = true;
    form.begin(SettingsSection::Vpn).unwrap();
    form.fail("storage rejected write".into());
    assert!(form.pending.is_none());
    assert!(form.values.vpn.system_proxy);
    assert_eq!(form.error.as_deref(), Some("storage rejected write"));
}

#[test]
fn late_save_receipt_cannot_erase_a_newer_profile_conflict() {
    let mut form = SettingsDraft::new(baseline("a", 1));
    form.values.vpn.system_proxy = true;
    form.begin(SettingsSection::Vpn).unwrap();
    form.observe(baseline("b", 3));
    let mut receipt = baseline("a", 2);
    receipt.values.vpn.system_proxy = true;
    form.finish(SettingsSection::Vpn, receipt.clone());
    assert_eq!(form.baseline, receipt);
    assert_eq!(
        form.incoming.as_ref().unwrap().profile_id.as_deref(),
        Some("b")
    );
    form.observe(receipt);
    assert_eq!(form.incoming.as_ref().unwrap().revision, 3);
    form.reload();
    assert_eq!(form.baseline.profile_id.as_deref(), Some("b"));
}
