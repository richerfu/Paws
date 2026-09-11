//! `paws.vpn` bridge plugin: system VPN extension control.
//!
//! ArkTS side owns the VPN start orchestration: the typed intent fence,
//! reconciliation of an older journaled owner, `startVpnExtensionAbility`,
//! the first-authorization redispatch, and the final `awaitPlatformVpnStart`
//! outcome. Rust submits one idempotent request and observes its receipt.

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
    /// Rust-process-scoped idempotency key. It lets the caller recover the
    /// operation receipt if the submit response is lost.
    pub request_id: String,
    /// Serialized `VpnOptions` JSON for the extension Want.
    pub options_json: String,
}

impl_bridge_napi_type!(VpnStartRequest, "paws.VpnStartRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStartResponse {
    /// Ability-session-scoped identifier used to observe this one submitted
    /// operation without ever submitting it a second time.
    pub operation_id: String,
}

impl_bridge_napi_type!(VpnStartResponse, "paws.VpnStartResponse");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStopRequest {
    pub request_id: String,
}

impl_bridge_napi_type!(VpnStopRequest, "paws.VpnStopRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnStopResponse {
    pub operation_id: String,
}

impl_bridge_napi_type!(VpnStopResponse, "paws.VpnStopResponse");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOwnedStopRequest {
    pub request_id: String,
    pub expected_session_id: String,
    /// Decimal u64, without loss through a JavaScript number.
    pub expected_config_revision: String,
}
impl_bridge_napi_type!(VpnOwnedStopRequest, "paws.VpnOwnedStopRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOwnedStopResponse {
    pub operation_id: String,
}
impl_bridge_napi_type!(VpnOwnedStopResponse, "paws.VpnOwnedStopResponse");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnRestartRequest {
    pub request_id: String,
    pub expected_session_id: String,
    /// Decimal u64, without loss through a JavaScript number.
    pub expected_config_revision: String,
    pub options_json: String,
}
impl_bridge_napi_type!(VpnRestartRequest, "paws.VpnRestartRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnRestartResponse {
    pub operation_id: String,
}
impl_bridge_napi_type!(VpnRestartResponse, "paws.VpnRestartResponse");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOperationLookupRequest {
    pub request_id: String,
}
impl_bridge_napi_type!(VpnOperationLookupRequest, "paws.VpnOperationLookupRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOperationLookupResponse {
    /// `found` or `unknown-session`. An unknown request is never evidence
    /// that an earlier submit did not reach a previous Ability session.
    pub status: String,
    pub operation_id: String,
    /// `pending`, `succeeded`, `failed`, or `unknown`.
    pub operation_status: String,
    pub result: bool,
    pub error: String,
}
impl_bridge_napi_type!(
    VpnOperationLookupResponse,
    "paws.VpnOperationLookupResponse"
);

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOperationWaitRequest {
    pub operation_id: String,
    /// One bounded long-poll slice. The ArkTS plugin resolves `pending`
    /// before the transport's fixed 60-second maximum.
    pub wait_ms: u32,
}
impl_bridge_napi_type!(VpnOperationWaitRequest, "paws.VpnOperationWaitRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct VpnOperationWaitResponse {
    /// `pending`, `succeeded`, `failed`, or `unavailable`.
    pub status: String,
    /// Meaningful for the owned-stop/restart result; true for ordinary
    /// start/stop completion.
    pub result: bool,
    pub error: String,
}
impl_bridge_napi_type!(VpnOperationWaitResponse, "paws.VpnOperationWaitResponse");

#[cfg(test)]
mod tests {
    use super::{
        VpnOperationLookupRequest, VpnOperationLookupResponse, VpnOperationWaitRequest,
        VpnOperationWaitResponse, VpnOwnedStopRequest, VpnOwnedStopResponse, VpnRestartRequest,
        VpnRestartResponse, VpnStartRequest, VpnStartResponse, VpnStopRequest, VpnStopResponse,
    };
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
        assert_eq!(
            <VpnOwnedStopRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOwnedStopRequest"
        );
        assert_eq!(
            <VpnOwnedStopResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOwnedStopResponse"
        );
        assert_eq!(
            <VpnRestartRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnRestartRequest"
        );
        assert_eq!(
            <VpnRestartResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnRestartResponse"
        );
        assert_eq!(
            <VpnOperationLookupRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOperationLookupRequest"
        );
        assert_eq!(
            <VpnOperationLookupResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOperationLookupResponse"
        );
        assert_eq!(
            <VpnOperationWaitRequest as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOperationWaitRequest"
        );
        assert_eq!(
            <VpnOperationWaitResponse as BridgeNapiType>::TYPE_NAME,
            "paws.VpnOperationWaitResponse"
        );
    }
}
