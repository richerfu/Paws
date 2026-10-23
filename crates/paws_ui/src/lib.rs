use napi_derive_ohos::napi;
use napi_ohos::{bindgen_prelude::Object, Env, Error, Result, Status};
use ohos_resource_manager_binding::ResourceManager;
use paws_model::RuntimeMode;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{fs, io};

// This cdylib is the platform binding; NSH owns all view rendering.
mod engine_bridge;
mod vpn_handoff;
pub use engine_bridge::{
    clear_engine_process_resolver, engine_tun_mtu, poll_engine_observations, poll_engine_traffic,
    prepare_engine, register_engine_process_resolver, reload_engine, start_engine, stop_engine,
    trim_engine_memory,
};

#[napi]
pub fn configure_app_home(home_dir: String) -> Result<()> {
    paws_core::configure_app_home(std::path::Path::new(&home_dir)).map_err(to_napi_error)
}

#[napi]
pub fn set_physical_network(net_id: i32) -> Result<()> {
    paws_core::network_binding::configure_physical_network(net_id).map_err(to_napi_error)
}

#[napi]
pub async fn persist_engine_session(session: String, engine: String, net_id: i32) -> Result<()> {
    paws_core::shared_core()
        .persist_engine_session(&session, &engine, net_id)
        .await
        .map_err(to_napi_error)
}
#[napi]
pub fn engine_decision(options_json: String) -> Result<String> {
    serde_json::to_string(
        &paws_core::shared_core()
            .engine_decision(&options_json)
            .map_err(to_napi_error)?,
    )
    .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}
#[napi]
pub fn record_engine_health(session: String, success: bool, stage: String) -> Result<()> {
    paws_core::shared_core()
        .record_engine_health(&session, success, &stage)
        .map_err(to_napi_error)
}
#[napi]
pub fn record_engine_qualification(text: String) -> Result<()> {
    paws_core::shared_core()
        .record_engine_qualification(&text)
        .map_err(to_napi_error)
}

#[napi]
pub async fn validate_engine_startup(session: String) -> Result<String> {
    serde_json::to_string(
        &paws_core::shared_core()
            .validate_engine_startup(&session)
            .await
            .map_err(to_napi_error)?,
    )
    .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
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
pub fn prepare_platform_vpn_handoff(attempt_id: String) -> Result<String> {
    vpn_handoff::prepare(&attempt_id)
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
}

#[napi]
pub async fn send_platform_vpn_handoff(
    ashmem_fd: i32,
    notification_fd: i32,
    attempt_id: String,
    options_json: String,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let pending_attempt_id = attempt_id.clone();
        let core = paws_core::shared_core();
        vpn_handoff::send(ashmem_fd, notification_fd, attempt_id, options_json, || {
            core.validate_platform_vpn_start_request(
                ashmem_fd,
                notification_fd,
                &pending_attempt_id,
            )
            .map_err(|error| io::Error::other(error.to_string()))
        })
    })
    .await
    .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?
    .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
}

#[napi]
pub async fn receive_and_attach_platform_vpn_handoff(token: String) -> Result<String> {
    let received = tokio::task::spawn_blocking(move || vpn_handoff::receive(&token))
        .await
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
    let core = paws_core::shared_core();
    core.validate_platform_vpn_start_request(
        received.ashmem.as_raw_fd(),
        received.notification.as_raw_fd(),
        &received.payload.attempt_id,
    )
    .map_err(to_napi_error)?;
    core.attach_platform_shared_memory(
        received.ashmem.as_raw_fd(),
        received.notification.as_raw_fd(),
    )
    .map_err(to_napi_error)?;
    serde_json::to_string(&received.payload)
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
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
    if paws_app::PreferencesStore::set_system_locale(locale).map_err(app_error)? {
        cancel_ui_revision_wait();
    }
    Ok(())
}

#[napi]
pub fn configure_system_color_mode(color_mode: i32) -> Result<()> {
    if !matches!(color_mode, -1..=1) {
        return Err(Error::from_reason("invalid system color mode"));
    }
    if paws_app::PreferencesStore::set_system_color_mode(color_mode).map_err(app_error)? {
        cancel_ui_revision_wait();
    }
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
pub fn claim_platform_vpn_auto_recovery() -> Result<String> {
    Ok(paws_core::shared_core()
        .claim_platform_vpn_auto_recovery()
        .map_err(to_napi_error)?
        .unwrap_or_default())
}

#[napi]
pub fn rearm_platform_vpn_auto_recovery(error: String) {
    paws_core::shared_core().rearm_platform_vpn_auto_recovery(&error);
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
pub fn resolve_vpn_engine(options_json: String) -> Result<String> {
    paws_core::shared_core()
        .resolve_vpn_engine(&options_json)
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
pub fn query_vpn_speed() -> Result<String> {
    paws_core::shared_core()
        .vpn_speed_json()
        .map_err(to_napi_error)
}

/// Debug Want acceptance checks run in the real application's UID and HTTP
/// routing context. The result contains counts and outcomes, never credentials.
#[napi]
pub async fn verify_app_http_regression() -> Result<String> {
    if !cfg!(debug_assertions) {
        return Err(Error::new(
            Status::GenericFailure,
            "acceptance checks require a debug native build".to_owned(),
        ));
    }
    ohos_hilog_binding::info("PawsHttpQA: acceptance begins");
    let core = paws_core::shared_core();
    core.set_engine_log_recording_enabled(true)
        .await
        .map_err(to_napi_error)?;
    ohos_hilog_binding::info("PawsHttpQA: log recording ready");
    let before = core.command_context().map_err(to_napi_error)?;
    let profile_id = before
        .active_profile
        .ok_or_else(|| Error::from_reason("acceptance requires an active profile"))?;
    if !before.vpn_running {
        return Err(Error::from_reason("acceptance requires a connected VPN"));
    }
    let started = std::time::Instant::now();
    ohos_hilog_binding::info("PawsHttpQA: subscription refresh begins");
    core.refresh_profile(&profile_id)
        .await
        .map_err(to_napi_error)?;
    let refresh_ms = started.elapsed().as_millis();
    ohos_hilog_binding::info(format!(
        "PawsHttpQA: subscription refresh completed in {refresh_ms} ms"
    ));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut ip_ok = false;
    while tokio::time::Instant::now() < deadline {
        core.refresh_exit_location_if_due()
            .await
            .map_err(to_napi_error)?;
        let status = acceptance_domain(&core, paws_core::RuntimeDomain::Status).await?;
        if status["exitLocation"]["ip"]
            .as_str()
            .is_some_and(|ip| !ip.is_empty())
        {
            ip_ok = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    if !ip_ok {
        let status = acceptance_domain(&core, paws_core::RuntimeDomain::Status).await?;
        let error = status["exitLocation"]["error"]
            .as_str()
            .unwrap_or("exit IP acceptance timed out")
            .to_owned();
        return Err(Error::from_reason(error));
    }
    ohos_hilog_binding::info("PawsHttpQA: exit IP detected");
    let negative = core
        .prepare_profile_import_from_url(
            "http://127.0.0.1:9/qa-private-diagnostic-token",
            Some("negative HTTP acceptance".to_owned()),
        )
        .await;
    if negative.is_ok() {
        return Err(Error::from_reason(
            "negative HTTP acceptance unexpectedly succeeded",
        ));
    }
    let logs = acceptance_domain(&core, paws_core::RuntimeDomain::Logs).await?;
    let error_logged = logs["logs"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["level"].as_str() == Some("error")
                && entry["message"].as_str().is_some_and(|message| {
                    message.contains("profile download failed")
                        && !message.contains("qa-private-diagnostic-token")
                })
        })
    });
    if !error_logged {
        return Err(Error::from_reason(
            "HTTP failure was not recorded with redaction",
        ));
    }
    let config = acceptance_domain(&core, paws_core::RuntimeDomain::Config).await?;
    let rules = acceptance_domain(&core, paws_core::RuntimeDomain::Rules).await?;
    let about = acceptance_domain(&core, paws_core::RuntimeDomain::About).await?;
    let output = serde_json::json!({
        "subscriptionRefreshMs": refresh_ms,
        "ipDetected": ip_ok,
        "errorLogged": error_logged,
        "publicMixedEnabled": config["networkPorts"]["mixedEnabled"],
        "rules": rules["rules"].as_array().map(Vec::len),
        "meowVersion": about["about"]["meowRsVersion"],
    });
    paws_model::to_json(&output).map_err(to_napi_error)
}

/// Acceptance reads the same explicitly requested domain as the UI. It never
/// recreates the removed aggregate snapshot transport, including in its poll.
async fn acceptance_domain(
    core: &paws_core::CoreHandle,
    domain: paws_core::RuntimeDomain,
) -> Result<serde_json::Value> {
    let projection = core
        .refresh_and_query_runtime_domain(paws_core::RuntimeDomainKey {
            domain,
            group_key: None,
        })
        .await
        .map_err(to_napi_error)?;
    let mut value =
        serde_json::to_value(projection).map_err(|error| Error::from_reason(error.to_string()))?;
    Ok(value["data"].take())
}

fn to_napi_error(error: paws_model::PawsError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

fn app_error(error: paws_app::AppError) -> Error {
    Error::from_reason(error.to_string())
}

static UI_WAIT_CANCEL: std::sync::LazyLock<tokio::sync::watch::Sender<u64>> =
    std::sync::LazyLock::new(|| tokio::sync::watch::channel(0).0);

/// A host gateway owns one application command session. Disposing the gateway
/// cancels import preparation, while an acquired commit finishes its response
/// and platform effect independently of component event delivery.
#[napi]
pub struct UiCommandSession {
    application: std::sync::Arc<paws_app::Application>,
}

#[napi]
impl UiCommandSession {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            application: std::sync::Arc::new(paws_app::Application::shared()),
        }
    }

    #[napi]
    pub async fn dispatch(&self, request_json: String) -> Result<String> {
        self.application
            .dispatch_json(&request_json)
            .await
            .map_err(app_error)
    }

    #[napi]
    pub async fn query_state(&self, request_json: String) -> Result<String> {
        let application = self.application.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(application.query_state_json(&request_json))
        })
        .await
        .map_err(|error| Error::from_reason(error.to_string()))?
        .map_err(app_error)
    }

    #[napi]
    pub fn subscribe_state(&self, request_json: String) -> Result<String> {
        self.application
            .subscribe_state_json(&request_json)
            .map_err(app_error)
    }

    #[napi]
    pub fn observe_state(&self, critical_only: bool) -> Result<String> {
        self.application
            .observe_state_json(critical_only)
            .map_err(app_error)
    }

    #[napi]
    pub fn close(&self) {
        self.application.close();
    }
}
impl Default for UiCommandSession {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for UiCommandSession {
    fn drop(&mut self) {
        self.application.close();
    }
}

#[napi]
pub fn query_ui_preferences() -> Result<String> {
    let preferences = paws_app::PreferencesStore::configured()
        .map_err(app_error)?
        .snapshot()
        .map_err(app_error)?;
    serde_json::to_string(&preferences).map_err(|error| Error::from_reason(error.to_string()))
}

#[napi]
pub fn query_active_vpn_options() -> Result<String> {
    paws_core::shared_core()
        .active_vpn_options_json()
        .map_err(to_napi_error)
}

#[napi]
pub async fn wait_for_ui_revision(after_revision: String) -> Result<String> {
    let after = after_revision
        .parse::<u64>()
        .map_err(|_| Error::from_reason("invalid UI revision"))?;
    let mut cancel = UI_WAIT_CANCEL.subscribe();
    let application = paws_app::Application::shared();
    let revision = tokio::select! {
        revision = application.wait_for_revision(after) => revision.map_err(app_error)?,
        _ = cancel.changed() => paws_core::shared_core().subscribe_runtime_revisions().borrow().revision,
    };
    Ok(revision.to_string())
}

#[napi]
pub fn cancel_ui_revision_wait() {
    UI_WAIT_CANCEL.send_modify(|generation| *generation = generation.wrapping_add(1));
}
