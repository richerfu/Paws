use super::*;

#[test]
fn vpn_stack_parses_only_runnable_backends() {
    assert_eq!(
        VpnStack::try_from("netstack-smoltcp").unwrap(),
        VpnStack::Smoltcp
    );
    assert_eq!(VpnStack::try_from("smoltcp").unwrap(), VpnStack::Smoltcp);
    assert_eq!(VpnStack::try_from(" LWIP ").unwrap(), VpnStack::Lwip);
    assert!(VpnStack::try_from("gvisor").is_err());
    assert_eq!(SUPPORTED_VPN_STACKS, &[VpnStack::Smoltcp, VpnStack::Lwip]);
}

#[test]
fn default_vpn_options_omit_per_app_system_fields() {
    let json = to_json(&VpnOptions::default()).unwrap();
    assert!(!json.contains("perAppMode"));
    assert!(!json.contains("trustedApplications"));
    assert!(!json.contains("blockedApplications"));
}
