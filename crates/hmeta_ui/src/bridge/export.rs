//! `paws.export` bridge plugin: export text to a user-chosen file.
//!
//! ArkTS side owns the `DocumentViewPicker.save` flow with a pre-filled
//! suggested file name (which the built-in `ohos.files` plugin does not
//! support) plus the file write.

use arkit::napi_derive_ohos::napi;
use arkit::openharmony_ability::{
    impl_bridge_napi_type, AsyncBridge, BridgeContextRequirement, BridgePlugin,
};

pub struct PawsExportBridgePlugin;

impl BridgePlugin for PawsExportBridgePlugin {
    type Mode = AsyncBridge;

    const ID: &'static str = "paws.export";
    const REQUIRED_CONTEXTS: &'static [BridgeContextRequirement] =
        &[BridgeContextRequirement::Ability];
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ExportTextRequest {
    /// `"profile"` or `"log"`; selects the fallback name, extension and
    /// suffix choices on the ArkTS side.
    pub export_kind: String,
    pub suggested_name: String,
    pub content: String,
}

impl_bridge_napi_type!(ExportTextRequest, "paws.ExportTextRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ExportTextResponse {}

impl_bridge_napi_type!(ExportTextResponse, "paws.ExportTextResponse");

#[cfg(test)]
mod tests {
    use super::{ExportTextRequest, ExportTextResponse};
    use arkit::openharmony_ability::BridgeNapiType;

    #[test]
    fn export_uses_stable_named_napi_contracts() {
        assert_eq!(
            <ExportTextRequest as BridgeNapiType>::TYPE_NAME,
            "paws.ExportTextRequest"
        );
        assert_eq!(
            <ExportTextResponse as BridgeNapiType>::TYPE_NAME,
            "paws.ExportTextResponse"
        );
    }
}
