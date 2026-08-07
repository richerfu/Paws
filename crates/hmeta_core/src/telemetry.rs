use super::*;

struct RuntimeUiCacheQueue {
    pending: Mutex<HashMap<PathBuf, RuntimeUiCache>>,
    ready: std::sync::Condvar,
}

struct RuntimeUiCacheWriter {
    queue: Option<Arc<RuntimeUiCacheQueue>>,
}

impl RuntimeUiCacheWriter {
    fn start() -> Self {
        let queue = Arc::new(RuntimeUiCacheQueue {
            pending: Mutex::new(HashMap::new()),
            ready: std::sync::Condvar::new(),
        });
        let worker_queue = Arc::clone(&queue);
        let worker = std::thread::Builder::new()
            .name("hmeta-ui-cache-writer".to_owned())
            .spawn(move || loop {
                let pending = {
                    let Ok(mut pending) = worker_queue.pending.lock() else {
                        tracing::warn!(
                            target: "hmeta_core::telemetry",
                            "runtime UI cache queue lock poisoned"
                        );
                        return;
                    };
                    while pending.is_empty() {
                        let Ok(next) = worker_queue.ready.wait(pending) else {
                            tracing::warn!(
                                target: "hmeta_core::telemetry",
                                "runtime UI cache queue wait failed"
                            );
                            return;
                        };
                        pending = next;
                    }
                    std::mem::take(&mut *pending)
                };
                for (path, cache) in pending {
                    if let Err(error) = persist_runtime_ui_cache_value(&cache, &path) {
                        tracing::warn!(
                            target: "hmeta_core::telemetry",
                            "runtime UI cache persist failed: {error}"
                        );
                    }
                }
            });
        match worker {
            Ok(_) => Self { queue: Some(queue) },
            Err(error) => {
                tracing::warn!(
                    target: "hmeta_core::telemetry",
                    "runtime UI cache writer could not start: {error}"
                );
                Self { queue: None }
            }
        }
    }

    fn schedule(&self, cache: RuntimeUiCache, path: PathBuf) {
        let Some(queue) = &self.queue else {
            return;
        };
        let Ok(mut pending) = queue.pending.lock() else {
            tracing::warn!(
                target: "hmeta_core::telemetry",
                "runtime UI cache queue lock poisoned"
            );
            return;
        };
        pending.insert(path, cache);
        drop(pending);
        queue.ready.notify_one();
    }
}

static RUNTIME_UI_CACHE_WRITER: Lazy<RuntimeUiCacheWriter> = Lazy::new(RuntimeUiCacheWriter::start);

pub(super) fn apply_traffic_sample(
    state: &mut CoreState,
    stats: &TunStats,
) -> Result<(), HMetaError> {
    // Reading from a TUN descriptor receives packets written by applications
    // (device -> network), while writing to it delivers packets back to those
    // applications (network -> device). `TunStats` uses rx/tx from the native
    // descriptor's point of view, so their user-facing meanings are inverted.
    let upload_total = stats.rx_bytes;
    let download_total = stats.tx_bytes;
    let now = Instant::now();
    let (upload_delta, download_delta) =
        if let Some((last_at, last_upload, last_download)) = state.last_traffic_sample {
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let upload_delta = upload_total.saturating_sub(last_upload);
            let download_delta = download_total.saturating_sub(last_download);
            state.traffic.tun_upload_speed = ((upload_delta as f64) / elapsed) as u64;
            state.traffic.tun_download_speed = ((download_delta as f64) / elapsed) as u64;
            (upload_delta, download_delta)
        } else {
            (upload_total, download_total)
        };

    if let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) {
        state
            .profiles
            .add_profile_traffic(&profile_id, upload_delta, download_delta)?;
    }
    state.traffic.tun_upload_bytes = upload_total;
    state.traffic.tun_download_bytes = download_total;
    state.traffic.upload_bytes = state.traffic.tun_upload_bytes;
    state.traffic.download_bytes = state.traffic.tun_download_bytes;
    state.traffic.upload_speed = state.traffic.tun_upload_speed;
    state.traffic.download_speed = state.traffic.tun_download_speed;
    state.last_traffic_sample = Some((now, upload_total, download_total));
    Ok(())
}

pub(super) fn apply_meow_traffic_sample(
    state: &mut CoreState,
    tunnel: &Tunnel,
    use_as_primary: bool,
) -> Result<(), HMetaError> {
    let (upload_total, download_total) = tunnel.statistics().snapshot();
    let upload_total = non_negative_i64_to_u64(upload_total);
    let download_total = non_negative_i64_to_u64(download_total);
    let now = Instant::now();
    let (upload_delta, download_delta) =
        if let Some((last_at, last_upload, last_download)) = state.last_meow_traffic_sample {
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let upload_delta = upload_total.saturating_sub(last_upload);
            let download_delta = download_total.saturating_sub(last_download);
            state.traffic.meow_upload_speed = ((upload_delta as f64) / elapsed) as u64;
            state.traffic.meow_download_speed = ((download_delta as f64) / elapsed) as u64;
            (upload_delta, download_delta)
        } else {
            (upload_total, download_total)
        };

    if use_as_primary {
        if let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) {
            state
                .profiles
                .add_profile_traffic(&profile_id, upload_delta, download_delta)?;
        }
        state.traffic.upload_bytes = upload_total;
        state.traffic.download_bytes = download_total;
        state.traffic.upload_speed = state.traffic.meow_upload_speed;
        state.traffic.download_speed = state.traffic.meow_download_speed;
    }
    state.traffic.meow_upload_bytes = upload_total;
    state.traffic.meow_download_bytes = download_total;
    state.last_meow_traffic_sample = Some((now, upload_total, download_total));
    Ok(())
}

pub(super) fn baseline_meow_traffic_sample(state: &mut CoreState) {
    let Some(tunnel) = state.tunnel.clone() else {
        return;
    };
    let (upload_total, download_total) = tunnel.statistics().snapshot();
    let upload_total = non_negative_i64_to_u64(upload_total);
    let download_total = non_negative_i64_to_u64(download_total);
    state.traffic.meow_upload_bytes = upload_total;
    state.traffic.meow_download_bytes = download_total;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    state.last_meow_traffic_sample = Some((Instant::now(), upload_total, download_total));
}

pub(super) fn settle_traffic_before_platform_stop(
    state: &mut CoreState,
    tun_stats: Option<&TunStats>,
) -> Result<(), HMetaError> {
    if let Some(stats) = tun_stats {
        apply_traffic_sample(state, stats)?;
        baseline_meow_traffic_sample(state);
    } else if let Some(tunnel) = state.tunnel.clone() {
        apply_meow_traffic_sample(state, &tunnel, true)?;
    }
    state.traffic.upload_speed = 0;
    state.traffic.download_speed = 0;
    state.traffic.tun_upload_speed = 0;
    state.traffic.tun_download_speed = 0;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    Ok(())
}

pub(super) fn settle_traffic_before_profile_switch(
    state: &mut CoreState,
    tun_stats: Option<&TunStats>,
) -> Result<(), HMetaError> {
    if let Some(stats) = tun_stats {
        apply_traffic_sample(state, stats)?;
        baseline_meow_traffic_sample(state);
    } else if let Some(tunnel) = state.tunnel.clone() {
        apply_meow_traffic_sample(state, &tunnel, true)?;
    }
    state.traffic.upload_speed = 0;
    state.traffic.download_speed = 0;
    state.traffic.tun_upload_speed = 0;
    state.traffic.tun_download_speed = 0;
    state.traffic.meow_upload_speed = 0;
    state.traffic.meow_download_speed = 0;
    Ok(())
}

pub(super) fn record_traffic_history(state: &mut CoreState) {
    state.traffic_history.push_back(TrafficHistoryPoint {
        download_speed: state.traffic.download_speed,
        upload_speed: state.traffic.upload_speed,
    });
    while state.traffic_history.len() > MAX_TRAFFIC_HISTORY {
        state.traffic_history.pop_front();
    }
}

pub(super) fn proxy_test_metadata(url: &str, in_name: &str) -> Result<Metadata, HMetaError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|err| HMetaError::Core(format!("invalid proxy test URL: {err}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| HMetaError::Core("proxy test URL has no host".to_owned()))?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    Ok(Metadata {
        network: Network::Tcp,
        // The echo probe writes an opaque payload after `dial_tcp`; it is not
        // an HTTP request even when its target is expressed as an http(s) URL.
        // `Inner` makes HTTP outbound adapters establish a CONNECT tunnel
        // instead of treating the payload as an HTTP-forward-proxy request.
        conn_type: ConnType::Inner,
        dst_port: port,
        host: host.into(),
        in_name: in_name.into(),
        in_port: 0,
        ..Metadata::default()
    })
}

pub(super) fn dns_snapshot(options: &VpnOptions, tun_stats: Option<&TunStats>) -> DnsSnapshot {
    DnsSnapshot {
        model: if options.dns_hijacking {
            "tun-hijack".to_owned()
        } else {
            "meow-listener".to_owned()
        },
        hijacking: options.dns_hijacking,
        listen: "127.0.0.1:1053".to_owned(),
        upstreams: options.dns_servers.clone(),
        fallbacks: options.dns_fallbacks.clone(),
        nameserver_policy: options.dns_nameserver_policy.clone(),
        tun_addresses: options.dns_addresses.clone(),
        handled_packets: tun_stats.map(|stats| stats.dns_packets).unwrap_or(0),
        cache_hits: tun_stats.map(|stats| stats.dns_cache_hits).unwrap_or(0),
        cache_misses: tun_stats.map(|stats| stats.dns_cache_misses).unwrap_or(0),
        recent_queries: tun_stats
            .map(|stats| stats.recent_dns_queries.clone())
            .unwrap_or_default(),
    }
}

pub(super) fn runtime_ui_cache_path(profiles: &ProfileStore) -> PathBuf {
    profiles.root().join(RUNTIME_UI_CACHE_FILE)
}

pub(super) fn active_profile_revision(profiles: &ProfileStore, profile_id: &str) -> Option<String> {
    profiles
        .profile(profile_id)
        .ok()
        .and_then(|profile| profile.updated_at.clone())
}

pub(super) fn load_runtime_ui_cache(profiles: &ProfileStore) -> Option<RuntimeUiCache> {
    let active_profile = profiles.active_profile()?;
    let content = fs::read(runtime_ui_cache_path(profiles)).ok()?;
    let cache = serde_json::from_slice::<RuntimeUiCache>(&content).ok()?;
    if cache.version != RUNTIME_UI_CACHE_VERSION
        || cache.active_profile != active_profile
        || cache.profile_updated_at != active_profile_revision(profiles, active_profile)
    {
        return None;
    }
    Some(cache)
}

/// Cheap in-lock clone of the cache payload; never does file I/O.
fn runtime_ui_cache_snapshot(state: &CoreState) -> Option<(RuntimeUiCache, PathBuf)> {
    let active_profile = state.profiles.active_profile()?;
    Some((
        RuntimeUiCache {
            version: RUNTIME_UI_CACHE_VERSION,
            active_profile: active_profile.to_owned(),
            profile_updated_at: active_profile_revision(&state.profiles, active_profile),
            proxy_groups: state.proxy_groups.clone(),
        },
        runtime_ui_cache_path(&state.profiles),
    ))
}

/// Serialize and atomically replace the runtime UI cache. Lock-free; callers
/// must not hold the core state mutex.
fn persist_runtime_ui_cache_value(
    cache: &RuntimeUiCache,
    path: &std::path::Path,
) -> Result<(), HMetaError> {
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let content = serde_json::to_vec(cache)
        .map_err(|error| HMetaError::Core(format!("serialize runtime UI cache failed: {error}")))?;
    fs::write(&temp_path, content)
        .map_err(|error| HMetaError::Core(format!("write runtime UI cache failed: {error}")))?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        HMetaError::Core(format!("replace runtime UI cache failed: {error}"))
    })
}

pub(super) fn persist_runtime_ui_cache_best_effort(state: &mut CoreState) {
    // The callers hold the core state mutex. Device storage I/O can stall for
    // seconds (observed as a >6s main-thread ANR while a tokio worker blocked
    // on `fs::write` with the lock held), so only clone the payload in-lock
    // and hand serialization + file I/O to the single cache-writer owner.
    if !state.runtime_ui_cache_writes_enabled {
        return;
    }
    let Some((cache, path)) = runtime_ui_cache_snapshot(state) else {
        return;
    };
    RUNTIME_UI_CACHE_WRITER.schedule(cache, path);
}

pub(super) fn platform_vpn_state(state: &CoreState) -> PlatformVpnState {
    PlatformVpnState {
        start_attempt_id: state.platform_start_attempt_id.clone(),
        start_outcome: state.platform_start_outcome,
        extension_attached: state.platform_extension_attached,
        starting: state.platform_vpn_starting,
        running: state.platform_vpn_running,
        network_protected: state.platform_network_protected,
        network_protect_error: state.platform_network_protect_error.clone(),
        updated_at: state.platform_vpn_state_updated_at,
    }
}

pub(super) fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

pub(super) fn current_global_proxy(state: &CoreState) -> Option<String> {
    let profile_id = state.profiles.active_profile()?;
    state
        .profiles
        .selected_proxies(profile_id)
        .ok()?
        .remove("GLOBAL")
}

pub(super) fn platform_ipc_error(error: platform_ipc::PlatformIpcError) -> HMetaError {
    HMetaError::Core(error.to_string())
}

pub(super) fn apply_platform_proxy_selections(state: &mut CoreState, control: &PlatformVpnControl) {
    let current_profile = state.profiles.active_profile();
    if control
        .active_profile
        .as_deref()
        .is_some_and(|profile_id| current_profile != Some(profile_id))
    {
        return;
    }
    let Some(tunnel) = state.tunnel.clone() else {
        return;
    };
    let route = tunnel.route_snapshot();
    let mut changed = false;
    for (group_name, proxy_name) in &control.proxy_selections {
        let Some(group) = route.proxies.get(group_name.as_str()) else {
            continue;
        };
        let Some(selection) = group.selection() else {
            continue;
        };
        if proxy_name.is_empty() {
            if selection.can_unfix() && selection.fixed().as_deref() != Some("") {
                selection.force_set(None);
                changed = true;
            }
            continue;
        }
        if group.current().as_deref() == Some(proxy_name.as_str())
            || !group
                .members()
                .is_some_and(|members| members.iter().any(|member| member == proxy_name))
        {
            continue;
        }
        selection.force_set(Some(proxy_name));
        changed = true;
    }
    drop(route);
    if changed {
        refresh_proxy_groups_preserving_order(state, &tunnel);
        state.logs.push(info_log(
            "proxy selections synchronized from platform control",
        ));
    }
}

pub(super) fn vpn_lifecycle(
    engine_loaded: bool,
    platform_vpn_starting: bool,
    platform_vpn_running: bool,
    native_vpn_running: bool,
    network_protected: bool,
    network_protect_error: Option<&str>,
) -> VpnLifecycle {
    if native_vpn_running || platform_vpn_running {
        if network_protect_error.is_some() && !network_protected {
            VpnLifecycle::ProtectFailed
        } else {
            VpnLifecycle::Connected
        }
    } else if platform_vpn_starting {
        VpnLifecycle::Starting
    } else if network_protect_error.is_some() {
        VpnLifecycle::Failed
    } else if engine_loaded {
        VpnLifecycle::EngineLoaded
    } else {
        VpnLifecycle::Stopped
    }
}
