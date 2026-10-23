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

pub(super) async fn validate_meow_capability(
    runtime_yaml: &str,
    runtime_path: &Path,
) -> Result<Vec<String>, PawsError> {
    let config =
        load_meow_config_from_path(runtime_yaml, runtime_path, MeowLoadPurpose::Capability).await?;
    let mut unverified = config
        .proxy_providers
        .values()
        .filter(|provider| provider.updated_at_secs() == 0)
        .map(|provider| provider.name.clone())
        .collect::<Vec<_>>();
    unverified.extend(
        config
            .rule_providers
            .iter()
            .filter(|(_, provider)| {
                provider.updated_at_secs() == 0
                    && !provider
                        .provider_type
                        .to_string()
                        .eq_ignore_ascii_case("inline")
            })
            .map(|(name, _)| name.clone()),
    );
    unverified.sort();
    unverified.dedup();
    Ok(unverified)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MeowLoadPurpose {
    Preview,
    Capability,
    Runtime,
}

#[cfg(test)]
pub(super) async fn load_meow_config(raw_yaml: &str) -> Result<Config, PawsError> {
    load_meow_config_for(raw_yaml, MeowLoadPurpose::Runtime).await
}
pub(super) async fn load_meow_config_for(
    raw_yaml: &str,
    purpose: MeowLoadPurpose,
) -> Result<Config, PawsError> {
    MeowConfigLoader {
        raw_yaml,
        path: None,
        purpose,
    }
    .load()
    .await
}

pub(super) async fn load_meow_config_from_path(
    raw_yaml: &str,
    path: &Path,
    purpose: MeowLoadPurpose,
) -> Result<Config, PawsError> {
    MeowConfigLoader {
        raw_yaml,
        path: Some(path),
        purpose,
    }
    .load()
    .await
}

struct MeowConfigLoader<'a> {
    raw_yaml: &'a str,
    path: Option<&'a Path>,
    purpose: MeowLoadPurpose,
}

fn validate_read_only_meow_construction(
    raw: &meow_config::raw::RawConfig,
) -> Result<(), PawsError> {
    if raw
        .proxies
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(meow_config::proxy_parser::node_selects_external_plugin)
    {
        return Err(PawsError::Core(
            "read-only Meow preview cannot construct an external SIP003 plugin".into(),
        ));
    }
    if raw.proxy_providers.as_ref().is_some_and(|providers| {
        providers
            .values()
            .any(|provider| provider.allow_external_plugin == Some(true))
    }) {
        return Err(PawsError::Core(
            "read-only Meow preview cannot safely validate Provider caches that allow external plugins".into()
        ));
    }
    Ok(())
}

impl MeowConfigLoader<'_> {
    async fn load(self) -> Result<Config, PawsError> {
        let raw_yaml = self.raw_yaml;
        let started = Instant::now();
        let result = self.load_inner().await;
        if let Err(error) = &result {
            // Validation before the start transaction also uses this loader.
            // Record at this boundary so a rejected configuration does not
            // depend on reaching reload_config's later error handler.
            push_runtime_log(
                &RUNTIME_LOGS,
                error_log(format!(
                    "configuration load rejected (meow {MEOW_RS_VERSION}, {} ms): {}",
                    started.elapsed().as_millis(),
                    config_error_message(raw_yaml, &error.to_string())
                )),
            );
        }
        result
    }

    async fn load_inner(self) -> Result<Config, PawsError> {
        if self.purpose != MeowLoadPurpose::Preview {
            validate_transport_contract(self.raw_yaml)?;
        }
        // The engine's public API accepts UTF-8 paths. Never silently change
        // the identity of a file containing non-UTF-8 bytes.
        let path = self
            .path
            .map(|path| {
                path.to_str().map(str::to_owned).ok_or_else(|| {
                    PawsError::Core("meow config path is not valid UTF-8".to_owned())
                })
            })
            .transpose()?;
        let raw_yaml = self.raw_yaml.to_owned();
        let options = meow_config::ConfigLoadOptions {
            provider_network: match self.purpose {
                MeowLoadPurpose::Preview | MeowLoadPurpose::Capability => {
                    meow_config::ProviderNetworkPolicy::CacheOnly
                }
                MeowLoadPurpose::Runtime => meow_config::ProviderNetworkPolicy::Live,
            },
        };
        let strict = self.purpose != MeowLoadPurpose::Preview;
        let read_only = self.purpose != MeowLoadPurpose::Runtime;
        let runtime = tokio::runtime::Handle::current();
        // YAML decoding/proxy construction must stay off the UI worker.
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async {
                let mut raw = meow_config::parse_raw_yaml(&raw_yaml)?;
                if read_only {
                    // Cache-only network authority does not prevent a proxy
                    // constructor from launching an executable. Keep that
                    // separate authority out of both read-only purposes.
                    validate_read_only_meow_construction(&raw)?;
                }
                if strict {
                    raw.strict = Some(true);
                }
                let cache = path
                    .as_deref()
                    .map(meow_config::resource_cache_dir_for_config_path);
                meow_config::build_config_with_options(raw, cache.as_deref(), options).await
            })
        })
        .await
        .map_err(|err| PawsError::Core(format!("meow config worker failed: {err}")))?
        .map_err(|err| {
            PawsError::Core(format!(
                "meow config load failed: {}",
                config_error_message(self.raw_yaml, &format!("{err:#}"))
            ))
        })
    }
}

// Preserve the engine's cause chain without copying credentials or signed
// provider URLs into the UI, platform failure state, or exported log archive.
pub(super) fn config_error_message(raw_yaml: &str, message: &str) -> String {
    fn collect(value: &serde_yaml::Value, replacements: &mut Vec<(String, String)>) {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for (key, value) in map {
                    if let (Some(key), Some(secret)) = (key.as_str(), value.as_str()) {
                        if matches!(
                            key.to_ascii_lowercase().as_str(),
                            "password"
                                | "uuid"
                                | "psk"
                                | "private-key"
                                | "public-key"
                                | "short-id"
                                | "username"
                                | "secret"
                                | "auth"
                                | "auth-str"
                                | "token"
                                | "authorization"
                        ) && !secret.is_empty()
                        {
                            replacements.push((secret.to_owned(), "[redacted]".to_owned()));
                        } else if key.eq_ignore_ascii_case("url") {
                            if let Ok(url) = reqwest::Url::parse(secret) {
                                replacements.push((
                                    secret.to_owned(),
                                    format!(
                                        "{}://{}/[redacted]",
                                        url.scheme(),
                                        url.host_str().unwrap_or("provider")
                                    ),
                                ));
                            }
                        }
                    }
                    collect(value, replacements);
                }
            }
            serde_yaml::Value::Sequence(values) => {
                for value in values {
                    collect(value, replacements);
                }
            }
            _ => {}
        }
    }
    let mut replacements = Vec::new();
    if let Ok(value) = serde_yaml::from_str(raw_yaml) {
        collect(&value, &mut replacements);
    }
    replacements.sort_by_key(|(secret, _)| std::cmp::Reverse(secret.len()));
    replacements
        .into_iter()
        .fold(message.to_owned(), |message, (secret, replacement)| {
            message.replace(&secret, &replacement)
        })
}

pub(super) fn validate_transport_contract(raw_yaml: &str) -> Result<(), PawsError> {
    let document = serde_yaml::from_str::<serde_yaml::Value>(raw_yaml)
        .map_err(|error| PawsError::Core(format!("profile YAML parse failed: {error}")))?;
    let Some(root) = document.as_mapping() else {
        return Ok(());
    };
    if yaml_value(root, "dns")
        .and_then(serde_yaml::Value::as_mapping)
        .is_some_and(|dns| yaml_value(dns, "fake-ip-range6").is_some_and(|value| !value.is_null()))
    {
        return Err(PawsError::Core(
            "Meow does not implement an IPv6 fake-IP pool; refusing to ignore fake-ip-range6"
                .into(),
        ));
    }
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

fn validate_websocket_contract(proxy: &serde_yaml::Mapping, name: &str) -> Result<(), PawsError> {
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

fn yaml_value<'a>(mapping: &'a serde_yaml::Mapping, name: &str) -> Option<&'a serde_yaml::Value> {
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

#[cfg(test)]
pub(super) fn tunnel_from_config(config: Config, mode: RuntimeMode) -> Tunnel {
    tunnel_from_config_for(config, mode, MeowLoadPurpose::Runtime)
}
pub(super) fn tunnel_from_config_for(
    config: Config,
    mode: RuntimeMode,
    purpose: MeowLoadPurpose,
) -> Tunnel {
    let tunnel = Tunnel::new_with_slot(config.dns.resolver_slot.clone());
    tunnel.set_dialer_registry(config.provider_dialer_registry);
    tunnel.update_routing(config.proxies, config.rules, config.dialer_registry);
    tunnel.set_mode(mode_to_tunnel(mode));
    if purpose == MeowLoadPurpose::Runtime {
        tunnel.spawn_background_tasks();
        tunnel.reconcile_health_checks(&meow_config::extract_health_check_specs(
            config.raw.proxy_groups.as_deref().unwrap_or(&[]),
        ));
    }
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
    let baseline = serde_yaml::to_value(baseline)
        .map_err(|error| PawsError::Core(format!("cannot serialize controller config: {error}")))?;
    let current = serde_yaml::to_value(current)
        .map_err(|error| PawsError::Core(format!("cannot serialize controller config: {error}")))?;
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
                // RawConfig's typed Option fields serialize as null. They
                // mean absent settings, whereas an explicit invalid null in
                // imported user YAML must still fail strict field checks.
                let mut value = value.clone();
                omit_absent_typed_fields(&mut value);
                profile.insert(key, value);
            }
            _ => {
                profile.remove(&key);
            }
        }
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(profile.clone()))
        .map_err(|error| PawsError::Core(format!("profile YAML serialization failed: {error}")))
}

fn omit_absent_typed_fields(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            map.retain(|_, value| !value.is_null());
            for value in map.values_mut() {
                omit_absent_typed_fields(value);
            }
        }
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                omit_absent_typed_fields(value);
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn typed_controller_merge_omits_absent_options_without_loosening_profile_source_validation() {
        let baseline =
            meow_config::parse_raw_yaml("dns: {nameserver: [1.1.1.1]}\nrules: ['MATCH,DIRECT']\n")
                .unwrap();
        let current =
            meow_config::parse_raw_yaml("dns: {nameserver-policy: {}}\nrules: ['MATCH,DIRECT']\n")
                .unwrap();
        let result = merge_external_raw_config(
            "dns: {nameserver: [1.1.1.1]}\nrules: ['MATCH,DIRECT']\n",
            &baseline,
            &current,
        )
        .unwrap();
        let result: serde_yaml::Value = serde_yaml::from_str(&result).unwrap();
        let dns = result.get("dns").unwrap().as_mapping().unwrap();
        assert!(!dns.contains_key(serde_yaml::Value::String("nameserver".into())));
        assert!(dns
            .get(serde_yaml::Value::String("nameserver-policy".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .is_empty());
        assert!(dns.values().all(|value| !value.is_null()));
    }
    #[tokio::test]
    async fn read_only_loader_rejects_external_constructors_before_static_or_cache_loading() {
        for source in [
            "proxies: [{name: static, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: fixture, plugin: nonexistent-fixture-executable}]\nrules: ['MATCH,DIRECT']\n",
            "proxy-providers: {cached: {type: file, path: /nonexistent/synthetic-provider.yaml, allow-external-plugin: true}}\nrules: ['MATCH,DIRECT']\n",
        ] {
            for purpose in [MeowLoadPurpose::Preview, MeowLoadPurpose::Capability] {
                let error = match load_meow_config_for(source, purpose).await {
                    Ok(_) => panic!("read-only load accepted external constructor authority"),
                    Err(error) => error.to_string(),
                };
                assert!(error.contains("read-only Meow preview"), "{error}");
                assert!(!error.contains("failed to start ss plugin"), "guard must run before constructors");
                assert!(!error.contains("nonexistent-fixture-executable"));
            }
        }
    }
    /// This fixture answers one uncompressed IN question. EDNS is additional
    /// data, so neither its tail nor the original packet length locates QTYPE.
    fn bootstrap_dns_answer(query: &[u8]) -> Option<Vec<u8>> {
        if query.len() < 12 || query[4..6] != [0, 1] {
            return None;
        }
        let mut cursor = 12;
        loop {
            let label = *query.get(cursor)? as usize;
            cursor += 1;
            if label == 0 {
                break;
            }
            if label > 63 {
                return None;
            }
            cursor = cursor.checked_add(label)?;
            if cursor > query.len() {
                return None;
            }
        }
        let kind = u16::from_be_bytes([*query.get(cursor)?, *query.get(cursor + 1)?]);
        if query.get(cursor + 2..cursor + 4)? != [0, 1] {
            return None;
        }
        let mut reply = query[..cursor + 4].to_vec();
        reply[2] = 0x81;
        reply[3] = 0x80;
        reply[6] = 0;
        reply[7] = u8::from(kind == 1);
        reply[8..12].fill(0);
        if kind == 1 {
            reply.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 127, 0, 0, 1]);
        }
        Some(reply)
    }

    #[test]
    fn bootstrap_fixture_handles_edns_after_the_actual_question() {
        let mut query = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 1];
        for label in ["proxy-bootstrap", "synthetic", "test"] {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        let question_end = query.len();
        query.extend_from_slice(&[0, 0, 41, 16, 0, 0, 0, 0, 0, 0, 0]); // EDNS OPT
        let reply = bootstrap_dns_answer(&query).unwrap();
        assert_eq!(&reply[..2], &[0x12, 0x34]);
        assert_eq!(&reply[6..12], &[0, 1, 0, 0, 0, 0]);
        assert_eq!(&reply[12..question_end], &query[12..question_end]);
        assert_eq!(
            &reply[question_end..],
            &[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 127, 0, 0, 1]
        );
        query[question_end - 3] = 28; // AAAA receives a valid empty answer.
        let reply = bootstrap_dns_answer(&query).unwrap();
        assert_eq!(reply.len(), question_end);
        assert_eq!(&reply[6..12], &[0; 6]);
    }

    #[tokio::test]
    async fn cache_only_preview_has_no_network_and_real_meow_prepare_rehydrates_live() {
        use axum::{extract::State, routing::get, Router};
        #[derive(Clone)]
        struct Requests {
            providers: Arc<AtomicU64>,
            health: Arc<AtomicU64>,
        }
        let requests = Requests {
            providers: Arc::default(),
            health: Arc::default(),
        };
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/provider",
                get(|State(requests): State<Requests>| async move {
                    requests.providers.fetch_add(1, Ordering::SeqCst);
                    "proxies:\n  - name: cached-node\n    type: socks5\n    server: 127.0.0.1\n    port: 9\n"
                }),
            )
            .route(
                "/health",
                get(|State(requests): State<Requests>| async move {
                    requests.health.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }),
            )
            .with_state(requests.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dns = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let dns_address = dns.local_addr().unwrap();
        let dns_queries = Arc::new(AtomicU64::new(0));
        let queries = dns_queries.clone();
        let dns_server = tokio::spawn(async move {
            let mut buffer = [0u8; 2048];
            while let Ok((length, peer)) = dns.recv_from(&mut buffer).await {
                queries.fetch_add(1, Ordering::SeqCst);
                let Some(reply) = bootstrap_dns_answer(&buffer[..length]) else {
                    continue;
                };
                let _ = dns.send_to(&reply, peer).await;
            }
        });
        let root = std::env::temp_dir().join(format!("paws-preview-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        let yaml=format!("dns:\n  enable: true\n  nameserver: [https://resolver.synthetic.test/dns-query]\n  default-nameserver: [udp://{dns_address}]\n  proxy-server-nameserver: [https://proxy-bootstrap.synthetic.test/dns-query]\n  nameserver-policy:\n    '+.synthetic.test': [https://policy.synthetic.test/dns-query]\nproxy-providers:\n  remote:\n    type: http\n    url: http://{address}/provider\n    path: ./remote.yaml\n    interval: 86400\nproxy-groups:\n  - name: auto\n    type: url-test\n    proxies: [DIRECT]\n    use: [remote]\n    url: http://{address}/health\n    interval: 300\n    lazy: false\nrules:\n  - MATCH,auto\n");
        // The production ProfileStore intentionally replaces bootstrap DNS
        // with the app's managed physical bootstrap list. Exercise the loader
        // permission boundary directly with an isolated, explicit local DNS
        // file instead of assuming Core retains this fixture's bootstrap URL.
        let bootstrap_dir = root.join("bootstrap-fixture");
        fs::create_dir_all(&bootstrap_dir).unwrap();
        let bootstrap_path = bootstrap_dir.join("config.yaml");
        fs::write(&bootstrap_path, &yaml).unwrap();
        drop(
            load_meow_config_from_path(&yaml, &bootstrap_path, MeowLoadPurpose::Preview)
                .await
                .unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(requests.providers.load(Ordering::SeqCst), 0);
        assert_eq!(requests.health.load(Ordering::SeqCst), 0);
        assert_eq!(
            dns_queries.load(Ordering::SeqCst),
            0,
            "Preview must not bootstrap main/proxy/policy hostname resolvers"
        );
        drop(
            load_meow_config_from_path(&yaml, &bootstrap_path, MeowLoadPurpose::Runtime)
                .await
                .unwrap(),
        );
        assert!(
            dns_queries.load(Ordering::SeqCst) > 0,
            "Runtime loader must really query its explicit local bootstrap"
        );
        assert!(
            requests.providers.load(Ordering::SeqCst) > 0,
            "Runtime loader must really download its HTTP provider"
        );
        requests.providers.store(0, Ordering::SeqCst);
        requests.health.store(0, Ordering::SeqCst);
        dns_queries.store(0, Ordering::SeqCst);
        // Core's separate promotion scenario has only literal resolver hosts;
        // no test bootstrap can accidentally escape to a managed public DNS.
        let yaml = yaml
            .replace("resolver.synthetic.test", "127.0.0.1")
            .replace("proxy-bootstrap.synthetic.test", "127.0.0.1")
            .replace("policy.synthetic.test", "127.0.0.1");
        let profile = {
            let mut state = core.lock_state().unwrap();
            let profile = state
                .profiles
                .import_profile_content("synthetic", "test", &yaml, None)
                .unwrap();
            state.profiles.set_active(&profile).unwrap();
            profile
        };
        core.reload_config_for_purpose(&profile, MeowLoadPurpose::Preview)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(requests.providers.load(Ordering::SeqCst), 0);
        assert_eq!(requests.health.load(Ordering::SeqCst), 0);
        assert_eq!(
            dns_queries.load(Ordering::SeqCst),
            0,
            "Preview must not bootstrap nameserver/proxy/policy hostnames through default DNS"
        );
        assert!(!core.lock_state().unwrap().meow_runtime_ready);
        assert!(core.prepare_active_vpn_inner(false, true).await.unwrap());
        tokio::time::timeout(Duration::from_secs(3), async {
            while requests.providers.load(Ordering::SeqCst) == 0
                || requests.health.load(Ordering::SeqCst) == 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(core.lock_state().unwrap().meow_runtime_ready);
        server.abort();
        dns_server.abort();
        drop(core);
        let _ = fs::remove_dir_all(root);
    }
}
