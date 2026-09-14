use napi_derive_ohos::napi;
use napi_ohos::{bindgen_prelude::Object, Env, Error, Result, Status};
use ohos_resource_manager_binding::ResourceManager;
use paws_model::RuntimeMode;
use std::path::{Path, PathBuf};
use std::{fs, io};

mod activity_filter;
mod bridge;
mod i18n;
mod locale;
mod log_filter;
mod manual_rule;
mod mode_feedback;
mod notification;
mod profile_filter;
mod profile_refresh_feedback;
mod provider_refresh_feedback;
mod proxy_filter;
mod proxy_grid;
mod reactive_signal;
mod resource_filter;
mod route_status;
mod rule_feedback;
mod settings_draft;
mod settings_feedback;
mod subscription_converter;
mod subscription_scan;
mod system_preferences;
mod time_format;
mod traffic_history;
mod ui;
mod ui_preferences;
mod virtual_identity;
mod vpn_feedback;
mod vpn_operation;
mod yaml_summary;

use arkit::entry;
use arkit::prelude::Element;

/// Application entry: arkit's `#[entry]` generates the NAPI init/render/
/// destroy lifecycle and bridge event ports, registers the `paws.*` bridge
/// plugins declaratively, and passes the shared `OpenHarmonyApp` handle into
/// the entry function so the platform call surface can resolve it.
#[entry(plugins = [
    openharmony_ability_plugin_files::FilesBridgePlugin,
    openharmony_ability_plugin_url::UrlBridgePlugin,
    bridge::PawsScanBridgePlugin,
    bridge::PawsClipboardBridgePlugin,
    bridge::PawsColorModeBridgePlugin,
    bridge::PawsVpnBridgePlugin,
    bridge::PawsExportBridgePlugin,
    bridge::PawsSafeAreaBridgePlugin,
])]
fn app(handle: arkit::openharmony_ability::OpenHarmonyApp) -> Element {
    let initial_safe_area = bridge::initial_safe_area(&handle);
    bridge::set_app(handle);
    ui::App(initial_safe_area)
}

#[napi]
pub fn configure_app_home(home_dir: String) -> Result<()> {
    paws_core::configure_app_home(std::path::Path::new(&home_dir)).map_err(to_napi_error)
}

#[napi]
pub fn initialize_platform_shared_memory() -> Result<String> {
    let fds = paws_core::shared_core()
        .initialize_platform_shared_memory()
        .map_err(to_napi_error)?;
    Ok(format!("{},{}", fds.ashmem_fd, fds.notification_fd))
}

#[napi]
pub fn attach_platform_shared_memory(ashmem_fd: i32, notification_fd: i32) -> Result<()> {
    paws_core::shared_core()
        .attach_platform_shared_memory(ashmem_fd, notification_fd)
        .map_err(to_napi_error)
}

#[napi]
pub fn validate_platform_vpn_start_request(
    ashmem_fd: i32,
    notification_fd: i32,
    attempt_id: String,
) -> Result<()> {
    paws_core::shared_core()
        .validate_platform_vpn_start_request(ashmem_fd, notification_fd, &attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn wait_for_platform_change_event() -> Result<bool> {
    paws_core::shared_core()
        .wait_for_platform_change_event()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn cancel_platform_change_wait() {
    paws_core::shared_core().cancel_platform_change_wait();
}

#[napi]
pub fn sync_platform_changes() -> Result<()> {
    paws_core::shared_core()
        .sync_platform_changes()
        .map_err(to_napi_error)
}

#[napi]
pub fn advance_platform_vpn_intent() -> Result<String> {
    paws_core::shared_core()
        .advance_platform_vpn_intent()
        .map(|epoch| epoch.to_string())
        .map_err(to_napi_error)
}

#[napi]
pub fn is_platform_vpn_intent_current(intent_epoch: String) -> Result<bool> {
    paws_core::shared_core()
        .is_platform_vpn_intent_current(parse_positive_u64(&intent_epoch, "VPN intent epoch")?)
        .map_err(to_napi_error)
}

#[napi]
pub fn is_platform_vpn_stop_current(intent_epoch: String, attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .is_platform_vpn_stop_current(
            parse_positive_u64(&intent_epoch, "VPN intent epoch")?,
            &attempt_id,
        )
        .map_err(to_napi_error)
}

#[napi]
pub fn begin_platform_vpn_os_stop(intent_epoch: String, attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .begin_platform_vpn_os_stop(
            parse_positive_u64(&intent_epoch, "VPN intent epoch")?,
            &attempt_id,
        )
        .map_err(to_napi_error)
}

#[napi]
pub fn complete_platform_vpn_os_stop(intent_epoch: String, attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .complete_platform_vpn_os_stop(
            parse_positive_u64(&intent_epoch, "VPN intent epoch")?,
            &attempt_id,
        )
        .map_err(to_napi_error)
}

#[napi]
pub fn fail_platform_vpn_os_stop(intent_epoch: String, attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .fail_platform_vpn_os_stop(
            parse_positive_u64(&intent_epoch, "VPN intent epoch")?,
            &attempt_id,
        )
        .map_err(to_napi_error)
}

#[napi]
pub fn begin_platform_vpn_start_for_intent(intent_epoch: String) -> Result<String> {
    paws_core::shared_core()
        .begin_platform_vpn_start_for_intent(parse_positive_u64(&intent_epoch, "VPN intent epoch")?)
        .map_err(to_napi_error)
}

#[napi]
pub fn bind_platform_vpn_start(attempt_id: String) -> Result<String> {
    paws_core::shared_core()
        .bind_platform_vpn_start(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn await_platform_vpn_start(attempt_id: String) -> Result<String> {
    let outcome = paws_core::shared_core()
        .await_platform_vpn_start(&attempt_id)
        .await
        .map_err(to_napi_error)?;
    Ok(match outcome {
        paws_core::PlatformStartOutcome::Connected => "connected",
        paws_core::PlatformStartOutcome::Failed => "failed",
        paws_core::PlatformStartOutcome::Cancelled => "cancelled",
        paws_core::PlatformStartOutcome::Idle => "idle",
        paws_core::PlatformStartOutcome::Pending => "pending",
    }
    .to_owned())
}

#[napi]
pub fn acknowledge_terminal_platform_vpn_start_delivery(
    ashmem_fd: i32,
    notification_fd: i32,
    attempt_id: String,
) -> Result<bool> {
    paws_core::shared_core()
        .acknowledge_terminal_platform_vpn_start_delivery(ashmem_fd, notification_fd, &attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn await_platform_vpn_stop(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .await_platform_vpn_stop(&attempt_id)
        .await
        .map_err(to_napi_error)
}

/// Called only after ArkTS has awaited HarmonyOS
/// `stopVpnExtensionAbility`. Core still requires release of the exact owner
/// lease (or proof the terminal Want never attached) before releasing an
/// orphaned cleanup barrier.
#[napi]
pub async fn recover_platform_vpn_cleanup_after_confirmed_stop(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .recover_platform_vpn_cleanup_after_confirmed_stop(&attempt_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn fail_platform_vpn_start(attempt_id: String, error: String) -> Result<bool> {
    paws_core::shared_core()
        .fail_platform_vpn_start(&attempt_id, error)
        .map_err(to_napi_error)
}

#[napi]
pub fn fail_unattached_platform_vpn_start(attempt_id: String, error: String) -> Result<bool> {
    paws_core::shared_core()
        .fail_unattached_platform_vpn_start(&attempt_id, error)
        .map_err(to_napi_error)
}

#[napi]
pub fn cancel_platform_vpn_start(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .cancel_platform_vpn_start(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub fn request_platform_vpn_stop(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .request_platform_vpn_stop(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub fn configure_ui_locale(locale: String) -> Result<()> {
    if locale.trim().is_empty() {
        return Err(Error::from_reason("system locale must not be empty"));
    }
    system_preferences::set_locale(locale);
    Ok(())
}

#[napi]
pub fn configure_system_color_mode(color_mode: i32) -> Result<()> {
    if !matches!(color_mode, -1..=1) {
        return Err(Error::from_reason("invalid system color mode"));
    }
    system_preferences::set_color_mode(color_mode);
    Ok(())
}

const GEODATA_RAW_DIR: &str = "geodata";
const GEODATA_SEED_FILES: &[(&str, &str)] = &[
    ("geodata/Country.mmdb", "Country.mmdb"),
    ("geodata/GeoLite2-ASN.mmdb", "GeoLite2-ASN.mmdb"),
    ("geodata/geosite.dat", "geosite.dat"),
];

#[napi]
pub fn seed_geodata_from_rawfiles<'a>(
    env: Env,
    #[napi(ts_arg_type = "resourceManager.ResourceManager")] resource_manager: Object<'a>,
) -> Result<u32> {
    let home_dir = std::env::var("PAWS_HOME").map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "PAWS_HOME is not configured before geodata seed".to_owned(),
        )
    })?;
    let geodata_dir = PathBuf::from(home_dir).join("geodata");
    fs::create_dir_all(&geodata_dir).map_err(io_to_napi)?;

    let resource_manager = ResourceManager::new(env, resource_manager);
    let raw_dir = resource_manager
        .open_dir(GEODATA_RAW_DIR, false)
        .map_err(|err| Error::new(Status::GenericFailure, err.to_string()))?;

    let mut seeded = 0;
    for (raw_path, dest_name) in GEODATA_SEED_FILES {
        if !raw_dir.files.contains_key(*raw_path) {
            continue;
        }
        let dest = geodata_dir.join(dest_name);
        if dest.metadata().map(|meta| meta.len() > 0).unwrap_or(false) {
            continue;
        }

        let raw_file = raw_dir.open_file64(*raw_path);
        let size = raw_file.file_size();
        if size <= 0 {
            continue;
        }
        write_seed_file(&dest, &raw_file.read(size))?;
        seeded += 1;
    }

    Ok(seeded)
}

fn write_seed_file(dest: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = dest.with_extension("seed.tmp");
    fs::write(&tmp, bytes).map_err(io_to_napi)?;
    fs::rename(&tmp, dest).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        io_to_napi(err)
    })
}

fn io_to_napi(err: io::Error) -> Error {
    Error::new(Status::GenericFailure, err.to_string())
}

#[napi]
pub fn claim_current_platform_vpn_stop(intent_epoch: String) -> Result<String> {
    paws_core::shared_core()
        .claim_current_platform_vpn_stop(parse_positive_u64(&intent_epoch, "VPN intent epoch")?)
        .map_err(to_napi_error)
}

#[napi]
pub async fn prepare_vpn(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .prepare_platform_vpn(&attempt_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn start_vpn(fd: i32, options_json: String, attempt_id: String) -> Result<()> {
    paws_core::shared_core()
        .start_platform_vpn(fd, &options_json, &attempt_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn stop_vpn(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .stop_platform_vpn(&attempt_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn complete_platform_vpn_cleanup(attempt_id: String) -> Result<bool> {
    paws_core::shared_core()
        .complete_platform_vpn_cleanup(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub fn extension_tick(attempt_id: String) -> Result<String> {
    paws_core::shared_core()
        .extension_tick(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub fn current_platform_vpn_session_id() -> Result<String> {
    paws_core::shared_core()
        .current_platform_vpn_session_id()
        .map_err(to_napi_error)
}

#[napi]
pub fn is_platform_vpn_session_current(
    session_id: String,
    expected_config_revision: String,
) -> Result<bool> {
    let revision = parse_config_revision(&expected_config_revision)?;
    paws_core::shared_core()
        .is_platform_vpn_session_current_at_revision(&session_id, revision)
        .map_err(to_napi_error)
}

#[napi]
pub fn is_runtime_config_revision_current(expected_config_revision: String) -> Result<bool> {
    let revision = parse_config_revision(&expected_config_revision)?;
    paws_core::shared_core()
        .is_runtime_config_revision_current(revision)
        .map_err(to_napi_error)
}

fn parse_config_revision(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::new(
            Status::InvalidArg,
            "config revision must be an unsigned decimal integer".to_owned(),
        ));
    }
    value.parse::<u64>().map_err(|_| {
        Error::new(
            Status::InvalidArg,
            "config revision is outside the supported u64 range".to_owned(),
        )
    })
}

fn parse_positive_u64(value: &str, label: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{label} must be an unsigned decimal integer"),
        ));
    }
    let parsed = value.parse::<u64>().map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{label} is outside the supported range"),
        )
    })?;
    if parsed == 0 {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{label} must be greater than zero"),
        ));
    }
    Ok(parsed)
}

#[napi]
pub fn persist_vpn_telemetry() -> Result<()> {
    paws_core::shared_core()
        .persist_vpn_telemetry()
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_starting(attempt_id: String, starting: bool) -> Result<bool> {
    paws_core::shared_core()
        .set_platform_vpn_starting_for_attempt(&attempt_id, starting)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_failed(attempt_id: String, error: String) -> Result<bool> {
    paws_core::shared_core()
        .set_platform_vpn_failed_for_attempt(&attempt_id, error)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_network_protected(
    attempt_id: String,
    protected: bool,
    error: Option<String>,
) -> Result<bool> {
    paws_core::shared_core()
        .set_platform_network_protected_for_attempt(&attempt_id, protected, error)
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_url_and_activate(
    url: String,
    name: Option<String>,
) -> Result<String> {
    let core = paws_core::shared_core();
    let expected_config_revision = core
        .config_projection()
        .map_err(to_napi_error)?
        .revisions
        .config_revision;
    core.import_profile_from_url_and_activate_checked(&url, name, expected_config_revision)
        .await
        .map(|receipt| receipt.profile_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_content_and_activate(
    name: String,
    source: String,
    raw_yaml: String,
) -> Result<String> {
    let core = paws_core::shared_core();
    let expected_config_revision = core
        .config_projection()
        .map_err(to_napi_error)?
        .revisions
        .config_revision;
    core.import_profile_from_content_and_activate_checked(
        &name,
        &source,
        &raw_yaml,
        None,
        expected_config_revision,
    )
    .await
    .map(|receipt| receipt.profile_id)
    .map_err(to_napi_error)
}

#[napi]
pub async fn validate_profile_content(raw_yaml: String) -> Result<()> {
    paws_core::shared_core()
        .validate_profile_content(&raw_yaml)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn profile_raw_yaml(profile_id: String) -> Result<String> {
    paws_core::shared_core()
        .profile_raw_yaml(&profile_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_mode(mode: String) -> Result<()> {
    let mode = RuntimeMode::try_from(mode.as_str()).map_err(to_napi_error)?;
    paws_core::shared_core()
        .set_mode(mode)
        .map_err(to_napi_error)
}

#[napi]
pub async fn select_proxy(group: String, proxy: String) -> Result<()> {
    paws_core::shared_core()
        .select_proxy_via_controller(&group, &proxy)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn unfix_proxy(group: String) -> Result<()> {
    paws_core::shared_core()
        .unfix_proxy_via_controller(&group)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn test_proxy_delay(
    proxy_name: String,
    url: Option<String>,
    timeout_ms: Option<i64>,
) -> Result<i32> {
    let delay = paws_core::shared_core()
        .test_proxy_delay_via_controller(
            &proxy_name,
            url.as_deref(),
            timeout_ms.and_then(|value| u64::try_from(value).ok()),
        )
        .await
        .map_err(to_napi_error)?;
    Ok(i32::from(delay))
}

#[napi]
pub async fn test_proxy_echo(
    proxy_name: String,
    url: String,
    payload: String,
    timeout_ms: Option<i64>,
) -> Result<String> {
    paws_core::shared_core()
        .test_proxy_echo(
            &proxy_name,
            &url,
            &payload,
            timeout_ms.and_then(|value| u64::try_from(value).ok()),
        )
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_provider(provider_name: String) -> Result<()> {
    paws_core::shared_core()
        .refresh_provider(&provider_name)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_provider_of_type(provider_type: String, provider_name: String) -> Result<()> {
    paws_core::shared_core()
        .refresh_provider_of_type(&provider_type, &provider_name)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_all_providers() -> Result<()> {
    paws_core::shared_core()
        .refresh_all_providers()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn close_connection(id: String) -> Result<()> {
    paws_core::shared_core()
        .close_connection(&id)
        .map_err(to_napi_error)
}

#[napi]
pub fn close_all_connections() -> Result<()> {
    paws_core::shared_core()
        .close_all_connections()
        .map_err(to_napi_error)
}

#[napi]
pub fn clear_request_history() -> Result<()> {
    paws_core::shared_core()
        .clear_request_history()
        .map_err(to_napi_error)
}

#[napi]
pub fn clear_logs() -> Result<()> {
    paws_core::shared_core().clear_logs().map_err(to_napi_error)
}

#[napi]
pub fn query_snapshot() -> Result<String> {
    paws_core::shared_core()
        .snapshot_json()
        .map_err(to_napi_error)
}

fn to_napi_error(error: paws_model::PawsError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
