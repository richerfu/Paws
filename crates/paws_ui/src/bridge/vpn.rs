//! `paws.vpn` bridge plugin: system VPN extension control.
//!
//! ArkTS side owns the VPN start orchestration: `beginPlatformVpnStart`,
//! `startVpnExtensionAbility`, the first-authorization redispatch, and the
//! final `awaitPlatformVpnStart` outcome. Rust only submits the options JSON
//! and waits for the outcome.

use arkit::napi_derive_ohos::napi;
use arkit::openharmony_ability::{
    impl_bridge_napi_type, AsyncBridge, BridgeContextRequirement, BridgePlugin,
};

pub struct PawsVpnBridgePlugin;

impl BridgePlugin for PawsVpnBridgePlugin {
    type Mode = AsyncBridge;

    const ID: &'static str = "paws.vpn";
    const REQUIRED_CONTEXTS: &'static [BridgeContextRequirement] =
        &[BridgeContextRequirement::Ability];
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStartRequest {
    /// Serialized `VpnOptions` JSON for the extension Want.
    pub options_json: String,
}

impl_bridge_napi_type!(VpnStartRequest, "paws.VpnStartRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStartResponse {}

impl_bridge_napi_type!(VpnStartResponse, "paws.VpnStartResponse");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStopRequest {}

impl_bridge_napi_type!(VpnStopRequest, "paws.VpnStopRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStopResponse {}

impl_bridge_napi_type!(VpnStopResponse, "paws.VpnStopResponse");

#[cfg(test)]
mod tests {
    use super::{VpnStartRequest, VpnStartResponse, VpnStopRequest, VpnStopResponse};
    use arkit::openharmony_ability::BridgeNapiType;

    #[test]
    fn vpn_uses_stable_named_napi_contracts() {
        assert_eq!(
            <VpnStartRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnStartRequest"
        );
        assert_eq!(
            <VpnStartResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnStartResponse"
        );
        assert_eq!(
            <VpnStopRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnStopRequest"
        );
        assert_eq!(
            <VpnStopResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnStopResponse"
        );
    }
}
