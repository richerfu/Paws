use arkit::entry;
use arkit::prelude::Element;
use hmeta_model::{RuntimeMode, VpnOptions};
use napi_derive_ohos::napi;
#[cfg(target_env = "ohos")]
use napi_ohos::Env;
use napi_ohos::{bindgen_prelude::Object, Error, Result, Status};
#[cfg(target_env = "ohos")]
use ohos_resource_manager_binding::ResourceManager;
use std::collections::BTreeMap;
#[cfg(target_env = "ohos")]
use std::path::{Path, PathBuf};
#[cfg(target_env = "ohos")]
use std::{fs, io};

mod activity_filter;
mod l10n;
mod log_filter;
mod manual_rule;
mod mode_feedback;
mod notification;
mod platform_callbacks;
mod profile_filter;
mod profile_refresh_feedback;
mod provider_refresh_feedback;
mod proxy_filter;
mod proxy_grid;
mod resource_filter;
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

#[entry]
fn app() -> Element {
    ui::App()
}

#[napi]
pub fn configure_app_home(home_dir: String) -> Result<()> {
    std::env::set_var("HMETA_HOME", home_dir);
    Ok(())
}

#[napi]
pub fn configure_ui_locale(locale: String) -> Result<()> {
    std::env::set_var("HMETA_UI_LOCALE", locale);
    Ok(())
}

#[napi]
pub fn configure_system_color_mode(color_mode: i32) -> Result<()> {
    std::env::set_var("HMETA_SYSTEM_COLOR_MODE", color_mode.to_string());
    Ok(())
}

#[cfg(target_env = "ohos")]
const GEODATA_RAW_DIR: &str = "geodata";
#[cfg(target_env = "ohos")]
const GEODATA_SEED_FILES: &[(&str, &str)] = &[
    ("geodata/Country.mmdb", "Country.mmdb"),
    ("geodata/GeoLite2-ASN.mmdb", "GeoLite2-ASN.mmdb"),
    ("geodata/geosite.dat", "geosite.dat"),
];

#[cfg(target_env = "ohos")]
#[napi]
pub fn seed_geodata_from_rawfiles<'a>(
    env: Env,
    #[napi(ts_arg_type = "resourceManager.ResourceManager")] resource_manager: Object<'a>,
) -> Result<u32> {
    let home_dir = std::env::var("HMETA_HOME").map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "HMETA_HOME is not configured before geodata seed".to_owned(),
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

#[cfg(target_env = "ohos")]
fn write_seed_file(dest: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = dest.with_extension("seed.tmp");
    fs::write(&tmp, bytes).map_err(io_to_napi)?;
    fs::rename(&tmp, dest).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        io_to_napi(err)
    })
}

#[cfg(target_env = "ohos")]
fn io_to_napi(err: io::Error) -> Error {
    Error::new(Status::GenericFailure, err.to_string())
}

#[napi]
pub async fn prepare_vpn() -> Result<bool> {
    hmeta_core::shared_core()
        .prepare_active_vpn()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn start_vpn(fd: i32, options_json: String) -> Result<()> {
    hmeta_core::shared_core()
        .start_vpn(fd, &options_json)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn stop_vpn() -> Result<()> {
    hmeta_core::shared_core().stop_vpn().map_err(to_napi_error)
}

#[napi]
pub fn persist_vpn_telemetry() -> Result<()> {
    let core = hmeta_core::shared_core();
    let sync_core = core.clone();
    napi_ohos::bindgen_prelude::spawn(async move {
        let _ = sync_core.sync_external_controller_config().await;
    });
    core.persist_vpn_telemetry().map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_running(running: bool) -> Result<()> {
    hmeta_core::shared_core()
        .set_platform_vpn_running(running)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_starting(starting: bool) -> Result<()> {
    hmeta_core::shared_core()
        .set_platform_vpn_starting(starting)
        .map_err(to_napi_error)
}

#[napi]
pub fn expire_platform_vpn_start() -> Result<bool> {
    hmeta_core::shared_core()
        .expire_platform_vpn_start()
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_vpn_failed(error: String) -> Result<()> {
    hmeta_core::shared_core()
        .set_platform_vpn_failed(error)
        .map_err(to_napi_error)
}

#[napi]
pub fn set_platform_network_protected(protected: bool, error: Option<String>) -> Result<()> {
    hmeta_core::shared_core()
        .set_platform_network_protected(protected, error)
        .map_err(to_napi_error)
}

#[napi]
pub async fn reload_config(profile_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .reload_config(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_url(url: String, name: Option<String>) -> Result<String> {
    hmeta_core::shared_core()
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
    hmeta_core::shared_core()
        .import_profile_from_content(&name, &source, &raw_yaml, None)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn import_profile_from_picker() -> Result<String> {
    let (name, raw_yaml) = platform_callbacks::pick_profile_text()
        .await
        .map_err(|err| Error::new(Status::GenericFailure, err))?;
    hmeta_core::shared_core()
        .import_profile_from_content(&name, "local-file", &raw_yaml, None)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_profile(profile_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .refresh_profile(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_all_profiles() -> Result<()> {
    hmeta_core::shared_core()
        .refresh_all_profiles()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_due_profiles() -> Result<()> {
    hmeta_core::shared_core()
        .refresh_due_profiles()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn update_profile_content(profile_id: String, raw_yaml: String) -> Result<()> {
    hmeta_core::shared_core()
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
    hmeta_core::shared_core()
        .update_profile_subscription(&profile_id, &name, &subscription_url)
        .map_err(to_napi_error)
}

#[napi]
pub async fn validate_profile_content(raw_yaml: String) -> Result<()> {
    hmeta_core::shared_core()
        .validate_profile_content(&raw_yaml)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn profile_raw_yaml(profile_id: String) -> Result<String> {
    hmeta_core::shared_core()
        .profile_raw_yaml(&profile_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn restore_profile_backup(profile_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .restore_profile_backup(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_profile_dns_servers(profile_id: String, dns_servers_json: String) -> Result<()> {
    let dns_servers: Vec<String> = serde_json::from_str(&dns_servers_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    hmeta_core::shared_core()
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
    hmeta_core::shared_core()
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
    hmeta_core::shared_core()
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
pub async fn activate_profile(profile_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .activate_profile(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn delete_profile(profile_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .delete_profile(&profile_id)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn set_mode(mode: String) -> Result<()> {
    let mode = RuntimeMode::try_from(mode.as_str()).map_err(to_napi_error)?;
    hmeta_core::shared_core()
        .set_mode(mode)
        .map_err(to_napi_error)
}

#[napi]
pub async fn select_proxy(group: String, proxy: String) -> Result<()> {
    hmeta_core::shared_core()
        .select_proxy_via_controller(&group, &proxy)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn unfix_proxy(group: String) -> Result<()> {
    hmeta_core::shared_core()
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
    let ids = hmeta_core::shared_core()
        .import_rules_from_content(profile_id.as_deref(), &source, &rules_text)
        .map_err(to_napi_error)?;
    serde_json::to_string(&ids).map_err(|err| Error::new(Status::GenericFailure, err.to_string()))
}

#[napi]
pub fn set_rule_enabled(profile_id: String, rule_id: String, enabled: bool) -> Result<()> {
    hmeta_core::shared_core()
        .set_rule_enabled(&profile_id, &rule_id, enabled)
        .map_err(to_napi_error)
}

#[napi]
pub fn reorder_rules(profile_id: String, ordered_rule_ids_json: String) -> Result<()> {
    let ordered_rule_ids: Vec<String> = serde_json::from_str(&ordered_rule_ids_json)
        .map_err(|err| Error::new(Status::InvalidArg, err.to_string()))?;
    hmeta_core::shared_core()
        .reorder_rules(&profile_id, &ordered_rule_ids)
        .map_err(to_napi_error)
}

#[napi]
pub fn delete_rule(rule_id: String) -> Result<()> {
    hmeta_core::shared_core()
        .delete_rule(&rule_id)
        .map_err(to_napi_error)
}

#[napi]
pub async fn test_proxy_delay(
    proxy_name: String,
    url: Option<String>,
    timeout_ms: Option<i64>,
) -> Result<i32> {
    let delay = hmeta_core::shared_core()
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
    hmeta_core::shared_core()
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
    hmeta_core::shared_core()
        .refresh_provider(&provider_name)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_provider_of_type(provider_type: String, provider_name: String) -> Result<()> {
    hmeta_core::shared_core()
        .refresh_provider_of_type(&provider_type, &provider_name)
        .await
        .map_err(to_napi_error)
}

#[napi]
pub async fn refresh_all_providers() -> Result<()> {
    hmeta_core::shared_core()
        .refresh_all_providers()
        .await
        .map_err(to_napi_error)
}

#[napi]
pub fn close_connection(id: String) -> Result<()> {
    hmeta_core::shared_core()
        .close_connection(&id)
        .map_err(to_napi_error)
}

#[napi]
pub fn close_all_connections() -> Result<()> {
    hmeta_core::shared_core()
        .close_all_connections()
        .map_err(to_napi_error)
}

#[napi]
pub fn clear_request_history() -> Result<()> {
    hmeta_core::shared_core()
        .clear_request_history()
        .map_err(to_napi_error)
}

#[napi]
pub fn clear_logs() -> Result<()> {
    hmeta_core::shared_core()
        .clear_logs()
        .map_err(to_napi_error)
}

#[napi]
pub fn query_snapshot() -> Result<String> {
    hmeta_core::shared_core()
        .snapshot_json()
        .map_err(to_napi_error)
}

#[napi]
pub fn default_vpn_options() -> Result<String> {
    hmeta_model::to_json(&VpnOptions::default()).map_err(to_napi_error)
}

#[napi]
pub fn register_platform_callbacks(callbacks: Object<'static>) -> Result<()> {
    platform_callbacks::register_platform_callbacks(callbacks)
}

fn to_napi_error(error: hmeta_model::HMetaError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
