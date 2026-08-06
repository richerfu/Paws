//! `paws.clipboard` bridge plugin: copy text to the system pasteboard.

use arkit::napi_derive_ohos::napi;
use arkit::openharmony_ability::{
    impl_bridge_napi_type, AsyncBridge, BridgeContextRequirement, BridgePlugin,
};

pub struct PawsClipboardBridgePlugin;

impl BridgePlugin for PawsClipboardBridgePlugin {
    type Mode = AsyncBridge;

    const ID: &'static str = "paws.clipboard";
    const REQUIRED_CONTEXTS: &'static [BridgeContextRequirement] =
        &[BridgeContextRequirement::Ability];
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ClipboardSetRequest {
    pub text: String,
}

impl_bridge_napi_type!(ClipboardSetRequest, "paws.ClipboardSetRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ClipboardSetResponse {}

impl_bridge_napi_type!(ClipboardSetResponse, "paws.ClipboardSetResponse");

#[cfg(test)]
mod tests {
    use super::{ClipboardSetRequest, ClipboardSetResponse};
    use arkit::openharmony_ability::BridgeNapiType;

    #[test]
    fn clipboard_uses_stable_named_napi_contracts() {
        assert_eq!(
            <ClipboardSetRequest as BridgeNapiType>::TYPE_NAME,
            "paws.ClipboardSetRequest"
        );
        assert_eq!(
            <ClipboardSetResponse as BridgeNapiType>::TYPE_NAME,
            "paws.ClipboardSetResponse"
        );
    }
}
