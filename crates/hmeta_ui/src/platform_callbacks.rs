use napi_ohos::{
    bindgen_prelude::{CallbackContext, Function, JsObjectValue, Object, PromiseRaw, Unknown},
    threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
    Error, Result, Status,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, RwLock};
use tokio::sync::oneshot;

type FilePickerCall<'a> = Function<'a, (), Unknown<'a>>;
type FilePickerThreadsafeFunction = ThreadsafeFunction<(), Unknown<'static>, (), Status, false>;
type FilePickerSlot = LazyLock<RwLock<Option<Arc<FilePickerThreadsafeFunction>>>>;
type VpnStartCall<'a> = Function<'a, String, Unknown<'a>>;
type VpnStartThreadsafeFunction =
    ThreadsafeFunction<String, Unknown<'static>, String, Status, false>;
type VpnStartSlot = LazyLock<RwLock<Option<Arc<VpnStartThreadsafeFunction>>>>;
type VpnStopCall<'a> = Function<'a, (), Unknown<'a>>;
type VpnStopThreadsafeFunction = ThreadsafeFunction<(), Unknown<'static>, (), Status, false>;
type VpnStopSlot = LazyLock<RwLock<Option<Arc<VpnStopThreadsafeFunction>>>>;
type OpenExternalUrlCall<'a> = Function<'a, String, Unknown<'a>>;
type OpenExternalUrlThreadsafeFunction =
    ThreadsafeFunction<String, Unknown<'static>, String, Status, false>;
type OpenExternalUrlSlot = LazyLock<RwLock<Option<Arc<OpenExternalUrlThreadsafeFunction>>>>;
type ExportProfileCall<'a> = Function<'a, String, Unknown<'a>>;
type ExportProfileThreadsafeFunction =
    ThreadsafeFunction<String, Unknown<'static>, String, Status, false>;
type ExportProfileSlot = LazyLock<RwLock<Option<Arc<ExportProfileThreadsafeFunction>>>>;
type SetColorModeCall<'a> = Function<'a, i32, Unknown<'a>>;
type SetColorModeThreadsafeFunction = ThreadsafeFunction<i32, Unknown<'static>, i32, Status, false>;
type SetColorModeSlot = LazyLock<RwLock<Option<Arc<SetColorModeThreadsafeFunction>>>>;

static PROFILE_FILE_PICKER: FilePickerSlot = LazyLock::new(|| RwLock::new(None));
static REQUEST_START_VPN: VpnStartSlot = LazyLock::new(|| RwLock::new(None));
static REQUEST_STOP_VPN: VpnStopSlot = LazyLock::new(|| RwLock::new(None));
static OPEN_EXTERNAL_URL: OpenExternalUrlSlot = LazyLock::new(|| RwLock::new(None));
static EXPORT_PROFILE: ExportProfileSlot = LazyLock::new(|| RwLock::new(None));
static SET_COLOR_MODE: SetColorModeSlot = LazyLock::new(|| RwLock::new(None));

pub(crate) fn register_platform_callbacks(callbacks: Object<'static>) -> Result<()> {
    if callbacks.has_named_property("pickProfileFile")? {
        let pick_profile_file: FilePickerCall<'static> =
            callbacks.get_named_property("pickProfileFile")?;
        set_profile_file_picker(pick_profile_file)?;
    }
    if callbacks.has_named_property("requestStartVpn")? {
        let request_start_vpn: VpnStartCall<'static> =
            callbacks.get_named_property("requestStartVpn")?;
        set_request_start_vpn(request_start_vpn)?;
    }
    if callbacks.has_named_property("requestStopVpn")? {
        let request_stop_vpn: VpnStopCall<'static> =
            callbacks.get_named_property("requestStopVpn")?;
        set_request_stop_vpn(request_stop_vpn)?;
    }
    if callbacks.has_named_property("openExternalUrl")? {
        let open_external_url: OpenExternalUrlCall<'static> =
            callbacks.get_named_property("openExternalUrl")?;
        set_open_external_url(open_external_url)?;
    }
    if callbacks.has_named_property("exportProfile")? {
        let export_profile: ExportProfileCall<'static> =
            callbacks.get_named_property("exportProfile")?;
        set_export_profile(export_profile)?;
    }
    if callbacks.has_named_property("setColorMode")? {
        let set_color_mode: SetColorModeCall<'static> =
            callbacks.get_named_property("setColorMode")?;
        set_color_mode_callback(set_color_mode)?;
    }
    Ok(())
}

fn set_profile_file_picker(pick_profile_file: FilePickerCall<'static>) -> Result<()> {
    let tsfn = pick_profile_file
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    PROFILE_FILE_PICKER
        .write()
        .map_err(|_| Error::from_reason("failed to store profile picker callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

fn set_request_start_vpn(request_start_vpn: VpnStartCall<'static>) -> Result<()> {
    let tsfn = request_start_vpn
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    REQUEST_START_VPN
        .write()
        .map_err(|_| Error::from_reason("failed to store VPN start callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

fn set_request_stop_vpn(request_stop_vpn: VpnStopCall<'static>) -> Result<()> {
    let tsfn = request_stop_vpn
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    REQUEST_STOP_VPN
        .write()
        .map_err(|_| Error::from_reason("failed to store VPN stop callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

fn set_open_external_url(open_external_url: OpenExternalUrlCall<'static>) -> Result<()> {
    let tsfn = open_external_url
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    OPEN_EXTERNAL_URL
        .write()
        .map_err(|_| Error::from_reason("failed to store external URL callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

fn set_export_profile(export_profile: ExportProfileCall<'static>) -> Result<()> {
    let tsfn = export_profile
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    EXPORT_PROFILE
        .write()
        .map_err(|_| Error::from_reason("failed to store profile export callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

fn set_color_mode_callback(set_color_mode: SetColorModeCall<'static>) -> Result<()> {
    let tsfn = set_color_mode
        .build_threadsafe_function()
        .callee_handled::<false>()
        .build()?;
    SET_COLOR_MODE
        .write()
        .map_err(|_| Error::from_reason("failed to store color mode callback"))?
        .replace(Arc::new(tsfn));
    Ok(())
}

pub(crate) fn set_color_mode(color_mode: i32) -> Result<()> {
    let tsfn = SET_COLOR_MODE
        .read()
        .map_err(|_| Error::from_reason("failed to read color mode callback"))?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| Error::from_reason("color mode callback is not registered"))?;
    let status = tsfn.call(color_mode, ThreadsafeFunctionCallMode::NonBlocking);
    if status == Status::Ok {
        Ok(())
    } else {
        Err(Error::from_reason(format!(
            "call color mode callback failed with status: {status:?}"
        )))
    }
}

pub(crate) async fn request_start_vpn(options_json: String) -> std::result::Result<(), String> {
    let tsfn = REQUEST_START_VPN
        .read()
        .map_err(|_| "failed to read VPN start callback".to_owned())?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "VPN start callback is not registered".to_owned())?;
    invoke_string_void_callback(tsfn, options_json, "VPN start").await
}

pub(crate) async fn request_stop_vpn() -> std::result::Result<(), String> {
    let tsfn = REQUEST_STOP_VPN
        .read()
        .map_err(|_| "failed to read VPN stop callback".to_owned())?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "VPN stop callback is not registered".to_owned())?;
    invoke_void_callback(tsfn, "VPN stop").await
}

pub(crate) async fn pick_profile_text() -> std::result::Result<(String, String), String> {
    let uris = pick_files()
        .await
        .map_err(|err| format!("select profile file failed: {err}"))?;
    let uri = uris
        .first()
        .map(String::as_str)
        .filter(|uri| !uri.trim().is_empty())
        .ok_or_else(|| "profile import cancelled".to_owned())?;
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

pub(crate) async fn open_external_url(url: String) -> std::result::Result<(), String> {
    let tsfn = OPEN_EXTERNAL_URL
        .read()
        .map_err(|_| "failed to read external URL callback".to_owned())?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "external URL callback is not registered".to_owned())?;
    invoke_string_void_callback(tsfn, url, "external URL").await
}

pub(crate) async fn export_profile(
    suggested_name: String,
    content: String,
) -> std::result::Result<(), String> {
    let tsfn = EXPORT_PROFILE
        .read()
        .map_err(|_| "failed to read profile export callback".to_owned())?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "profile export callback is not registered".to_owned())?;
    let request = serde_json::json!({
        "suggestedName": suggested_name,
        "content": content,
    })
    .to_string();
    invoke_string_void_callback(tsfn, request, "profile export").await
}

async fn pick_files() -> Result<Vec<String>> {
    let tsfn = PROFILE_FILE_PICKER
        .read()
        .map_err(|_| Error::from_reason("failed to read profile picker callback"))?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| Error::from_reason("profile picker callback is not registered"))?;
    let uris = invoke_picker(tsfn).await?;
    #[cfg(target_env = "ohos")]
    persist_uris_or_err(&uris, FILE_SHARE_READ_MODE)?;
    Ok(uris)
}

async fn invoke_string_void_callback(
    tsfn: Arc<OpenExternalUrlThreadsafeFunction>,
    value: String,
    label: &'static str,
) -> std::result::Result<(), String> {
    let (tx, rx) = oneshot::channel::<Result<()>>();
    let status = tsfn.call_with_return_value(value, ThreadsafeFunctionCallMode::NonBlocking, {
        move |result, _| {
            match result {
                Ok(value) => {
                    let tx_cell = Rc::new(Cell::new(Some(tx)));
                    let tx_in_catch = tx_cell.clone();
                    let promise = unsafe { value.cast::<PromiseRaw<'static, ()>>() }?;
                    promise
                        .then(move |_ctx| {
                            if let Some(sender) = tx_cell.replace(None) {
                                let _ = sender.send(Ok(()));
                            }
                            Ok(())
                        })?
                        .catch(move |ctx: CallbackContext<Unknown>| {
                            if let Some(sender) = tx_in_catch.replace(None) {
                                let _ = sender.send(Err(ctx.value.into()));
                            }
                            Ok(())
                        })?;
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            }
            Ok(())
        }
    });
    if status != Status::Ok {
        return Err(format!(
            "call {label} callback failed with status: {status:?}"
        ));
    }
    rx.await
        .map_err(|_| format!("{label} callback receiver dropped"))?
        .map_err(|err| err.to_string())
}

async fn invoke_void_callback(
    tsfn: Arc<VpnStopThreadsafeFunction>,
    label: &'static str,
) -> std::result::Result<(), String> {
    let (tx, rx) = oneshot::channel::<Result<()>>();
    let status = tsfn.call_with_return_value((), ThreadsafeFunctionCallMode::NonBlocking, {
        move |result, _| {
            match result {
                Ok(value) => {
                    let tx_cell = Rc::new(Cell::new(Some(tx)));
                    let tx_in_catch = tx_cell.clone();
                    let promise = unsafe { value.cast::<PromiseRaw<'static, ()>>() }?;
                    promise
                        .then(move |_ctx| {
                            if let Some(sender) = tx_cell.replace(None) {
                                let _ = sender.send(Ok(()));
                            }
                            Ok(())
                        })?
                        .catch(move |ctx: CallbackContext<Unknown>| {
                            if let Some(sender) = tx_in_catch.replace(None) {
                                let _ = sender.send(Err(ctx.value.into()));
                            }
                            Ok(())
                        })?;
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            }
            Ok(())
        }
    });
    if status != Status::Ok {
        return Err(format!(
            "call {label} callback failed with status: {status:?}"
        ));
    }
    rx.await
        .map_err(|_| format!("{label} callback receiver dropped"))?
        .map_err(|err| err.to_string())
}

async fn invoke_picker(tsfn: Arc<FilePickerThreadsafeFunction>) -> Result<Vec<String>> {
    let (tx, rx) = oneshot::channel::<Result<Vec<String>>>();
    let status = tsfn.call_with_return_value((), ThreadsafeFunctionCallMode::NonBlocking, {
        move |result, _| {
            match result {
                Ok(value) => {
                    let tx_cell = Rc::new(Cell::new(Some(tx)));
                    let tx_in_catch = tx_cell.clone();
                    let promise = unsafe { value.cast::<PromiseRaw<'static, Vec<String>>>() }?;
                    promise
                        .then(move |ctx| {
                            if let Some(sender) = tx_cell.replace(None) {
                                let _ = sender.send(Ok(ctx.value));
                            }
                            Ok(())
                        })?
                        .catch(move |ctx: CallbackContext<Unknown>| {
                            if let Some(sender) = tx_in_catch.replace(None) {
                                let _ = sender.send(Err(ctx.value.into()));
                            }
                            Ok(())
                        })?;
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            }
            Ok(())
        }
    });
    if status != Status::Ok {
        return Err(Error::from_reason(format!(
            "call profile picker callback failed with status: {status:?}"
        )));
    }
    rx.await
        .map_err(|_| Error::from_reason("profile picker callback receiver dropped"))?
}

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

#[cfg(target_env = "ohos")]
const FILE_SHARE_READ_MODE: u32 = 1 << 0;

#[cfg(target_env = "ohos")]
fn uri_to_native_path(uri: &str) -> Option<PathBuf> {
    match ohos_fileuri_binding::get_path_from_uri(uri) {
        Ok(path) => Some(PathBuf::from(path)),
        Err(_) => uri
            .strip_prefix("file://")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(uri))),
    }
}

#[cfg(not(target_env = "ohos"))]
fn uri_to_native_path(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(uri)))
}

#[cfg(target_env = "ohos")]
fn persist_uris_or_err(uris: &[String], operation_mode: u32) -> Result<()> {
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
