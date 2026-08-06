//! `paws.color-mode` bridge plugin: apply the app-wide color mode on the
//! Ability context (`context.setColorMode`).

use arkit::napi_derive_ohos::napi;
use arkit::openharmony_ability::{
    impl_bridge_napi_type, AsyncBridge, BridgeContextRequirement, BridgePlugin,
};

pub struct PawsColorModeBridgePlugin;

impl BridgePlugin for PawsColorModeBridgePlugin {
    type Mode = AsyncBridge;

    const ID: &'static str = "paws.color-mode";
    const REQUIRED_CONTEXTS: &'static [BridgeContextRequirement] =
        &[BridgeContextRequirement::Ability];
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ColorModeRequest {
    /// `ConfigurationConstant.ColorMode` value pushed from native UI.
    pub mode: i32,
}

impl_bridge_napi_type!(ColorModeRequest, "paws.ColorModeRequest");

#[napi(object)]
#[derive(Clone, Debug)]
pub struct ColorModeResponse {}

impl_bridge_napi_type!(ColorModeResponse, "paws.ColorModeResponse");

#[cfg(test)]
mod tests {
    use super::{ColorModeRequest, ColorModeResponse};
    use arkit::openharmony_ability::BridgeNapiType;

    #[test]
    fn color_mode_uses_stable_named_napi_contracts() {
        assert_eq!(
            <ColorModeRequest as BridgeNapiType>::TYPE_NAME,
            "paws.ColorModeRequest"
        );
        assert_eq!(
            <ColorModeResponse as BridgeNapiType>::TYPE_NAME,
            "paws.ColorModeResponse"
        );
    }
}
