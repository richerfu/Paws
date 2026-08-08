//! `paws.scan` bridge plugin: QR-code scan of a subscription link.
//!
//! ArkTS side owns the ScanKit platform call (`scanBarcode.startScanForResult`
//! on the Ability context); Rust only receives the trimmed scan payload.

use arkit::napi_derive_ohos::napi;
use arkit::openharmony_ability::{
    impl_bridge_napi_type, AsyncBridge, BridgeContextRequirement, BridgePlugin,
};

pub struct PawsScanBridgePlugin;

impl BridgePlugin for PawsScanBridgePlugin {
    type Mode = AsyncBridge;

    const ID: &'static str = "paws.scan";
    const REQUIRED_CONTEXTS: &'static [BridgeContextRequirement] =
        &[BridgeContextRequirement::Ability];
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ScanRequest {}

impl_bridge_napi_type!(ScanRequest, "paws.ScanRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ScanResponse {
    /// Trimmed scan payload; empty when the user cancelled the scan dialog.
    pub content: String,
}

impl_bridge_napi_type!(ScanResponse, "paws.ScanResponse");

#[cfg(test)]
mod tests {
    use super::{ScanRequest, ScanResponse};
    use arkit::openharmony_ability::BridgeNapiType;

    #[test]
    fn scan_uses_stable_named_napi_contracts() {
        assert_eq!(
            <ScanRequest as BridgeNapiType>::TYPE_NAME,
            "paws.ScanRequest"
        );
        assert_eq!(
            <ScanResponse as BridgeNapiType>::TYPE_NAME,
            "paws.ScanResponse"
        );
    }
}
