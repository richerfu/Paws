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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
pub(crate) use self::export::{
    ExportImageRequest, ExportImageResponse, ExportTextRequest, ExportTextResponse,
    PawsExportBridgePlugin,
};
pub(crate) use self::safe_area::{initial_safe_area, InitialSafeArea, PawsSafeAreaBridgePlugin};
pub(crate) use self::scan::{PawsScanBridgePlugin, ScanRequest, ScanResponse};
pub(crate) use self::vpn::{
    PawsVpnBridgePlugin, VpnOperationLookupRequest, VpnOperationLookupResponse,
    VpnOperationWaitRequest, VpnOperationWaitResponse, VpnOwnedStopRequest, VpnOwnedStopResponse,
    VpnRestartRequest, VpnRestartResponse, VpnStartRequest, VpnStartResponse, VpnStopRequest,
    VpnStopResponse,
};
pub(crate) use crate::vpn_operation::{VpnOperationReceipt, VpnUnconfirmedOperation};

/// Rust-side handle of the current Ability session, installed by `init`.
static INNER_APP: LazyLock<RwLock<Option<OpenHarmonyApp>>> = LazyLock::new(|| RwLock::new(None));

// openharmony-ability clamps every individual bridge call to 60 seconds. VPN
// work can legitimately outlive that transport budget (first authorization,
// cleanup, then restart), so submit once and observe the retained operation in
// bounded long-poll slices. A transport failure after submission is therefore
// uncertain, never evidence that the platform operation failed or stopped.
const VPN_SUBMIT_TIMEOUT_MS: u32 = 10_000;
const VPN_OPERATION_WAIT_SLICE_MS: u32 = 45_000;
const VPN_OPERATION_TRANSPORT_TIMEOUT_MS: u32 = 55_000;
const VPN_START_BUDGET: Duration = Duration::from_secs(150);
const VPN_STOP_BUDGET: Duration = Duration::from_secs(150);
const VPN_RESTART_BUDGET: Duration = Duration::from_secs(300);
static NEXT_VPN_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static VPN_REQUEST_NONCE: LazyLock<String> = LazyLock::new(|| {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{started_at}", std::process::id())
});

fn next_vpn_request_id() -> String {
    let next = NEXT_VPN_REQUEST_ID
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    format!("rust-{}-{next}", VPN_REQUEST_NONCE.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VpnOperationOutcome<T> {
    Completed(T),
    /// The one submitted operation is still owned by the ArkTS session, but
    /// its terminal result could not be observed within our business budget.
    Unconfirmed(VpnUnconfirmedOperation),
}

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

enum VpnReceiptStatus {
    Pending,
    Completed(bool),
    Failed(String),
    Unobservable(String),
}

fn vpn_operation_failure(label: &str, error: &str) -> String {
    let error = error.trim();
    if error.is_empty() {
        format!("{label} failed without an error message")
    } else {
        error.to_owned()
    }
}

fn unconfirmed(receipt: VpnOperationReceipt, reason: impl Into<String>) -> VpnUnconfirmedOperation {
    VpnUnconfirmedOperation {
        message: format!(
            "{} result is not yet confirmed ({}). The submitted platform operation may still be continuing; confirm it again or stop and resynchronize before starting another connection",
            receipt.label,
            reason.into()
        ),
        receipt,
    }
}

fn lookup_status(
    receipt: &mut VpnOperationReceipt,
    response: VpnOperationLookupResponse,
) -> std::result::Result<VpnReceiptStatus, String> {
    match response.status.as_str() {
        "found" => {
            if response.operation_id.trim().is_empty() {
                return Err(format!(
                    "{} receipt lookup returned an empty operation id",
                    receipt.label
                ));
            }
            receipt.operation_id = Some(response.operation_id);
            match response.operation_status.as_str() {
                "pending" => Ok(VpnReceiptStatus::Pending),
                "succeeded" => Ok(VpnReceiptStatus::Completed(response.result)),
                "failed" => Ok(VpnReceiptStatus::Failed(vpn_operation_failure(
                    receipt.label,
                    &response.error,
                ))),
                "unknown" => Ok(VpnReceiptStatus::Unobservable(format!(
                    "request {} has no observable terminal status in the current Ability session",
                    receipt.request_id
                ))),
                status => Err(format!(
                    "{} receipt lookup returned invalid operation status '{status}'",
                    receipt.label
                )),
            }
        }
        "unknown-session" => Ok(VpnReceiptStatus::Unobservable(format!(
            "request {} is not owned by the current Ability session",
            receipt.request_id
        ))),
        status => Err(format!(
            "{} receipt lookup returned invalid status '{status}'",
            receipt.label
        )),
    }
}

async fn lookup_vpn_operation(
    receipt: &mut VpnOperationReceipt,
) -> std::result::Result<VpnReceiptStatus, String> {
    let response = call_async_with_timeout::<
        PawsVpnBridgePlugin,
        VpnOperationLookupRequest,
        VpnOperationLookupResponse,
    >(
        "lookup-vpn-operation",
        VpnOperationLookupRequest {
            request_id: receipt.request_id.clone(),
        },
        VPN_SUBMIT_TIMEOUT_MS,
    )
    .await?;
    lookup_status(receipt, response)
}

async fn observe_vpn_operation(
    receipt: &mut VpnOperationReceipt,
    wait_ms: u32,
) -> std::result::Result<VpnReceiptStatus, String> {
    let Some(operation_id) = receipt.operation_id.clone() else {
        return lookup_vpn_operation(receipt).await;
    };
    let response = match call_async_with_timeout::<
        PawsVpnBridgePlugin,
        VpnOperationWaitRequest,
        VpnOperationWaitResponse,
    >(
        "await-vpn-operation",
        VpnOperationWaitRequest {
            operation_id,
            wait_ms,
        },
        VPN_OPERATION_TRANSPORT_TIMEOUT_MS,
    )
    .await
    {
        Ok(response) => response,
        Err(_) => return lookup_vpn_operation(receipt).await,
    };
    match response.status.as_str() {
        "pending" => Ok(VpnReceiptStatus::Pending),
        "succeeded" => Ok(VpnReceiptStatus::Completed(response.result)),
        "failed" => Ok(VpnReceiptStatus::Failed(vpn_operation_failure(
            receipt.label,
            &response.error,
        ))),
        "unavailable" => {
            receipt.operation_id = None;
            lookup_vpn_operation(receipt).await
        }
        status => Err(format!(
            "{} returned invalid operation status '{status}'",
            receipt.label
        )),
    }
}

async fn await_vpn_operation(
    mut receipt: VpnOperationReceipt,
    budget: Duration,
) -> std::result::Result<VpnOperationOutcome<bool>, String> {
    let deadline = Instant::now() + budget;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(VpnOperationOutcome::Unconfirmed(unconfirmed(
                receipt,
                "the observation budget elapsed",
            )));
        }
        let wait_ms = remaining
            .as_millis()
            .min(u128::from(VPN_OPERATION_WAIT_SLICE_MS))
            .max(1) as u32;
        match observe_vpn_operation(&mut receipt, wait_ms).await {
            Ok(VpnReceiptStatus::Pending) => {}
            Ok(VpnReceiptStatus::Completed(result)) => {
                return Ok(VpnOperationOutcome::Completed(result));
            }
            Ok(VpnReceiptStatus::Failed(error)) => return Err(error),
            Ok(VpnReceiptStatus::Unobservable(reason)) => {
                return Ok(VpnOperationOutcome::Unconfirmed(unconfirmed(
                    receipt, reason,
                )));
            }
            Err(error) => {
                return Ok(VpnOperationOutcome::Unconfirmed(unconfirmed(
                    receipt, error,
                )));
            }
        }
    }
}

/// Perform one authoritative receipt observation after an operation entered
/// the recoverable `Unconfirmed` UI phase. An unobservable receipt deliberately
/// keeps the UI operation active; it is not equivalent to a platform failure.
pub(crate) async fn confirm_vpn_operation(
    mut receipt: VpnOperationReceipt,
) -> std::result::Result<VpnOperationOutcome<bool>, String> {
    match observe_vpn_operation(&mut receipt, VPN_OPERATION_WAIT_SLICE_MS).await {
        Ok(VpnReceiptStatus::Completed(result)) => Ok(VpnOperationOutcome::Completed(result)),
        Ok(VpnReceiptStatus::Failed(error)) => Err(error),
        Ok(VpnReceiptStatus::Pending) => Ok(VpnOperationOutcome::Unconfirmed(unconfirmed(
            receipt,
            "the operation is still pending",
        ))),
        Ok(VpnReceiptStatus::Unobservable(reason)) => Ok(VpnOperationOutcome::Unconfirmed(
            unconfirmed(receipt, reason),
        )),
        Err(error) => Ok(VpnOperationOutcome::Unconfirmed(unconfirmed(
            receipt, error,
        ))),
    }
}

fn submitted_receipt(
    request_id: String,
    operation_id: Option<String>,
    label: &'static str,
) -> VpnOperationReceipt {
    VpnOperationReceipt {
        request_id,
        operation_id,
        label,
    }
}

pub(crate) async fn request_start_vpn(
    options_json: String,
) -> std::result::Result<VpnOperationOutcome<()>, String> {
    let request_id = next_vpn_request_id();
    let submitted =
        call_async_with_timeout::<PawsVpnBridgePlugin, VpnStartRequest, VpnStartResponse>(
            "start-vpn",
            VpnStartRequest {
                request_id: request_id.clone(),
                options_json,
            },
            VPN_SUBMIT_TIMEOUT_MS,
        )
        .await;
    let receipt = submitted_receipt(
        request_id,
        submitted.ok().map(|submitted| submitted.operation_id),
        "VPN start",
    );
    match await_vpn_operation(receipt, VPN_START_BUDGET).await? {
        VpnOperationOutcome::Completed(true) => Ok(VpnOperationOutcome::Completed(())),
        VpnOperationOutcome::Completed(false) => {
            Err("VPN start completed without applying the request".to_owned())
        }
        VpnOperationOutcome::Unconfirmed(operation) => {
            Ok(VpnOperationOutcome::Unconfirmed(operation))
        }
    }
}

pub(crate) async fn request_stop_vpn() -> std::result::Result<VpnOperationOutcome<()>, String> {
    let request_id = next_vpn_request_id();
    let submitted =
        call_async_with_timeout::<PawsVpnBridgePlugin, VpnStopRequest, VpnStopResponse>(
            "stop-vpn",
            VpnStopRequest {
                request_id: request_id.clone(),
            },
            VPN_SUBMIT_TIMEOUT_MS,
        )
        .await;
    let receipt = submitted_receipt(
        request_id,
        submitted.ok().map(|submitted| submitted.operation_id),
        "VPN stop",
    );
    match await_vpn_operation(receipt, VPN_STOP_BUDGET).await? {
        VpnOperationOutcome::Completed(true) => Ok(VpnOperationOutcome::Completed(())),
        VpnOperationOutcome::Completed(false) => {
            Err("VPN stop completed without applying the request".to_owned())
        }
        VpnOperationOutcome::Unconfirmed(operation) => {
            Ok(VpnOperationOutcome::Unconfirmed(operation))
        }
    }
}

/// Stop only the connected session which still owns the checked config
/// revision. A later user action or configuration mutation wins.
pub(crate) async fn request_stop_vpn_if_current(
    expected_session_id: String,
    expected_config_revision: u64,
) -> std::result::Result<VpnOperationOutcome<bool>, String> {
    let request_id = next_vpn_request_id();
    let submitted =
        call_async_with_timeout::<PawsVpnBridgePlugin, VpnOwnedStopRequest, VpnOwnedStopResponse>(
            "stop-vpn-if-current",
            VpnOwnedStopRequest {
                request_id: request_id.clone(),
                expected_session_id,
                expected_config_revision: expected_config_revision.to_string(),
            },
            VPN_SUBMIT_TIMEOUT_MS,
        )
        .await;
    await_vpn_operation(
        submitted_receipt(
            request_id,
            submitted.ok().map(|submitted| submitted.operation_id),
            "owned VPN stop",
        ),
        VPN_STOP_BUDGET,
    )
    .await
}

/// Restart only the session which owned the edit. A later user start/stop wins.
pub(crate) async fn request_restart_vpn(
    expected_session_id: String,
    expected_config_revision: u64,
    options_json: String,
) -> std::result::Result<VpnOperationOutcome<bool>, String> {
    let request_id = next_vpn_request_id();
    let submitted =
        call_async_with_timeout::<PawsVpnBridgePlugin, VpnRestartRequest, VpnRestartResponse>(
            "restart-vpn",
            VpnRestartRequest {
                request_id: request_id.clone(),
                expected_session_id,
                expected_config_revision: expected_config_revision.to_string(),
                options_json,
            },
            VPN_SUBMIT_TIMEOUT_MS,
        )
        .await;
    await_vpn_operation(
        submitted_receipt(
            request_id,
            submitted.ok().map(|submitted| submitted.operation_id),
            "owned VPN restart",
        ),
        VPN_RESTART_BUDGET,
    )
    .await
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

/// Observe the platform acknowledgement before marking a color preference as
/// applied. Queueing a bridge call alone does not mean setColorMode succeeded.
pub(crate) async fn set_color_mode(color_mode: i32) -> std::result::Result<(), String> {
    call_async::<PawsColorModeBridgePlugin, ColorModeRequest, ColorModeResponse>(
        "set-color-mode",
        ColorModeRequest { mode: color_mode },
    )
    .await?;
    Ok(())
}

pub(crate) async fn export_profile(
    suggested_name: String,
    content: String,
) -> std::result::Result<(), String> {
    export_text("profile", suggested_name, content).await
}

pub(crate) async fn export_profile_qr(
    suggested_name: String,
    png_bytes: Vec<u8>,
) -> std::result::Result<(), String> {
    use base64::Engine as _;
    let png_base64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    call_async::<PawsExportBridgePlugin, ExportImageRequest, ExportImageResponse>(
        "export-image",
        ExportImageRequest {
            suggested_name,
            png_base64,
        },
    )
    .await?;
    Ok(())
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

#[cfg(test)]
mod vpn_receipt_tests {
    use super::*;

    fn receipt() -> VpnOperationReceipt {
        VpnOperationReceipt {
            request_id: "request-1".to_owned(),
            operation_id: None,
            label: "VPN test",
        }
    }

    #[test]
    fn unknown_session_is_not_misreported_as_an_unsubmitted_operation() {
        let mut receipt = receipt();
        let status = lookup_status(
            &mut receipt,
            VpnOperationLookupResponse {
                status: "unknown-session".to_owned(),
                operation_id: String::new(),
                operation_status: "unknown".to_owned(),
                result: false,
                error: String::new(),
            },
        )
        .expect("unknown ownership is a valid non-terminal observation");

        assert!(matches!(status, VpnReceiptStatus::Unobservable(_)));
        assert_eq!(receipt.request_id, "request-1");
        assert!(!matches!(status, VpnReceiptStatus::Completed(_)));
    }

    #[test]
    fn lookup_preserves_the_exact_terminal_failure() {
        let mut receipt = receipt();
        let status = lookup_status(
            &mut receipt,
            VpnOperationLookupResponse {
                status: "found".to_owned(),
                operation_id: "ability:7".to_owned(),
                operation_status: "failed".to_owned(),
                result: false,
                error: "cleanup deadline".to_owned(),
            },
        )
        .expect("terminal failure receipt");

        assert!(
            matches!(status, VpnReceiptStatus::Failed(ref error) if error == "cleanup deadline")
        );
        assert_eq!(receipt.operation_id.as_deref(), Some("ability:7"));
    }

    #[test]
    fn found_but_unknown_status_remains_unconfirmed() {
        let mut receipt = receipt();
        let status = lookup_status(
            &mut receipt,
            VpnOperationLookupResponse {
                status: "found".to_owned(),
                operation_id: "ability:8".to_owned(),
                operation_status: "unknown".to_owned(),
                result: false,
                error: String::new(),
            },
        )
        .expect("unknown terminal status remains observable as uncertainty");

        assert!(matches!(status, VpnReceiptStatus::Unobservable(_)));
        assert_eq!(receipt.operation_id.as_deref(), Some("ability:8"));
    }
}

pub(crate) async fn pick_profile_text() -> std::result::Result<Option<(String, String)>, String> {
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
    let Some(uri) = response
        .files
        .first()
        .map(String::as_str)
        .filter(|uri| !uri.trim().is_empty())
    else {
        return Ok(None);
    };
    persist_uris_or_err(&response.files).map_err(|err| err.to_string())?;
    let path =
        picker_uri_to_path(uri).ok_or_else(|| "failed to resolve profile file URI".to_owned())?;
    let text = read_text_from_path(&path)?;
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Local Profile")
        .to_owned();
    Ok(Some((name, text)))
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
