use super::*;

pub(super) async fn track_controller_mutation(
    AxumState(revision): AxumState<Arc<AtomicU64>>,
    request: Request,
    next: Next,
) -> AxumResponse {
    let mutation = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let response = next.run(request).await;
    if mutation && response.status().is_success() {
        revision.fetch_add(1, Ordering::AcqRel);
    }
    response
}

pub(super) async fn serve_api_controller(
    listener: tokio::net::TcpListener,
    state: Arc<meow_api::routes::AppState>,
    revision: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = meow_api::routes::create_router(state).layer(axum::middleware::from_fn_with_state(
        revision,
        track_controller_mutation,
    ));
    axum::serve(listener, app).await?;
    Ok(())
}

pub(super) async fn monitor_controller_memory(
    addr: SocketAddr,
    secret: Option<String>,
    memory_in_use_bytes: Arc<AtomicU64>,
    memory_limit_bytes: Arc<AtomicU64>,
) {
    loop {
        let mut url = format!("ws://{addr}/memory");
        if let Some(secret) = secret.as_deref().filter(|secret| !secret.is_empty()) {
            if let Ok(mut parsed) = reqwest::Url::parse(&url) {
                parsed.query_pairs_mut().append_pair("token", secret);
                url = parsed.to_string();
            }
        }
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut socket, _)) => {
                while let Some(frame) = socket.next().await {
                    let payload = match frame {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            Some(text.as_bytes().to_vec())
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                            Some(bytes.to_vec())
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                        _ => None,
                    };
                    let Some(payload) = payload else { continue };
                    if let Ok(frame) = serde_json::from_slice::<ControllerMemoryFrame>(&payload) {
                        memory_in_use_bytes.store(frame.inuse, Ordering::Relaxed);
                        memory_limit_bytes.store(frame.oslimit, Ordering::Relaxed);
                    }
                }
            }
            Err(error) => {
                tracing::debug!(%error, "waiting for meow controller memory stream");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

pub fn shared_core() -> Arc<CoreHandle> {
    CoreHandle::shared()
}

pub(super) async fn validate_meow_config(
    raw_yaml: &str,
    store_root: &std::path::Path,
) -> Result<(), PawsError> {
    let validation_yaml =
        paws_profile::sanitize_profile_for_meow_validation_at(raw_yaml, store_root)?;
    let _ = load_meow_config(&validation_yaml).await?;
    Ok(())
}

pub(super) async fn load_meow_config(raw_yaml: &str) -> Result<Config, PawsError> {
    validate_transport_contract(raw_yaml)?;
    // meow's async loader performs YAML decoding and a substantial part of
    // proxy construction before its first await. Keep that CPU-heavy work off
    // the UI/runtime worker that initiated a dashboard bootstrap.
    let raw_yaml = raw_yaml.to_owned();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        runtime.block_on(meow_config::load_config_from_str(&raw_yaml))
    })
    .await
    .map_err(|err| PawsError::Core(format!("meow config worker failed: {err}")))?
    .map_err(|err| PawsError::Core(format!("meow config load failed: {err}")))
}

fn validate_transport_contract(raw_yaml: &str) -> Result<(), PawsError> {
    let document = serde_yaml::from_str::<serde_yaml::Value>(raw_yaml)
        .map_err(|error| PawsError::Core(format!("profile YAML parse failed: {error}")))?;
    let Some(root) = document.as_mapping() else {
        return Ok(());
    };
    let Some(proxies) = yaml_value(root, "proxies").and_then(serde_yaml::Value::as_sequence) else {
        return Ok(());
    };
    for proxy in proxies {
        let Some(proxy) = proxy.as_mapping() else {
            continue;
        };
        let proxy_type = yaml_string(proxy, "type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let name = safe_proxy_name(yaml_string(proxy, "name").unwrap_or("<unnamed>"));
        let network = yaml_string(proxy, "network")
            .unwrap_or("tcp")
            .to_ascii_lowercase();

        if proxy_type == "trojan" {
            if network != "tcp" {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': Trojan network '{network}' is not implemented by the current engine; refusing to ignore it"
                )));
            }
            if yaml_string(proxy, "client-fingerprint").is_some() {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': Trojan client-fingerprint is not implemented by the current engine; refusing to ignore it"
                )));
            }
        }

        if matches!(proxy_type.as_str(), "vless" | "vmess") {
            if yaml_value(proxy, "fingerprint").is_some()
                && yaml_value(proxy, "client-fingerprint").is_none()
            {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': use 'client-fingerprint'; the 'fingerprint' alias is ignored by the current engine"
                )));
            }
            if yaml_value(proxy, "sni").is_some() && yaml_value(proxy, "servername").is_none() {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': use 'servername'; the 'sni' alias is ignored for {proxy_type} by the current engine"
                )));
            }
            if let Some(fingerprint) = yaml_string(proxy, "client-fingerprint") {
                let fingerprint = fingerprint.to_ascii_lowercase();
                if !matches!(
                    fingerprint.as_str(),
                    "chrome"
                        | "chrome120"
                        | "firefox"
                        | "firefox120"
                        | "safari"
                        | "safari16"
                        | "ios"
                        | "android"
                        | "edge"
                        | "random"
                ) {
                    return Err(PawsError::Core(format!(
                        "proxy '{name}': unsupported TLS client-fingerprint '{fingerprint}'"
                    )));
                }
            }
        }

        if matches!(proxy_type.as_str(), "vless" | "vmess") && network == "ws" {
            validate_websocket_contract(proxy, &name)?;
        }
    }
    Ok(())
}

fn validate_websocket_contract(
    proxy: &serde_yaml::Mapping,
    name: &str,
) -> Result<(), PawsError> {
    if let Some(alpn) = yaml_value(proxy, "alpn") {
        let valid = match alpn {
            serde_yaml::Value::Sequence(values) => {
                !values.is_empty()
                    && values.iter().all(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| value.eq_ignore_ascii_case("http/1.1"))
                    })
            }
            serde_yaml::Value::String(value) => value.eq_ignore_ascii_case("http/1.1"),
            _ => false,
        };
        if !valid {
            return Err(PawsError::Core(format!(
                "proxy '{name}': WebSocket ALPN must be exactly http/1.1; advertising h2 can negotiate a protocol this engine cannot speak"
            )));
        }
    }

    let headers = yaml_value(proxy, "ws-opts")
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|options| yaml_value(options, "headers"))
        .and_then(serde_yaml::Value::as_mapping);
    if let Some(headers) = headers {
        for (key, value) in headers {
            let Some(key) = key.as_str() else {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': WebSocket header names must be strings"
                )));
            };
            if key.eq_ignore_ascii_case("host") && key != "Host" {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': WebSocket host header must use the exact key 'Host' with the current engine"
                )));
            }
            if key == "Host" && value.as_str().is_none() {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': WebSocket Host header must be a string"
                )));
            }
            if !key.eq_ignore_ascii_case("host") {
                return Err(PawsError::Core(format!(
                    "proxy '{name}': WebSocket header '{key}' is not supported by the current engine; refusing to drop it"
                )));
            }
        }
    }
    Ok(())
}

fn yaml_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    name: &str,
) -> Option<&'a serde_yaml::Value> {
    mapping
        .iter()
        .find_map(|(key, value)| (key.as_str() == Some(name)).then_some(value))
}

fn yaml_string<'a>(mapping: &'a serde_yaml::Mapping, name: &str) -> Option<&'a str> {
    yaml_value(mapping, name).and_then(serde_yaml::Value::as_str)
}

fn safe_proxy_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

pub(super) fn tunnel_from_config(config: Config, mode: RuntimeMode) -> Tunnel {
    let tunnel = Tunnel::new(config.dns.resolver.clone());
    tunnel.update_rules(config.rules);
    tunnel.update_proxies(config.proxies);
    tunnel.set_mode(mode_to_tunnel(mode));
    tunnel.spawn_background_tasks();
    tunnel
}

pub(super) fn raw_configs_equal(left: &RawConfig, right: &RawConfig) -> Result<bool, PawsError> {
    let left = serde_yaml::to_value(left)
        .map_err(|error| PawsError::Core(format!("cannot inspect controller config: {error}")))?;
    let right = serde_yaml::to_value(right)
        .map_err(|error| PawsError::Core(format!("cannot inspect controller config: {error}")))?;
    Ok(left == right)
}

pub(super) fn merge_external_raw_config(
    profile_yaml: &str,
    baseline: &RawConfig,
    current: &RawConfig,
) -> Result<String, PawsError> {
    let mut profile = serde_yaml::from_str::<serde_yaml::Value>(profile_yaml)
        .map_err(|error| PawsError::Core(format!("profile YAML parse failed: {error}")))?;
    let profile = profile
        .as_mapping_mut()
        .ok_or_else(|| PawsError::Core("profile YAML root must be a mapping".to_owned()))?;
    let baseline = serde_yaml::to_value(baseline).map_err(|error| {
        PawsError::Core(format!("cannot serialize controller config: {error}"))
    })?;
    let current = serde_yaml::to_value(current).map_err(|error| {
        PawsError::Core(format!("cannot serialize controller config: {error}"))
    })?;
    let baseline = baseline
        .as_mapping()
        .ok_or_else(|| PawsError::Core("controller baseline is not a mapping".to_owned()))?;
    let current = current
        .as_mapping()
        .ok_or_else(|| PawsError::Core("controller config is not a mapping".to_owned()))?;

    let mut keys = baseline
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    keys.dedup();
    for key in keys {
        let Some(name) = key.as_str() else { continue };
        if controller_runtime_only_key(name) || baseline.get(&key) == current.get(&key) {
            continue;
        }
        match current.get(&key) {
            Some(value) if !value.is_null() => {
                profile.insert(key, value.clone());
            }
            _ => {
                profile.remove(&key);
            }
        }
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(profile.clone()))
        .map_err(|error| PawsError::Core(format!("profile YAML serialization failed: {error}")))
}

pub(super) fn controller_runtime_only_key(key: &str) -> bool {
    matches!(
        key,
        "port"
            | "socks-port"
            | "mixed-port"
            | "allow-lan"
            | "bind-address"
            | "mode"
            | "log-level"
            | "external-controller"
            | "external-ui"
            | "external-ui-name"
            | "external-ui-url"
            | "secret"
            | "tproxy-port"
            | "tproxy-sni"
            | "routing-mark"
            | "listeners"
            | "authentication"
            | "skip-auth-prefixes"
            | "max-connections"
    )
}

pub(super) fn sync_live_controller_route(state: &mut CoreState) -> Result<(), PawsError> {
    let Some(tunnel) = state.tunnel.clone() else {
        return Ok(());
    };
    state.mode = mode_from_tunnel(tunnel.mode());
    refresh_proxy_groups_preserving_order(state, &tunnel);
    let Some(profile_id) = state.profiles.active_profile().map(ToOwned::to_owned) else {
        return Ok(());
    };
    let persisted = state.profiles.selected_proxies(&profile_id)?;
    let selections = state
        .proxy_groups
        .iter()
        .filter_map(|group| {
            let selected = match group.fixed.as_deref() {
                Some(fixed) => fixed.to_owned(),
                None => group.selected.clone()?,
            };
            Some((group.name.clone(), selected))
        })
        .collect::<Vec<_>>();
    for (group, selected) in selections {
        if persisted.get(&group) != Some(&selected) {
            state
                .profiles
                .set_selected_proxy(&profile_id, group, selected)?;
        }
    }
    Ok(())
}

pub(super) fn restore_proxy_selections(
    tunnel: &Tunnel,
    selected_proxies: &std::collections::BTreeMap<String, String>,
) {
    if selected_proxies.is_empty() {
        return;
    }
    let route = tunnel.route_snapshot();
    let proxies = &route.proxies;
    for (group_name, proxy_name) in selected_proxies {
        let Some(group) = proxies.get(group_name.as_str()) else {
            continue;
        };
        let Some(selection) = group.selection() else {
            continue;
        };
        if proxy_name.is_empty() && selection.can_unfix() {
            selection.force_set(None);
        } else if group
            .members()
            .is_some_and(|members| members.iter().any(|member| member == proxy_name))
        {
            selection.force_set(Some(proxy_name));
        }
    }
}

pub(super) fn runtime_rule_summaries(
    profile_id: &str,
    loaded_lines: &[String],
    editable_rules: &[paws_model::RuleSummary],
) -> Vec<paws_model::RuleSummary> {
    let mut consumed_editable = vec![false; editable_rules.len()];
    let mut summaries = Vec::with_capacity(loaded_lines.len() + editable_rules.len());
    for (index, line) in loaded_lines.iter().enumerate() {
        if let Some((editable_index, editable)) =
            editable_rules
                .iter()
                .enumerate()
                .find(|(editable_index, editable)| {
                    !consumed_editable[*editable_index]
                        && editable.enabled
                        && editable.line.trim() == line.trim()
                })
        {
            consumed_editable[editable_index] = true;
            let mut editable = editable.clone();
            editable.order = index as u32;
            summaries.push(editable);
        } else {
            summaries.push(paws_model::RuleSummary {
                id: format!("runtime-rule-{index}"),
                profile_id: profile_id.to_owned(),
                line: line.clone(),
                enabled: true,
                order: index as u32,
                source: "profile-yaml".to_owned(),
            });
        }
    }
    for (editable_index, editable) in editable_rules.iter().enumerate() {
        if !consumed_editable[editable_index] {
            let mut editable = editable.clone();
            editable.order = summaries.len() as u32;
            summaries.push(editable);
        }
    }
    summaries
}

pub(super) fn rule_lookup_metadata(
    query: &str,
) -> Result<(String, RuleLookupInputKind, Metadata), PawsError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(PawsError::Core(
            "enter a domain name or IP address".to_owned(),
        ));
    }

    let ip_candidate = query
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(query);
    let mut metadata = Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Http,
        dst_port: 443,
        ..Metadata::default()
    };
    if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
        metadata.dst_ip = Some(ip);
        return Ok((ip.to_string(), RuleLookupInputKind::Ip, metadata));
    }

    let domain = query.trim_end_matches('.').to_lowercase();
    let invalid_character = domain.chars().any(|character| {
        !(character.is_alphanumeric() || character == '-' || character == '_') && character != '.'
    });
    let invalid_label = domain.split('.').any(|label| {
        label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
    });
    if domain.is_empty()
        || domain.len() > 253
        || invalid_character
        || invalid_label
        || query.contains("://")
    {
        return Err(PawsError::Core(
            "enter a valid domain name or IP address".to_owned(),
        ));
    }
    metadata.host = Metadata::lower_host(&domain);
    Ok((domain, RuleLookupInputKind::Domain, metadata))
}
