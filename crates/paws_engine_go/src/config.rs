//! Concrete configuration compilation belongs to the engine adapter.
mod dns;
#[cfg(any(target_env = "ohos", test))]
pub(crate) mod geo;
#[cfg(any(target_env = "ohos", test))]
mod geosite;
#[cfg(any(target_env = "ohos", test))]
pub(crate) mod provider;
mod provider_definition;
mod rule;
use paws_engine::{EngineError, EngineNetworkServices};
use paws_model::{RuntimeMode, VpnOptions};
pub use provider_definition::validate_sing_box_provider_definitions;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
#[path = "sing_box_proxy.rs"]
mod sing_box_proxy;
pub fn render_mihomo_runtime_yaml(
    runtime_yaml: &str,
    services: &EngineNetworkServices,
) -> Result<String, EngineError> {
    services.validate()?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(runtime_yaml)
        .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
    let root = value
        .as_mapping_mut()
        .ok_or_else(|| EngineError::InvalidConfig("profile root must be a YAML map".to_owned()))?;
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    for name in [
        "paws",
        "geodata",
        "external-controller-tls",
        "external-controller-unix",
        "external-controller-pipe",
        "secret",
    ] {
        root.remove(key(name));
    }
    root.insert(
        key("external-controller"),
        serde_yaml::Value::String("127.0.0.1:19090".to_owned()),
    );
    root.insert(
        key("allow-lan"),
        serde_yaml::Value::Bool(services.access.allow_lan),
    );
    root.insert(
        key("bind-address"),
        serde_yaml::Value::String(
            if services.access.allow_lan {
                "*"
            } else {
                "127.0.0.1"
            }
            .into(),
        ),
    );
    root.insert(
        key("mixed-port"),
        serde_yaml::Value::Number(
            if services.ports.mixed_enabled {
                services.ports.mixed_port
            } else {
                0
            }
            .into(),
        ),
    );
    root.insert(
        key("paws-runtime"),
        serde_yaml::to_value(services)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?,
    );
    let listeners = root
        .entry(key("listeners"))
        .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
    if let Some(list) = listeners.as_sequence_mut() {
        list.push(
            serde_yaml::from_str(
                "name: tinypaws-mihomo-benchmark\ntype: mixed\nport: 18490\nlisten: 127.0.0.1\n",
            )
            .map_err(|e| EngineError::InvalidConfig(e.to_string()))?,
        );
    }
    // TinyPaws owns the packaged geodata lifecycle. Letting mihomo replace a
    // live memory-mapped MMDB in the background can invalidate readers in the
    // VPN process; refreshes are installed atomically before a later reload.
    root.insert(key("geo-auto-update"), serde_yaml::Value::Bool(false));
    root.remove(key("geo-update-interval"));
    repair_mihomo_dns_detours(root);
    let runtime_mode = runtime_mode_from_mapping(root);
    harden_runtime_dns_mapping(root, runtime_mode, &BTreeMap::new(), false);
    let tun = root
        .entry(key("tun"))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    let tun = tun.as_mapping_mut().ok_or_else(|| {
        EngineError::InvalidConfig("profile field 'tun' must be a mapping".to_owned())
    })?;
    tun.insert(key("enable"), serde_yaml::Value::Bool(false));
    tun.insert(key("stack"), serde_yaml::Value::String("mips".to_owned()));
    tun.insert(key("auto-route"), serde_yaml::Value::Bool(false));
    serde_yaml::to_string(&value).map_err(|error| EngineError::InvalidConfig(error.to_string()))
}

fn repair_mihomo_dns_detours(root: &mut serde_yaml::Mapping) {
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    let route_names = declared_route_names(root);
    let Some(dns) = root
        .get_mut(key("dns"))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return;
    };
    let Some(policy) = dns
        .get_mut(key("nameserver-policy"))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return;
    };
    for servers in policy
        .values_mut()
        .filter_map(serde_yaml::Value::as_sequence_mut)
    {
        let detour = servers
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .find_map(|candidate| {
                let candidate = candidate.trim();
                if candidate.contains("://")
                    || candidate.contains('#')
                    || candidate.contains('.')
                    || candidate.parse::<IpAddr>().is_ok()
                {
                    return None;
                }
                route_names
                    .iter()
                    .find(|name| {
                        name.as_str() == candidate
                            || name
                                .trim_start_matches(|character: char| !character.is_alphanumeric())
                                .trim()
                                == candidate
                    })
                    .cloned()
            });
        let Some(detour) = detour else {
            continue;
        };
        if !servers
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .any(|server| {
                server.contains("://") || server.parse::<IpAddr>().is_ok() || server.contains('.')
            })
        {
            continue;
        }
        servers.retain(|server| {
            server.as_str().is_none_or(|server| {
                let server = server.trim();
                server != detour
                    && route_names.iter().all(|name| {
                        name.as_str() != server
                            && name
                                .trim_start_matches(|character: char| !character.is_alphanumeric())
                                .trim()
                                != server
                    })
            })
        });
        for server in servers.iter_mut() {
            let Some(raw) = server.as_str() else {
                continue;
            };
            let raw = raw.trim();
            if !(raw.contains("://") || raw.parse::<IpAddr>().is_ok() || raw.contains('.')) {
                continue;
            }
            // An explicit DNS detour is authoritative. Only translate the
            // shared profile shorthand `[server, group]`; never replace the
            // requested group with generic rule routing or rewrite bootstrap.
            if !raw.contains('#') && !raw.is_empty() {
                *server = serde_yaml::Value::String(format!("{raw}#{detour}"));
            }
        }
    }
}

pub fn harden_runtime_dns_yaml(
    runtime_yaml: &str,
    mode: Option<RuntimeMode>,
    selections: &BTreeMap<String, String>,
    plain_tcp_only_detour: bool,
) -> Result<String, EngineError> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(runtime_yaml)
        .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
    let Some(root) = value.as_mapping_mut() else {
        return Ok(runtime_yaml.to_owned());
    };
    repair_mihomo_dns_detours(root);
    let resolved_mode = mode.unwrap_or_else(|| runtime_mode_from_mapping(root));
    harden_runtime_dns_mapping(root, resolved_mode, selections, plain_tcp_only_detour);
    serde_yaml::to_string(&value).map_err(|error| EngineError::InvalidConfig(error.to_string()))
}

fn runtime_mode_from_mapping(root: &serde_yaml::Mapping) -> RuntimeMode {
    match yaml_string(root, "mode")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "direct" => RuntimeMode::Direct,
        "global" => RuntimeMode::Global,
        _ => RuntimeMode::Rule,
    }
}

fn declared_field_names(root: &serde_yaml::Mapping, field: &str) -> Vec<String> {
    yaml_field(root, field)
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(serde_yaml::Value::as_mapping)
        .filter_map(|item| yaml_field(item, "name").and_then(serde_yaml::Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn declared_route_names(root: &serde_yaml::Mapping) -> Vec<String> {
    let mut names = declared_field_names(root, "proxy-groups");
    names.extend(declared_field_names(root, "proxies"));
    names
}

fn resolve_declared_route(route_names: &[String], candidate: &str) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || matches!(
            candidate.to_ascii_uppercase().as_str(),
            "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
        )
    {
        return None;
    }
    route_names
        .iter()
        .find(|name| {
            name.as_str() == candidate
                || name
                    .trim_start_matches(|character: char| !character.is_alphanumeric())
                    .trim()
                    == candidate
        })
        .cloned()
}

fn preferred_dns_detour(
    root: &serde_yaml::Mapping,
    mode: RuntimeMode,
    selections: &BTreeMap<String, String>,
) -> Option<String> {
    if mode == RuntimeMode::Direct {
        return None;
    }
    let group_names = declared_field_names(root, "proxy-groups");
    let proxy_names = declared_field_names(root, "proxies");
    if group_names.is_empty() && proxy_names.is_empty() {
        return None;
    }
    let mut all_routes = group_names.clone();
    all_routes.extend(proxy_names.clone());
    if mode == RuntimeMode::Global {
        if let Some(selected) = selections
            .get("GLOBAL")
            .and_then(|target| resolve_declared_route(&all_routes, target))
        {
            return Some(selected);
        }
    } else if let Some(rules) = yaml_field(root, "rules").and_then(serde_yaml::Value::as_sequence) {
        for entry in rules.iter().rev().filter_map(serde_yaml::Value::as_str) {
            let mut parts = entry.split(',');
            let Some(kind) = parts.next() else {
                continue;
            };
            if matches!(kind.trim().to_ascii_uppercase().as_str(), "MATCH" | "FINAL") {
                if let Some(target) = parts
                    .next()
                    .and_then(|target| resolve_declared_route(&all_routes, target))
                {
                    return Some(target);
                }
            }
        }
    }
    group_names
        .into_iter()
        .next()
        .or_else(|| proxy_names.into_iter().next())
}

fn poisoned_foreign_dns_canonical_ip(host: &str) -> Option<&'static str> {
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
        .as_str()
    {
        "1.1.1.1" | "cloudflare-dns.com" | "one.one.one.one" => Some("1.1.1.1"),
        "1.0.0.1" => Some("1.0.0.1"),
        "8.8.8.8" | "dns.google" => Some("8.8.8.8"),
        "8.8.4.4" => Some("8.8.4.4"),
        "9.9.9.9" | "dns.quad9.net" => Some("9.9.9.9"),
        "149.112.112.112" => Some("149.112.112.112"),
        "208.67.222.222" => Some("208.67.222.222"),
        "208.67.220.220" => Some("208.67.220.220"),
        "2606:4700:4700::1111" => Some("[2606:4700:4700::1111]"),
        "2606:4700:4700::1001" => Some("[2606:4700:4700::1001]"),
        "2001:4860:4860::8888" => Some("[2001:4860:4860::8888]"),
        "2001:4860:4860::8844" => Some("[2001:4860:4860::8844]"),
        _ => None,
    }
}

fn rewrite_dns_server_entry(
    raw: &str,
    default_detour: &str,
    route_names: &[String],
    plain_tcp_only_detour: bool,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || matches!(raw, "system" | "local" | "dhcp://system") {
        return None;
    }
    if let Some((address_part, explicit_fragment)) = raw.split_once('#') {
        if !plain_tcp_only_detour {
            return None;
        }
        let detour = resolve_declared_route(route_names, explicit_fragment)?;
        let normalized = if address_part.contains("://") {
            address_part.to_owned()
        } else if let Ok(ip) = address_part.parse::<IpAddr>() {
            format!("udp://{}", std::net::SocketAddr::new(ip, 53))
        } else {
            format!("udp://{address_part}")
        };
        let url = reqwest::Url::parse(&normalized).ok()?;
        if matches!(url.scheme(), "udp" | "tcp") {
            return None;
        }
        let host = url
            .host_str()?
            .trim_start_matches('[')
            .trim_end_matches(']');
        let tcp_host = if let Some(canonical) = poisoned_foreign_dns_canonical_ip(host) {
            canonical.to_owned()
        } else if let Ok(ip) = host.parse::<IpAddr>() {
            if ip.is_ipv6() {
                format!("[{ip}]")
            } else {
                ip.to_string()
            }
        } else {
            return None;
        };
        return Some(format!("tcp://{tcp_host}:53#{detour}"));
    }

    let normalized = if raw.contains("://") {
        raw.to_owned()
    } else if let Ok(ip) = raw.parse::<IpAddr>() {
        format!("udp://{}", std::net::SocketAddr::new(ip, 53))
    } else if resolve_declared_route(route_names, raw).is_some() {
        return None;
    } else {
        format!("udp://{raw}")
    };
    let url = reqwest::Url::parse(&normalized).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let canonical_host = poisoned_foreign_dns_canonical_ip(host)?;
    match url.scheme() {
        "udp" | "tcp" => {
            let port = url.port().unwrap_or(53);
            if port == 0 || !matches!(url.path(), "" | "/") || url.query().is_some() {
                return None;
            }
            Some(format!("tcp://{canonical_host}:{port}#{default_detour}"))
        }
        "https" | "h3" => {
            let port = url.port();
            if port == Some(0) {
                return None;
            }
            if plain_tcp_only_detour {
                return Some(format!("tcp://{canonical_host}:53#{default_detour}"));
            }
            let port_suffix = port.map(|p| format!(":{p}")).unwrap_or_default();
            let path = if url.path().is_empty() || url.path() == "/" {
                "/dns-query"
            } else {
                url.path()
            };
            let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
            Some(format!(
                "{}://{canonical_host}{port_suffix}{path}{query}#{default_detour}",
                url.scheme()
            ))
        }
        "tls" | "quic" => {
            let port = url.port();
            if port == Some(0) || !matches!(url.path(), "" | "/") || url.query().is_some() {
                return None;
            }
            if plain_tcp_only_detour {
                return Some(format!("tcp://{canonical_host}:53#{default_detour}"));
            }
            let port_suffix = port.map(|p| format!(":{p}")).unwrap_or_default();
            Some(format!(
                "{}://{canonical_host}{port_suffix}#{default_detour}",
                url.scheme()
            ))
        }
        _ => None,
    }
}

fn rewrite_dns_value_in_place(
    value: &mut serde_yaml::Value,
    default_detour: &str,
    route_names: &[String],
    plain_tcp_only_detour: bool,
) {
    match value {
        serde_yaml::Value::String(raw) => {
            if let Some(rewritten) =
                rewrite_dns_server_entry(raw, default_detour, route_names, plain_tcp_only_detour)
            {
                *raw = rewritten;
            }
        }
        serde_yaml::Value::Sequence(items) => {
            let has_group_shorthand = items.iter().filter_map(serde_yaml::Value::as_str).any(|s| {
                let s = s.trim();
                !s.contains("://")
                    && !s.contains('#')
                    && s.parse::<IpAddr>().is_err()
                    && resolve_declared_route(route_names, s).is_some()
            });
            if has_group_shorthand {
                return;
            }
            for item in items.iter_mut() {
                if let serde_yaml::Value::String(raw) = item {
                    if let Some(rewritten) = rewrite_dns_server_entry(
                        raw,
                        default_detour,
                        route_names,
                        plain_tcp_only_detour,
                    ) {
                        *raw = rewritten;
                    }
                }
            }
        }
        _ => {}
    }
}

fn harden_runtime_dns_mapping(
    root: &mut serde_yaml::Mapping,
    mode: RuntimeMode,
    selections: &BTreeMap<String, String>,
    plain_tcp_only_detour: bool,
) {
    let route_names = declared_route_names(root);
    let Some(detour) = preferred_dns_detour(root, mode, selections) else {
        return;
    };
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    let Some(dns) = root
        .get_mut(key("dns"))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return;
    };
    if dns.get(key("enable")).and_then(serde_yaml::Value::as_bool) == Some(false) {
        return;
    }
    let domestic_bootstrap = || {
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("223.5.5.5".to_owned()),
            serde_yaml::Value::String("119.29.29.29".to_owned()),
        ])
    };
    for bootstrap_field in ["default-nameserver", "proxy-server-nameserver"] {
        let missing_or_empty = match dns.get(key(bootstrap_field)) {
            None | Some(serde_yaml::Value::Null) => true,
            Some(serde_yaml::Value::Sequence(seq)) => seq.is_empty(),
            _ => false,
        };
        if missing_or_empty {
            dns.insert(key(bootstrap_field), domestic_bootstrap());
        }
    }
    for list_field in ["nameserver", "fallback"] {
        if let Some(value) = dns.get_mut(key(list_field)) {
            rewrite_dns_value_in_place(value, &detour, &route_names, plain_tcp_only_detour);
        }
    }
    if let Some(policy) = dns
        .get_mut(key("nameserver-policy"))
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        for (matcher, servers) in policy.iter_mut() {
            let is_domestic_matcher = matcher.as_str().is_some_and(|m| {
                matches!(
                    m.trim().to_ascii_lowercase().as_str(),
                    "geosite:cn" | "geosite:private" | "geosite:apple-cn" | "geosite:steam@cn"
                )
            });
            if !is_domestic_matcher {
                rewrite_dns_value_in_place(servers, &detour, &route_names, plain_tcp_only_detour);
            }
        }
    }
}

pub struct RenderedSingBoxConfig {
    pub config_json: String,
    pub proxy_count: usize,
    pub group_count: usize,
    pub rule_count: usize,
    pub skipped_rule_count: usize,
}

fn yaml_field<'a>(mapping: &'a serde_yaml::Mapping, name: &str) -> Option<&'a serde_yaml::Value> {
    mapping.get(serde_yaml::Value::String(name.to_owned()))
}

fn yaml_string(mapping: &serde_yaml::Mapping, name: &str) -> Option<String> {
    yaml_field(mapping, name)
        .and_then(serde_yaml::Value::as_str)
        .map(ToOwned::to_owned)
}

fn yaml_u64(mapping: &serde_yaml::Mapping, name: &str) -> Option<u64> {
    yaml_field(mapping, name).and_then(serde_yaml::Value::as_u64)
}

fn sing_box_tag(name: &str) -> String {
    match name.trim().to_ascii_uppercase().as_str() {
        "DIRECT" => "direct".to_owned(),
        "REJECT" | "REJECT-DROP" => "block".to_owned(),
        _ => name.to_owned(),
    }
}

pub fn sing_box_proxy_outbound(proxy: &serde_yaml::Value) -> Option<serde_json::Value> {
    let mut outbound = sing_box_proxy::outbound(proxy)?;
    // Provider refresh invokes this same converter; new members must resolve
    // through the same actual owner DNS policy as the initial static roster.
    if outbound.get("server").is_some() {
        outbound["domain_resolver"] = serde_json::json!(dns::PROXY_RESOLVER);
    }
    Some(outbound)
}

pub fn render_sing_box_runtime_json(
    runtime_yaml: &str,
    mode: RuntimeMode,
    selections: &BTreeMap<String, String>,
    options: &VpnOptions,
    services: &EngineNetworkServices,
) -> Result<RenderedSingBoxConfig, EngineError> {
    services.validate()?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(runtime_yaml)
        .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
    {
        let root_mut = value.as_mapping_mut().ok_or_else(|| {
            EngineError::InvalidConfig("profile root must be a YAML map".to_owned())
        })?;
        harden_runtime_dns_mapping(root_mut, mode, selections, false);
    }
    let root = value
        .as_mapping()
        .ok_or_else(|| EngineError::InvalidConfig("profile root must be a YAML map".to_owned()))?;
    validate_sing_box_provider_definitions(root)?;
    let proxies = yaml_field(root, "proxies")
        .and_then(serde_yaml::Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let groups = yaml_field(root, "proxy-groups")
        .and_then(serde_yaml::Value::as_sequence)
        .cloned()
        .unwrap_or_default();

    let declarations = |name: &str| -> Result<BTreeMap<String, serde_json::Value>, EngineError> {
        match yaml_field(root, name) {
            None => Ok(BTreeMap::new()),
            Some(value) => {
                serde_json::from_value(serde_json::to_value(value).map_err(|_| {
                    EngineError::InvalidConfig(format!("invalid {name} declarations"))
                })?)
                .map_err(|_| {
                    EngineError::InvalidConfig(format!("{name} must be a name-to-definition map"))
                })
            }
        }
    };
    let proxy_providers = declarations("proxy-providers")?;
    let rule_providers = declarations("rule-providers")?;
    let mut outbounds = Vec::new();
    let mut available_tags = BTreeSet::from(["direct".to_owned(), "block".to_owned()]);
    for proxy in &proxies {
        let outbound = sing_box_proxy_outbound(proxy).ok_or_else(|| {
            EngineError::InvalidConfig(
                "sing-box proxy protocol, required field, or transport is unsupported".into(),
            )
        })?;
        let tag = outbound
            .get("tag")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| EngineError::InvalidConfig("proxy has no nonempty name".into()))?;
        if !available_tags.insert(tag.to_owned()) {
            return Err(EngineError::InvalidConfig(
                "duplicate or reserved proxy name".into(),
            ));
        }
        outbounds.push(outbound);
    }
    let proxy_count = outbounds.len();
    let first_proxy = outbounds
        .first()
        .and_then(|outbound| outbound["tag"].as_str())
        .map(ToOwned::to_owned);
    let mut group_names = Vec::with_capacity(groups.len());
    for group in &groups {
        let group = group
            .as_mapping()
            .ok_or_else(|| EngineError::InvalidConfig("proxy group must be a map".into()))?;
        let name = yaml_string(group, "name")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| EngineError::InvalidConfig("proxy group has no name".into()))?;
        if !available_tags.insert(name.clone()) {
            return Err(EngineError::InvalidConfig(
                "duplicate or reserved group name".into(),
            ));
        }
        group_names.push(name);
    }
    for (group, name) in groups.iter().zip(&group_names) {
        let group = group
            .as_mapping()
            .expect("group declarations were validated");
        let kind = yaml_string(group, "type")
            .ok_or_else(|| EngineError::InvalidConfig("proxy group has no type".into()))?;
        if !matches!(
            kind.as_str(),
            "select" | "url-test" | "fallback" | "load-balance"
        ) {
            return Err(EngineError::InvalidConfig(
                "sing-box group strategy is unsupported".into(),
            ));
        }
        let strings = |field: &str| -> Result<Vec<String>, EngineError> {
            match yaml_field(group, field) {
                None => Ok(Vec::new()),
                Some(value) => value
                    .as_sequence()
                    .ok_or_else(|| {
                        EngineError::InvalidConfig(format!("group {field} must be a sequence"))
                    })?
                    .iter()
                    .map(|entry| {
                        entry
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| {
                                EngineError::InvalidConfig(format!(
                                    "group {field} has a non-string or empty entry"
                                ))
                            })
                    })
                    .collect(),
            }
        };
        let members = strings("proxies")?
            .into_iter()
            .map(|name| sing_box_tag(&name))
            .collect::<Vec<_>>();
        if members
            .iter()
            .any(|member| member == name || !available_tags.contains(member))
        {
            return Err(EngineError::InvalidConfig(
                "group has an unknown or self-referencing member".into(),
            ));
        }
        let providers = strings("use")?;
        if providers
            .iter()
            .any(|provider| !proxy_providers.contains_key(provider))
        {
            return Err(EngineError::InvalidConfig(
                "group references an unknown proxy provider".into(),
            ));
        }
        let include_all = yaml_field(group, "include-all")
            .and_then(serde_yaml::Value::as_bool)
            .unwrap_or(false);
        let include_all_providers = include_all
            || yaml_field(group, "include-all-providers")
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(false);
        let include_all_proxies = include_all
            || yaml_field(group, "include-all-proxies")
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(false);
        if members.is_empty()
            && providers.is_empty()
            && !include_all_proxies
            && !include_all_providers
        {
            return Err(EngineError::InvalidConfig(
                "group has no member source".into(),
            ));
        }
        let selected = selections
            .get(name)
            .cloned()
            .or_else(|| yaml_string(group, "default-selected"));
        let get =
            |key: &str| yaml_field(group, key).and_then(|value| serde_json::to_value(value).ok());
        outbounds.push(serde_json::json!({
            "type":"paws-group", "tag":name, "kind":kind, "outbounds":members,"use":providers,
            "default":selected, "url":yaml_string(group,"url"), "interval":yaml_u64(group,"interval").unwrap_or(300),
            "timeout":yaml_u64(group,"timeout").unwrap_or(5000),"tolerance":yaml_u64(group,"tolerance").unwrap_or(50),
            "lazy":get("lazy").unwrap_or(serde_json::json!(true)), "strategy":yaml_string(group,"strategy").unwrap_or_else(||"consistent-hashing".into()),
            "include_all_proxies":include_all_proxies,"include_all_providers":include_all_providers,
            "filter":yaml_string(group,"filter"),"exclude_filter":yaml_string(group,"exclude-filter"),"exclude_type":yaml_string(group,"exclude-type"),
            "expected_status":yaml_string(group,"expected-status"),"disable_udp":get("disable-udp").unwrap_or(serde_json::json!(false)),
            "interrupt_exist_connections":get("interrupt-exist-connections").unwrap_or(serde_json::json!(false)),
        }));
    }
    let group_count = group_names.len();
    outbounds.push(serde_json::json!({"type":"direct","tag":"direct"}));
    outbounds.push(serde_json::json!({"type":"block","tag":"block"}));
    // Declaration order is policy: a BTreeSet must never choose the first
    // group alphabetically, and MATCH must stay in the source rule position.
    let final_outbound = group_names
        .first()
        .cloned()
        .or(first_proxy)
        .unwrap_or_else(|| "direct".into());
    let global_target = selections
        .get("GLOBAL")
        .map(|name| sing_box_tag(name))
        .unwrap_or_else(|| final_outbound.clone());
    if !available_tags.contains(&global_target) {
        return Err(EngineError::InvalidConfig(
            "GLOBAL selection names an unknown target".into(),
        ));
    }
    let mut route_rules = vec![serde_json::json!({"action":"sniff"})];
    if options.dns_hijacking {
        route_rules.push(serde_json::json!({"port":53,"action":"hijack-dns"}));
        route_rules.push(serde_json::json!({"protocol":"dns","action":"hijack-dns"}));
    }
    route_rules.push(serde_json::json!({"clash_mode":"direct","outbound":"direct"}));
    route_rules.push(serde_json::json!({"clash_mode":"global","outbound":global_target}));
    let rules = match yaml_field(root, "rules") {
        None => Vec::new(),
        Some(value) => value
            .as_sequence()
            .ok_or_else(|| EngineError::InvalidConfig("rules must be a sequence".into()))?
            .iter()
            .map(|rule| {
                rule.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| EngineError::InvalidConfig("rule entry must be a string".into()))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    for source in &rules {
        let (mut compiled, target) = rule::route(source)?;
        if !available_tags.contains(&target) {
            return Err(EngineError::InvalidConfig(
                "rule references an unknown route target".into(),
            ));
        }
        compiled["outbound"] = serde_json::json!(target);
        route_rules.push(compiled);
    }
    let translated_rule_count = rules.len();
    let skipped_rule_count = 0;
    let addresses = if options.addresses.is_empty() {
        vec!["172.19.0.1/30".to_owned()]
    } else {
        options.addresses.clone()
    };
    let dns = dns::compile(root, options, &available_tags)?;
    let (users, skip_auth_prefixes) = public_mixed_auth(root)?;
    let geodata_paths = geodata_paths(root)?;
    let mut config = serde_json::json!({
        "log": {"level": "info", "timestamp": true},
        "dns": dns.options,
        "inbounds": [
            {
                "type": "tun",
                "tag": "tun-in",
                "address": addresses,
                "mtu": options.mtu,
                // Smoltcp/Lwip select Meow's packet stack. sing-box has its
                // own compiled mixed stack and cannot parse those enum names.
                "stack": "mixed",
                "auto_route": false,
                "strict_route": false,
            },
            // Keep the candidate benchmark on sing-box's own data plane. The
            // previous benchmark forwarded port 7890 even though sing-box had
            // no mixed inbound, so a reset only proved that the test hit the
            // wrong/nonexistent listener.
            {
                "type": "mixed",
                "tag": "mixed-in",
                "listen": "127.0.0.1",
                "listen_port": 7890,
            },
            {"type":"mixed","tag":"sing-box-benchmark","listen":"127.0.0.1","listen_port":18492}
        ],
        "outbounds": outbounds,
        "experimental":{"clash_api":{"external_controller":"127.0.0.1:19090","default_mode":mode.as_str()}},
        "route": {
            "rules": route_rules,
            "final": final_outbound,
            "auto_detect_interface": true,
            "default_domain_resolver": dns::PROXY_RESOLVER,
        },
    });
    if let Some(cache_file) = dns.cache_file {
        config["experimental"]["cache_file"] = cache_file;
    }
    let inbounds = config["inbounds"]
        .as_array_mut()
        .expect("compiler creates inbound array");
    if services.ports.mixed_enabled {
        inbounds[1]["listen"] = serde_json::json!(if services.access.allow_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        });
        inbounds[1]["listen_port"] = serde_json::json!(services.ports.mixed_port);
        inbounds[1]["users"] = serde_json::json!(users);
        inbounds[1]["paws_skip_auth_prefixes"] = serde_json::json!(skip_auth_prefixes);
    } else {
        inbounds.remove(1);
    }
    // The bridge consumes this metadata before the strict native options
    // decoder. It owns the optional public API listener separately.
    config["paws_runtime"] = serde_json::to_value(services)
        .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
    config["paws_runtime"]["proxyProviders"] = serde_json::to_value(proxy_providers)
        .map_err(|_| EngineError::InvalidConfig("invalid proxy providers".into()))?;
    config["paws_runtime"]["ruleProviders"] = serde_json::to_value(rule_providers)
        .map_err(|_| EngineError::InvalidConfig("invalid rule providers".into()))?;
    config["paws_runtime"]["runtimeRules"] = serde_json::to_value(rules)
        .map_err(|_| EngineError::InvalidConfig("invalid runtime rules".into()))?;
    config["paws_runtime"]["dnsConfig"] = serde_json::to_value(dns.config)
        .map_err(|_| EngineError::InvalidConfig("invalid DNS metadata".into()))?;
    config["paws_runtime"]["geodataPaths"] = serde_json::to_value(geodata_paths)
        .map_err(|_| EngineError::InvalidConfig("invalid geodata resource paths".into()))?;
    Ok(RenderedSingBoxConfig {
        config_json: serde_json::to_string(&config)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?,
        proxy_count,
        group_count,
        rule_count: translated_rule_count,
        skipped_rule_count,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GeoDataPaths {
    country_mmdb: Option<std::path::PathBuf>,
    asn_mmdb: Option<std::path::PathBuf>,
    geosite: Option<std::path::PathBuf>,
}
fn geodata_paths(root: &serde_yaml::Mapping) -> Result<GeoDataPaths, EngineError> {
    let empty = serde_yaml::Mapping::new();
    let paths = match yaml_field(root, "geodata") {
        None | Some(serde_yaml::Value::Null) => &empty,
        Some(value) => value
            .as_mapping()
            .ok_or_else(|| EngineError::InvalidConfig("geodata must be a path mapping".into()))?,
    };
    let path = |field: &str| match yaml_field(paths, field) {
        None | Some(serde_yaml::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|path| !path.is_empty())
            .map(|path| Some(std::path::PathBuf::from(path)))
            .ok_or_else(|| EngineError::InvalidConfig(format!("invalid geodata {field}"))),
    };
    Ok(GeoDataPaths {
        country_mmdb: path("mmdb-path")?,
        asn_mmdb: path("asn-path")?,
        geosite: path("geosite-path")?,
    })
}
fn public_mixed_auth(
    root: &serde_yaml::Mapping,
) -> Result<(Vec<serde_json::Value>, Vec<String>), EngineError> {
    let list = |field: &str| match yaml_field(root, field) {
        None | Some(serde_yaml::Value::Null) => Ok(Vec::new()),
        Some(value) => value
            .as_sequence()
            .ok_or_else(|| EngineError::InvalidConfig(format!("{field} must be a string list")))?
            .iter()
            .map(|value| {
                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                    EngineError::InvalidConfig(format!("{field} must be a string list"))
                })
            })
            .collect::<Result<Vec<_>, _>>(),
    };
    let mut users = BTreeMap::new();
    for credential in list("authentication")? {
        let (username, password) = credential
            .split_once(':')
            .filter(|(name, _)| !name.is_empty())
            .ok_or_else(|| {
                EngineError::InvalidConfig(
                    "authentication requires nonempty usernames and ':'".into(),
                )
            })?;
        users.insert(username.to_owned(), password.to_owned());
    }
    // Match the established Meow library behavior. These skips belong only
    // to the user's public mixed listener, never the authenticated app lane.
    let mut skips = vec!["127.0.0.1/32".into(), "::1/128".into()];
    for prefix in list("skip-auth-prefixes")? {
        let valid = prefix.split_once('/').is_some_and(|(address, bits)| {
            address
                .parse::<IpAddr>()
                .ok()
                .zip(bits.parse::<u8>().ok())
                .is_some_and(|(address, bits)| bits <= if address.is_ipv4() { 32 } else { 128 })
        });
        if !valid {
            return Err(EngineError::InvalidConfig(
                "invalid skip-auth-prefixes CIDR".into(),
            ));
        }
        if !skips.contains(&prefix) {
            skips.push(prefix);
        }
    }
    Ok((
        users
            .into_iter()
            .map(
                |(username, password)| serde_json::json!({"username":username,"password":password}),
            )
            .collect(),
        skips,
    ))
}

/// Read the controller endpoint from the exact configuration sent to the backend.
pub fn controller_endpoint(
    engine: paws_model::VpnEngine,
    config: &str,
) -> Result<paws_engine::EngineControllerEndpoint, EngineError> {
    let value: serde_json::Value = if engine == paws_model::VpnEngine::SingBox {
        serde_json::from_str(config)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?
    } else {
        serde_json::to_value(
            serde_yaml::from_str::<serde_yaml::Value>(config)
                .map_err(|error| EngineError::InvalidConfig(error.to_string()))?,
        )
        .map_err(|error| EngineError::InvalidConfig(error.to_string()))?
    };
    let (address, secret) = if engine == paws_model::VpnEngine::SingBox {
        let api = &value["experimental"]["clash_api"];
        (api["external_controller"].as_str(), api["secret"].as_str())
    } else {
        (
            value["external-controller"].as_str(),
            value["secret"].as_str(),
        )
    };
    let address = address
        .ok_or_else(|| {
            EngineError::InvalidConfig("runtime has no native controller endpoint".into())
        })?
        .parse()
        .map_err(|_| EngineError::InvalidConfig("runtime controller address is invalid".into()))?;
    Ok(paws_engine::EngineControllerEndpoint {
        address,
        secret: secret
            .filter(|secret| !secret.is_empty())
            .map(ToOwned::to_owned),
    })
}

pub fn secure_controller_config(
    engine: paws_model::VpnEngine,
    config: &str,
    secret: &str,
) -> Result<String, EngineError> {
    if secret.is_empty() {
        return Err(EngineError::InvalidConfig(
            "private controller credential is missing".into(),
        ));
    }
    if engine == paws_model::VpnEngine::SingBox {
        let mut value: serde_json::Value = serde_json::from_str(config)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
        value["experimental"]["clash_api"]["secret"] = serde_json::Value::String(secret.into());
        let private = value["inbounds"]
            .as_array_mut()
            .and_then(|inbounds| {
                inbounds
                    .iter_mut()
                    .find(|inbound| inbound["tag"] == "sing-box-benchmark")
            })
            .ok_or_else(|| {
                EngineError::InvalidConfig("private app proxy listener is missing".into())
            })?;
        private["users"] = serde_json::json!([{"username":"paws-app","password":secret}]);
        private
            .as_object_mut()
            .expect("compiler emits inbound object")
            .remove("paws_skip_auth_prefixes");
        serde_json::to_string(&value).map_err(|error| EngineError::InvalidConfig(error.to_string()))
    } else {
        let mut value: serde_yaml::Value = serde_yaml::from_str(config)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
        value
            .as_mapping_mut()
            .ok_or_else(|| EngineError::InvalidConfig("profile root is not a map".into()))?
            .insert(
                serde_yaml::Value::String("secret".into()),
                serde_yaml::Value::String(secret.into()),
            );
        let private = value
            .get_mut("listeners")
            .and_then(serde_yaml::Value::as_sequence_mut)
            .and_then(|listeners| {
                listeners.iter_mut().find(|listener| {
                    listener.get("name").and_then(serde_yaml::Value::as_str)
                        == Some("tinypaws-mihomo-benchmark")
                })
            })
            .ok_or_else(|| {
                EngineError::InvalidConfig("private app proxy listener is missing".into())
            })?;
        private
            .as_mapping_mut()
            .ok_or_else(|| EngineError::InvalidConfig("private listener must be an object".into()))?
            .insert(
                serde_yaml::Value::String("users".into()),
                serde_yaml::to_value(
                    serde_json::json!([{"username":"paws-app","password":secret}]),
                )
                .map_err(|_| {
                    EngineError::InvalidConfig("invalid private listener authentication".into())
                })?,
            );
        serde_yaml::to_string(&value).map_err(|error| EngineError::InvalidConfig(error.to_string()))
    }
}

#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
