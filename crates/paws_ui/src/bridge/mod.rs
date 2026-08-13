//! Application-owned ArkTS bridge plugins and the Rust call surface.
//!
//! Paws is a HarmonyOS-only app. Platform capabilities that the
//! openharmony-ability built-in plugins do not cover (QR scan, clipboard,
//! app color mode, VPN extension control, exports with a pre-filled name) are
//! implemented here as `paws.*` bridge plugins: the ArkTS side owns the
//! platform objects, Rust submits named N-API values and awaits the outcome.
//! The helpers below are the single call surface used by the native UI.

mod clipboard;
mod color_mode;
mod export;
mod safe_area;
mod scan;
mod vpn;

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use arkit::napi_ohos::{Error, Result};
use arkit::openharmony_ability::{AsyncBridge, BridgeCallOptions, BridgePlugin, OpenHarmonyApp};
use openharmony_ability_plugin_files::{
    dialog_type, FileDialogFilter, FileDialogOptions, FilesExt,
};
use openharmony_ability_plugin_url::UrlExt;

pub(crate) use self::clipboard::{
    ClipboardSetRequest, ClipboardSetResponse, PawsClipboardBridgePlugin,
};
pub(crate) use self::color_mode::{ColorModeRequest, ColorModeResponse, PawsColorModeBridgePlugin};
pub(crate) use self::export::{ExportTextRequest, ExportTextResponse, PawsExportBridgePlugin};
pub(crate) use self::safe_area::{initial_safe_area, InitialSafeArea, PawsSafeAreaBridgePlugin};
pub(crate) use self::scan::{PawsScanBridgePlugin, ScanRequest, ScanResponse};
pub(crate) use self::vpn::{
    PawsVpnBridgePlugin, VpnStartRequest, VpnStartResponse, VpnStopRequest, VpnStopResponse,
};

/// Rust-side handle of the current Ability session, installed by `init`.
static INNER_APP: LazyLock<RwLock<Option<OpenHarmonyApp>>> = LazyLock::new(|| RwLock::new(None));

/// VPN start runs the full attach/redispatch dance on the ArkTS side; give it
/// a generous budget so a slow first-connection does not look like a failure.
const VPN_START_TIMEOUT_MS: u32 = 60_000;

pub(crate) fn set_app(app: OpenHarmonyApp) {
    *INNER_APP
        .write()
        .expect("INNER_APP write lock must not fail") = Some(app);
}

fn current_app() -> Result<OpenHarmonyApp> {
    INNER_APP
        .read()
        .expect("INNER_APP read lock must not fail")
        .as_ref()
        .cloned()
        .ok_or_else(|| Error::from_reason("OpenHarmony app not initialized"))
}

async fn call_async<P, R, S>(action: &str, request: R) -> std::result::Result<S, String>
where
    P: BridgePlugin<Mode = AsyncBridge>,
    R: arkit::openharmony_ability::BridgeNapiType,
    S: arkit::openharmony_ability::BridgeNapiType,
{
    let app = current_app().map_err(|err| err.to_string())?;
    let bridge = app.bridge().map_err(|err| err.to_string())?;
    bridge
        .call_async::<P, R, S>(action, request, BridgeCallOptions::default())
        .await
        .map_err(|err| err.to_string())
}

async fn call_async_with_timeout<P, R, S>(
    action: &str,
    request: R,
    timeout_ms: u32,
) -> std::result::Result<S, String>
where
    P: BridgePlugin<Mode = AsyncBridge>,
    R: arkit::openharmony_ability::BridgeNapiType,
    S: arkit::openharmony_ability::BridgeNapiType,
{
    let app = current_app().map_err(|err| err.to_string())?;
    let bridge = app.bridge().map_err(|err| err.to_string())?;
    bridge
        .call_async::<P, R, S>(
            action,
            request,
            BridgeCallOptions::default().with_timeout_ms(timeout_ms),
        )
        .await
        .map_err(|err| err.to_string())
}

pub(crate) async fn request_start_vpn(options_json: String) -> std::result::Result<(), String> {
    call_async_with_timeout::<PawsVpnBridgePlugin, VpnStartRequest, VpnStartResponse>(
        "start-vpn",
        VpnStartRequest { options_json },
        VPN_START_TIMEOUT_MS,
    )
    .await?;
    Ok(())
}

pub(crate) async fn request_stop_vpn() -> std::result::Result<(), String> {
    call_async::<PawsVpnBridgePlugin, VpnStopRequest, VpnStopResponse>(
        "stop-vpn",
        VpnStopRequest {},
    )
    .await?;
    Ok(())
}

pub(crate) async fn open_external_url(url: String) -> std::result::Result<(), String> {
    let is_web = url.starts_with("https://") || url.starts_with("http://");
    let is_clash_install = url.starts_with("clash://install-config?url=");
    if !is_web && !is_clash_install {
        return Err("unsupported external URL".to_owned());
    }
    let app = current_app().map_err(|err| err.to_string())?;
    app.open_url(url).await.map_err(|err| err.to_string())
}

pub(crate) async fn copy_text(text: String) -> std::result::Result<(), String> {
    call_async::<PawsClipboardBridgePlugin, ClipboardSetRequest, ClipboardSetResponse>(
        "set-text",
        ClipboardSetRequest { text },
    )
    .await?;
    Ok(())
}

/// Fire-and-forget color mode application. The native UI owns the preference
/// state; failures are logged by the ArkTS side and never surface to the UI.
pub(crate) fn set_color_mode(color_mode: i32) -> Result<()> {
    let app = current_app()?;
    let bridge = app.bridge()?;
    let task = async move {
        let _ = bridge
            .call_async::<PawsColorModeBridgePlugin, ColorModeRequest, ColorModeResponse>(
                "set-color-mode",
                ColorModeRequest { mode: color_mode },
                BridgeCallOptions::default(),
            )
            .await;
    };
    napi_ohos::bindgen_prelude::spawn(task);
    Ok(())
}

pub(crate) async fn export_profile(
    suggested_name: String,
    content: String,
) -> std::result::Result<(), String> {
    export_text("profile", suggested_name, content).await
}

pub(crate) async fn export_log(
    suggested_name: String,
    content: String,
) -> std::result::Result<(), String> {
    export_text("log", suggested_name, content).await
}

async fn export_text(
    export_kind: &str,
    suggested_name: String,
    content: String,
) -> std::result::Result<(), String> {
    call_async::<PawsExportBridgePlugin, ExportTextRequest, ExportTextResponse>(
        "export-text",
        ExportTextRequest {
            export_kind: export_kind.to_owned(),
            suggested_name,
            content,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn pick_profile_text() -> std::result::Result<(String, String), String> {
    let app = current_app().map_err(|err| err.to_string())?;
    let options = FileDialogOptions::new(dialog_type::OPEN_FILE)
        .allow_many(false)
        .filters(vec![
            FileDialogFilter::new().name("YAML").pattern(".yaml,.yml"),
            FileDialogFilter::new().name("Text").pattern(".txt"),
            FileDialogFilter::new().name("All").pattern("*"),
        ]);
    let response = app
        .show_file_dialog(options)
        .await
        .map_err(|err| format!("select profile file failed: {err}"))?;
    let uri = response
        .files
        .first()
        .map(String::as_str)
        .filter(|uri| !uri.trim().is_empty())
        .ok_or_else(|| "profile import cancelled".to_owned())?;
    persist_uris_or_err(&response.files).map_err(|err| err.to_string())?;
    let path =
        picker_uri_to_path(uri).ok_or_else(|| "failed to resolve profile file URI".to_owned())?;
    let text = read_text_from_path(&path)?;
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Local Profile")
        .to_owned();
    Ok((name, text))
}

pub(crate) async fn scan_subscription_code() -> std::result::Result<String, String> {
    let response =
        call_async::<PawsScanBridgePlugin, ScanRequest, ScanResponse>("scan-qr", ScanRequest {})
            .await?;
    Ok(response.content)
}

// --- Picker URI helpers ---

fn read_text_from_path(path: &PathBuf) -> std::result::Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|err| format!("failed to read profile file {}: {err}", path.display()))?;
    if bytes.is_empty() {
        return Err("profile file is empty".to_owned());
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn picker_uri_to_path(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return None;
    }
    uri_to_native_path(trimmed)
}

const FILE_SHARE_READ_MODE: u32 = 1 << 0;

fn uri_to_native_path(uri: &str) -> Option<PathBuf> {
    match ohos_fileuri_binding::get_path_from_uri(uri) {
        Ok(path) => Some(PathBuf::from(path)),
        Err(_) => uri
            .strip_prefix("file://")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(uri))),
    }
}

fn persist_uris_with_mode(uris: &[String], operation_mode: u32) -> Result<()> {
    let policies = uris
        .iter()
        .map(|uri| uri.trim())
        .filter(|uri| !uri.is_empty())
        .map(|uri| ohos_fileshare_binding::PolicyInfo {
            uri: uri.to_owned(),
            operation_mode,
        })
        .collect::<Vec<_>>();
    if policies.is_empty() {
        return Ok(());
    }
    let failed = ohos_fileshare_binding::persist_permission(&policies).map_err(|err| {
        Error::from_reason(format!("persist picker URI permission failed: {err}"))
    })?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::from_reason(format!(
            "persist picker URI permission partially failed: {failed:?}"
        )))
    }
}

fn persist_uris_or_err(uris: &[String]) -> Result<()> {
    persist_uris_with_mode(uris, FILE_SHARE_READ_MODE)
}
