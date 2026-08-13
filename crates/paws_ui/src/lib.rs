use paws_model::{RuntimeMode, VpnOptions};
use napi_derive_ohos::napi;
use napi_ohos::{bindgen_prelude::Object, Env, Error, Result, Status};
use ohos_resource_manager_binding::ResourceManager;
use std::collections::BTreeMap;
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
mod resource_filter;
mod route_status;
mod rule_feedback;
mod settings_feedback;
mod subscription_converter;
mod subscription_scan;
mod time_format;
mod traffic_history;
mod ui;
mod ui_preferences;
mod vpn_feedback;
mod yaml_summary;

use arkit::entry;
use arkit::prelude::Element;

/// Application entry: arkit's `#[entry]` generates the NAPI init/render/
/// destroy lifecycle and bridge event ports, registers the `paws.*` bridge
/// plugins declaratively, and passes the shared `OpenHarmonyApp` handle into
/// the entry function so the platform call surface can resolve it.
#[entry(plugins = [
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
    std::env::set_var("PAWS_HOME", home_dir);
    Ok(())
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
pub async fn wait_for_platform_change(timeout_ms: u32) -> Result<bool> {
    paws_core::shared_core()
        .wait_for_platform_change(std::time::Duration::from_millis(u64::from(timeout_ms)))
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn sync_platform_changes() -> Result<()> {
    paws_core::shared_core()
        .sync_platform_changes()
        .map_err(to_napi_error)
}

#[napi]
pub fn begin_platform_vpn_start() -> Result<String> {
    paws_core::shared_core()
        .begin_platform_vpn_start()
        .map_err(to_napi_error)
}

#[napi]
pub fn bind_platform_vpn_start(attempt_id: String) -> Result<()> {
    paws_core::shared_core()
        .bind_platform_vpn_start(&attempt_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn await_platform_vpn_start_attachment(
    attempt_id: String,
    timeout_ms: u32,
) -> Result<bool> {
    paws_core::shared_core()
        .await_platform_vpn_start_attachment(
            &attempt_id,
            std::time::Duration::from_millis(u64::from(timeout_ms)),
        )
        .await
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
pub fn configure_ui_locale(locale: String) -> Result<()> {
    std::env::set_var("PAWS_UI_LOCALE", locale);
    Ok(())
}

#[napi]
pub fn configure_system_color_mode(color_mode: i32) -> Result<()> {
    std::env::set_var("PAWS_SYSTEM_COLOR_MODE", color_mode.to_string());
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
pub async fn prepare_vpn() -> Result<bool> {
    paws_core::shared_core()
        .prepare_active_vpn()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn start_vpn(fd: i32, options_json: String) -> Result<()> {
    paws_core::shared_core()
        .start_vpn(fd, &options_json)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn stop_vpn() -> Result<()> {
    paws_core::shared_core().stop_vpn().map_err(to_napi_error)
}

#[napi]
pub fn persist_vpn_telemetry() -> Result<()> {
    let core = paws_core::shared_core();
    let sync_core = core.clone();
    napi_ohos::bindgen_prelude::spawn(async move {
        let _ = sync_core.sync_external_controller_config().await;
    });
    core.persist_vpn_telemetry().map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_running(running: bool) -> Result<()> {
    paws_core::shared_core()
        .set_platform_vpn_running(running)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_starting(starting: bool) -> Result<()> {
    paws_core::shared_core()
        .set_platform_vpn_starting(starting)
        .map_err(to_napi_error)
}

#[napi]
pub fn expire_platform_vpn_start() -> Result<bool> {
    paws_core::shared_core()
        .expire_platform_vpn_start()
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_failed(error: String) -> Result<()> {
    paws_core::shared_core()
        .set_platform_vpn_failed(error)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_network_protected(protected: bool, error: Option<String>) -> Result<()> {
    paws_core::shared_core()
        .set_platform_network_protected(protected, error)
        .map_err(to_napi_error)
}

#[napi]
pub async fn reload_config(profile_id: String) -> Result<()> {
    paws_core::shared_core()
        .reload_config(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_url(url: String, name: Option<String>) -> Result<String> {
    paws_core::shared_core()
        .import_profile_from_url(&url, name)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_content(
    name: String,
    source: String,
    raw_yaml: String,
) -> Result<String> {
    paws_core::shared_core()
        .import_profile_from_content(&name, &source, &raw_yaml, None)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_picker() -> Result<String> {
    let (name, raw_yaml) = bridge::pick_profile_text()
        .await
        .map_err(|err| Error::new(Status::GenericFailure, err))?;
    paws_core::shared_core()
        .import_profile_from_content(&name, "local-file", &raw_yaml, None)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_profile(profile_id: String) -> Result<()> {
    paws_core::shared_core()
        .refresh_profile(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_all_profiles() -> Result<()> {
    paws_core::shared_core()
        .refresh_all_profiles()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_due_profiles() -> Result<()> {
    paws_core::shared_core()
        .refresh_due_profiles()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn update_profile_content(profile_id: String, raw_yaml: String) -> Result<()> {
    paws_core::shared_core()
        .update_profile_content(&profile_id, &raw_yaml)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn update_profile_subscription(
    profile_id: String,
    name: String,
    subscription_url: String,
) -> Result<()> {
    paws_core::shared_core()
        .update_profile_subscription(&profile_id, &name, &subscription_url)
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
pub async fn restore_profile_backup(profile_id: String) -> Result<()> {
    paws_core::shared_core()
        .restore_profile_backup(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_profile_dns_servers(profile_id: String, dns_servers_json: String) -> Result<()> {
    let dns_servers: Vec<String> = serde_json::from_str(&dns_servers_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    paws_core::shared_core()
        .set_profile_dns_servers(&profile_id, dns_servers)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_profile_dns_config(
    profile_id: String,
    dns_servers_json: String,
    dns_fallbacks_json: String,
    dns_nameserver_policy_json: String,
) -> Result<()> {
    let dns_servers: Vec<String> = serde_json::from_str(&dns_servers_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    let dns_fallbacks: Vec<String> = serde_json::from_str(&dns_fallbacks_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    let dns_nameserver_policy: BTreeMap<String, Vec<String>> =
        serde_json::from_str(&dns_nameserver_policy_json)
            .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    paws_core::shared_core()
        .set_profile_dns_config(
            &profile_id,
            dns_servers,
            dns_fallbacks,
            dns_nameserver_policy,
        )
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_profile_vpn_config(
    profile_id: String,
    system_proxy: bool,
    dns_hijacking: bool,
    allow_bypass: bool,
    stack: String,
) -> Result<()> {
    paws_core::shared_core()
        .set_profile_vpn_config(
            &profile_id,
            system_proxy,
            dns_hijacking,
            allow_bypass,
            stack,
        )
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_profile_network_config(
    profile_id: String,
    mixed_port: u16,
    controller_port: u16,
    allow_lan: bool,
) -> Result<()> {
    paws_core::shared_core()
        .set_profile_network_config(
            &profile_id,
            paws_model::NetworkPortConfig {
                mixed_port,
                controller_port,
            },
            allow_lan,
        )
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn activate_profile(profile_id: String) -> Result<()> {
    paws_core::shared_core()
        .activate_profile(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn delete_profile(profile_id: String) -> Result<()> {
    paws_core::shared_core()
        .delete_profile(&profile_id)
        .await
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
pub fn import_rules_from_content(
    profile_id: Option<String>,
    source: String,
    rules_text: String,
) -> Result<String> {
    let ids = paws_core::shared_core()
        .import_rules_from_content(profile_id.as_deref(), &source, &rules_text)
        .map_err(to_napi_error)?;
    serde_json::to_string(&ids).map_err(|err| Error::new(Status::GenericFailure, err.to_string()))
}

#[napi]
pub fn set_rule_enabled(profile_id: String, rule_id: String, enabled: bool) -> Result<()> {
    paws_core::shared_core()
        .set_rule_enabled(&profile_id, &rule_id, enabled)
        .map_err(to_napi_error)
}

#[napi]
pub fn reorder_rules(profile_id: String, ordered_rule_ids_json: String) -> Result<()> {
    let ordered_rule_ids: Vec<String> = serde_json::from_str(&ordered_rule_ids_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    paws_core::shared_core()
        .reorder_rules(&profile_id, &ordered_rule_ids)
        .map_err(to_napi_error)
}

#[napi]
pub fn delete_rule(rule_id: String) -> Result<()> {
    paws_core::shared_core()
        .delete_rule(&rule_id)
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
    paws_core::shared_core()
        .clear_logs()
        .map_err(to_napi_error)
}

#[napi]
pub fn query_snapshot() -> Result<String> {
    paws_core::shared_core()
        .snapshot_json()
        .map_err(to_napi_error)
}

#[napi]
pub fn default_vpn_options() -> Result<String> {
    paws_model::to_json(&VpnOptions::default()).map_err(to_napi_error)
}

fn to_napi_error(error: paws_model::PawsError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
