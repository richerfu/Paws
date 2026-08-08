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

#[test]
fn network_ports_have_safe_defaults_and_reject_conflicts() {
    assert_eq!(
        NetworkPortConfig::default(),
        NetworkPortConfig {
            mixed_port: 7890,
            controller_port: 9090,
        }
    );
    assert!(NetworkPortConfig {
        mixed_port: 17890,
        controller_port: 19090,
    }
    .validate()
    .is_ok());
    assert!(NetworkPortConfig {
        mixed_port: 9090,
        controller_port: 9090,
    }
    .validate()
    .is_err());
    assert!(NetworkPortConfig {
        mixed_port: 80,
        controller_port: 9090,
    }
    .validate()
    .is_err());
}
