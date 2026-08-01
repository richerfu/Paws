use super::*;

pub fn vpn_options_from_yaml(raw_yaml: &str) -> Result<VpnOptions, HMetaError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping() else {
        return Err(HMetaError::Core(
            "profile root must be a YAML map".to_owned(),
        ));
    };

    let mut options = VpnOptions::default();
    if let Some(ipv6) = get_bool(root, "ipv6") {
        options.ipv6 = ipv6;
    }

    if let Some(Value::Mapping(dns)) = root.get(&value_key("dns")) {
        let nameservers = get_string_list(dns, "nameserver");
        if !nameservers.is_empty() {
            options.dns_servers = nameservers;
        }
        if dns.contains_key(&value_key("fallback")) {
            options.dns_fallbacks = get_string_list(dns, "fallback");
        }
        if dns.contains_key(&value_key("nameserver-policy")) {
            options.dns_nameserver_policy = get_string_list_map(dns, "nameserver-policy");
        }
    }

    if let Some(Value::Mapping(hmeta)) = root.get(&value_key("hmeta")) {
        if let Some(system_proxy) =
            get_bool(hmeta, "system-proxy").or_else(|| get_bool(hmeta, "systemProxy"))
        {
            options.system_proxy = system_proxy;
        }
        if let Some(allow_bypass) =
            get_bool(hmeta, "allow-bypass").or_else(|| get_bool(hmeta, "allowBypass"))
        {
            options.allow_bypass = allow_bypass;
        }
    }

    if let Some(Value::Mapping(tun)) = root.get(&value_key("tun")) {
        if let Some(mtu) = get_u16(tun, "mtu") {
            options.mtu = mtu;
        }
        if let Some(stack) = get_string(tun, "stack") {
            options.stack = normalize_vpn_stack(stack);
        }
        if let Some(Value::Bool(enabled)) = tun.get(&value_key("dns-hijack")) {
            options.dns_hijacking = *enabled;
        } else if let Some(Value::Sequence(items)) = tun.get(&value_key("dns-hijack")) {
            options.dns_hijacking = !items.is_empty();
        }

        let inet4 = get_string_list(tun, "inet4-address");
        let inet6 = get_string_list(tun, "inet6-address");
        let route_addresses = get_string_list(tun, "route-address");

        options.addresses.clear();
        if inet4.is_empty() {
            options.addresses.push(MEOW_V4_CLIENT.to_owned());
        } else {
            options.addresses.extend(inet4);
        }
        if !inet6.is_empty() {
            options.ipv6 = true;
            options.addresses.extend(inet6);
        }

        if !route_addresses.is_empty() {
            if route_addresses.iter().any(|route| route.contains(':')) {
                options.ipv6 = true;
            }
            options.routes = route_addresses;
        }
    }

    if options.ipv6
        && !options
            .addresses
            .iter()
            .any(|address| address.contains(':'))
    {
        options.addresses.push(MEOW_V6_CLIENT.to_owned());
    }
    if options.ipv6 && !options.routes.iter().any(|route| route.contains(':')) {
        options.routes.push("::/0".to_owned());
    }
    if options.dns_addresses.is_empty() {
        options.dns_addresses.push(MEOW_V4_ROUTER.to_owned());
    }

    Ok(options)
}

pub fn default_runtime_yaml() -> String {
    r#"mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - 223.5.5.5
    - 119.29.29.29
  fallback:
    - 1.1.1.1
    - 8.8.8.8
  nameserver-policy:
    geosite:cn:
      - 223.5.5.5
      - 119.29.29.29
    geosite:geolocation-!cn:
      - 1.1.1.1
      - 8.8.8.8
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#
    .to_owned()
}

pub(super) fn patch_dns(root: &mut Mapping, options: &VpnOptions) {
    let key = value_key("dns");
    let mut dns = root
        .remove(&key)
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    put_bool(&mut dns, "enable", true);
    put_string(&mut dns, "listen", "127.0.0.1:1053");
    dns.insert(
        value_key("nameserver"),
        Value::Sequence(
            options
                .dns_servers
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    if !options.dns_fallbacks.is_empty() {
        dns.insert(
            value_key("fallback"),
            Value::Sequence(
                options
                    .dns_fallbacks
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    } else {
        dns.remove(&value_key("fallback"));
    }
    if !options.dns_nameserver_policy.is_empty() {
        dns.insert(
            value_key("nameserver-policy"),
            Value::Mapping(
                options
                    .dns_nameserver_policy
                    .iter()
                    .map(|(matcher, servers)| {
                        (
                            value_key(matcher),
                            Value::Sequence(servers.iter().cloned().map(Value::String).collect()),
                        )
                    })
                    .collect(),
            ),
        );
    } else {
        dns.remove(&value_key("nameserver-policy"));
    }
    remove_app_managed_dns_fields(&mut dns);
    put_bool(&mut dns, "use-system-hosts", false);
    dns.insert(
        value_key("default-nameserver"),
        Value::Sequence(
            default_dns_bootstrap_servers(options)
                .map(|server| Value::String(server.to_owned()))
                .collect(),
        ),
    );
    root.insert(key, Value::Mapping(dns));
}

pub(super) fn remove_app_managed_dns_fields(dns: &mut Mapping) {
    for key in [
        "enhanced-mode",
        "fake-ip-range",
        "fake-ip-filter",
        "fake-ip-filter-mode",
        "fallback-filter",
    ] {
        dns.remove(&value_key(key));
    }
}

pub(super) fn sanitize_app_managed_dns_for_validation(root: &mut Mapping) {
    let Some(Value::Mapping(dns)) = root.get_mut(&value_key("dns")) else {
        return;
    };
    remove_app_managed_dns_fields(dns);
    dns.remove(&value_key("listen"));
    dns.remove(&value_key("default-nameserver"));
    put_bool(dns, "use-system-hosts", false);
}

pub(super) fn sanitize_app_managed_config(root: &mut Mapping) {
    for key in [
        "port",
        "socks-port",
        "redir-port",
        "tproxy-port",
        "mixed-port",
        "allow-lan",
        "bind-address",
        "lan-allowed-ips",
        "lan-disallowed-ips",
        "authentication",
        "skip-auth-prefixes",
        "external-controller",
        "external-controller-tls",
        "external-controller-unix",
        "external-controller-pipe",
        "external-ui",
        "external-ui-name",
        "external-ui-url",
        "external-controller-cors",
        "secret",
        "routing-mark",
        "interface-name",
        "tproxy-sni",
        "subscriptions",
        "listeners",
    ] {
        root.remove(&value_key(key));
    }
}

pub(super) fn patch_geox_url(root: &mut Mapping) {
    let key = value_key("geox-url");
    let mut geox = root
        .remove(&key)
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    put_string(
        &mut geox,
        "geoip",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
    );
    put_string(
        &mut geox,
        "geosite",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
    );
    put_string(
        &mut geox,
        "mmdb",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
    );
    put_string(
        &mut geox,
        "asn",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/GeoLite2-ASN.mmdb",
    );
    root.insert(key, Value::Mapping(geox));
}

pub(super) fn patch_geodata_paths(root: &mut Mapping, store_root: &Path) -> Result<(), HMetaError> {
    let geodata_dir = store_root.join("geodata");
    fs::create_dir_all(&geodata_dir).map_err(io_error)?;
    let key = value_key("geodata");
    root.remove(&key);
    let mut geodata = Mapping::new();
    put_string(
        &mut geodata,
        "mmdb-path",
        &geodata_dir.join("Country.mmdb").to_string_lossy(),
    );
    put_string(
        &mut geodata,
        "asn-path",
        &geodata_dir.join("GeoLite2-ASN.mmdb").to_string_lossy(),
    );
    put_string(
        &mut geodata,
        "geosite-path",
        &geodata_dir.join("geosite.dat").to_string_lossy(),
    );
    root.insert(key, Value::Mapping(geodata));
    Ok(())
}

pub(super) fn prune_unavailable_default_subscription_rules(root: &mut Mapping, store_root: &Path) {
    let Some(Value::Sequence(rules)) = root.get_mut(&value_key("rules")) else {
        return;
    };
    if rules.len() != DEFAULT_PROXY_SUBSCRIPTION_RULES.len()
        || !rules
            .iter()
            .zip(DEFAULT_PROXY_SUBSCRIPTION_RULES)
            .all(|(actual, expected)| {
                actual
                    .as_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
            })
    {
        return;
    }

    let geodata_dir = store_root.join("geodata");
    let geosite_available = file_has_content(&geodata_dir.join("geosite.dat"));
    let geoip_available = file_has_content(&geodata_dir.join("Country.mmdb"));
    rules.retain(|rule| {
        let Some(rule) = rule.as_str() else {
            return true;
        };
        (!rule.eq_ignore_ascii_case(DEFAULT_PROXY_SUBSCRIPTION_RULES[0]) || geosite_available)
            && (!rule.eq_ignore_ascii_case(DEFAULT_PROXY_SUBSCRIPTION_RULES[1]) || geoip_available)
    });
}

pub(super) fn upgrade_legacy_generated_subscription_rules(root: &mut Mapping) {
    let has_legacy_fallback = root
        .get(&value_key("rules"))
        .and_then(Value::as_sequence)
        .is_some_and(|rules| {
            matches!(rules.as_slice(), [rule] if rule.as_str().is_some_and(|rule| rule.eq_ignore_ascii_case("MATCH,Proxy")))
        });
    if !has_legacy_fallback || !looks_like_generated_proxy_subscription(root) {
        return;
    }
    root.insert(
        value_key("rules"),
        Value::Sequence(
            DEFAULT_PROXY_SUBSCRIPTION_RULES
                .iter()
                .map(|rule| Value::String((*rule).to_owned()))
                .collect(),
        ),
    );
}

pub(super) fn looks_like_generated_proxy_subscription(root: &Mapping) -> bool {
    let Some(proxies) = root.get(&value_key("proxies")).and_then(Value::as_sequence) else {
        return false;
    };
    let proxy_names = proxies
        .iter()
        .filter_map(Value::as_mapping)
        .filter_map(|proxy| get_string(proxy, "name"))
        .collect::<Vec<_>>();
    if proxy_names.is_empty() || proxy_names.len() != proxies.len() {
        return false;
    }

    let Some([Value::Mapping(group)]) = root
        .get(&value_key("proxy-groups"))
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
    else {
        return false;
    };
    if !get_string(group, "name").is_some_and(|name| name == "Proxy")
        || !get_string(group, "type").is_some_and(|group_type| group_type == "select")
    {
        return false;
    }

    let mut expected_members = proxy_names;
    expected_members.push("DIRECT".to_owned());
    get_string_list(group, "proxies") == expected_members
}

pub(super) fn file_has_content(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

pub(super) fn remove_app_managed_geodata_fields(root: &mut Mapping) {
    let Some(Value::Mapping(geodata)) = root.get_mut(&value_key("geodata")) else {
        return;
    };
    for key in [
        "auto-update",
        "auto-update-interval",
        "url",
        "geodata-mode",
        "geodata-loader",
        "geoip-matcher",
    ] {
        geodata.remove(&value_key(key));
    }
}

pub(super) fn patch_tun(root: &mut Mapping, options: &VpnOptions) {
    let mut tun = Mapping::new();
    put_bool(&mut tun, "enable", true);
    put_string(&mut tun, "device", "HMeta");
    put_i64(&mut tun, "mtu", i64::from(options.mtu));
    put_string(&mut tun, "stack", &options.stack);
    put_bool(&mut tun, "auto-route", true);
    if options.dns_hijacking {
        tun.insert(
            value_key("dns-hijack"),
            Value::Sequence(vec![Value::String("any:53".to_owned())]),
        );
    }
    root.insert(value_key("tun"), Value::Mapping(tun));
}

pub(super) fn rewrite_provider_paths(
    root: &mut Mapping,
    store_root: &Path,
    profile_id: &str,
) -> Result<(), HMetaError> {
    rewrite_provider_kind(
        root,
        "proxy-providers",
        store_root.join("providers/proxy").join(profile_id),
        false,
    )?;
    rewrite_provider_kind(
        root,
        "rule-providers",
        store_root.join("providers/rule").join(profile_id),
        true,
    )
}

pub(super) fn rewrite_provider_kind(
    root: &mut Mapping,
    key: &str,
    cache_dir: PathBuf,
    trim_inline_rule_provider_fields: bool,
) -> Result<(), HMetaError> {
    let Some(Value::Mapping(providers)) = root.get_mut(&value_key(key)) else {
        return Ok(());
    };
    fs::create_dir_all(&cache_dir).map_err(io_error)?;
    for (name, provider) in providers {
        let Value::String(name) = name else {
            continue;
        };
        let Value::Mapping(provider) = provider else {
            continue;
        };
        if trim_inline_rule_provider_fields && provider_type_is(provider, "inline") {
            provider.remove(&value_key("path"));
            provider.remove(&value_key("interval"));
            continue;
        }
        let path = cache_dir.join(provider_cache_file_name(name));
        provider.insert(
            value_key("path"),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    Ok(())
}

pub(super) fn provider_type_is(provider: &Mapping, expected: &str) -> bool {
    provider
        .get(&value_key("type"))
        .and_then(Value::as_str)
        .is_some_and(|provider_type| provider_type.eq_ignore_ascii_case(expected))
}

pub(super) fn provider_cache_file_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = sanitized.trim_matches('.');
    if !base.is_empty() && base == name {
        return format!("{base}.yaml");
    }

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();
    let base = if base.is_empty() { "provider" } else { base };
    format!("{base}-{hash:016x}.yaml")
}

pub(super) fn merge_rules(root: &mut Mapping, extra_rules: Vec<String>) {
    if extra_rules.is_empty() {
        return;
    }
    let mut original = root
        .remove(&value_key("rules"))
        .and_then(|value| value.as_sequence().cloned())
        .unwrap_or_default();
    let mut merged: Vec<Value> = extra_rules.into_iter().map(Value::String).collect();
    merged.append(&mut original);
    root.insert(value_key("rules"), Value::Sequence(dedup_rules(merged)));
}

pub(super) fn dedup_rules(rules: Vec<Value>) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen_match = false;
    for rule in rules {
        let is_match = rule
            .as_str()
            .map(|line| line.trim_start().to_ascii_uppercase().starts_with("MATCH,"))
            .unwrap_or(false);
        if is_match {
            if seen_match {
                continue;
            }
            seen_match = true;
        }
        result.push(rule);
    }
    result
}

pub(super) fn collect_provider_summaries(
    root: &Mapping,
    key: &str,
    provider_type: &str,
    providers: &mut Vec<ProviderSummary>,
) {
    let Some(Value::Mapping(items)) = root.get(&value_key(key)) else {
        return;
    };
    for (name, item) in items {
        let name = name.as_str().unwrap_or("<unnamed>").to_owned();
        let map = item.as_mapping();
        let path = map.and_then(|m| get_string(m, "path"));
        let cache_metadata = path.as_deref().and_then(provider_cache_metadata);
        let health_check = map
            .and_then(|m| m.get(&value_key("health-check")))
            .and_then(Value::as_mapping);
        providers.push(ProviderSummary {
            name,
            provider_type: provider_type.to_owned(),
            path,
            url: map.and_then(|m| get_string(m, "url")),
            vehicle_type: map.and_then(|m| get_string(m, "type")),
            interval_seconds: map.and_then(|m| get_u64(m, "interval")),
            filter: map.and_then(|m| get_string(m, "filter")),
            exclude_filter: map.and_then(|m| get_string(m, "exclude-filter")),
            behavior: map.and_then(|m| get_string(m, "behavior")),
            format: map.and_then(|m| get_string(m, "format")),
            health_check_enabled: health_check
                .and_then(|m| get_bool(m, "enable"))
                .unwrap_or(false),
            health_check_url: health_check.and_then(|m| get_string(m, "url")),
            health_check_interval_seconds: health_check.and_then(|m| get_u64(m, "interval")),
            expected_status: health_check.and_then(|m| get_string(m, "expected-status")),
            members: Vec::new(),
            cache_exists: cache_metadata
                .as_ref()
                .is_some_and(std::fs::Metadata::is_file),
            cache_bytes: cache_metadata
                .as_ref()
                .filter(|metadata| metadata.is_file())
                .map(std::fs::Metadata::len),
            cache_updated_at: cache_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_secs),
            stale_cache_available: false,
            last_refresh_at: None,
            last_refresh_error: None,
        });
    }
}

pub(super) fn provider_cache_metadata(path: &str) -> Option<std::fs::Metadata> {
    Path::new(path).metadata().ok()
}

pub(super) fn system_time_secs(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().to_string())
}

pub(super) fn put_string(map: &mut Mapping, key: &str, value: &str) {
    map.insert(value_key(key), Value::String(value.to_owned()));
}

pub(super) fn put_bool(map: &mut Mapping, key: &str, value: bool) {
    map.insert(value_key(key), Value::Bool(value));
}

pub(super) fn put_i64(map: &mut Mapping, key: &str, value: i64) {
    map.insert(value_key(key), Value::Number(value.into()));
}

pub(super) fn put_string_sequence(map: &mut Mapping, key: &str, values: Vec<String>) {
    if values.is_empty() {
        return;
    }
    map.insert(
        value_key(key),
        Value::Sequence(values.into_iter().map(Value::String).collect()),
    );
}

pub(super) fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn parse_positive_i64(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok().filter(|value| *value > 0)
}

pub(super) fn get_string(map: &Mapping, key: &str) -> Option<String> {
    map.get(&value_key(key))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn get_bool(map: &Mapping, key: &str) -> Option<bool> {
    map.get(&value_key(key)).and_then(Value::as_bool)
}

pub(super) fn get_u64(map: &Mapping, key: &str) -> Option<u64> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_u64() {
        return Some(number).filter(|value| *value > 0);
    }
    value
        .as_str()
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

pub(super) fn get_u16(map: &Mapping, key: &str) -> Option<u16> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_u64() {
        return u16::try_from(number).ok().filter(|value| *value > 0);
    }
    value
        .as_str()
        .and_then(|text| text.parse::<u16>().ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
pub(super) fn get_i64(map: &Mapping, key: &str) -> Option<i64> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    value.as_str().and_then(|text| text.parse::<i64>().ok())
}

pub(super) fn get_string_list(map: &Mapping, key: &str) -> Vec<String> {
    let Some(value) = map.get(&value_key(key)) else {
        return Vec::new();
    };
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Sequence(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(ToOwned::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn get_string_list_map(map: &Mapping, key: &str) -> BTreeMap<String, Vec<String>> {
    let Some(Value::Mapping(values)) = map.get(&value_key(key)) else {
        return BTreeMap::new();
    };
    values
        .iter()
        .filter_map(|(matcher, servers)| {
            let matcher = matcher.as_str()?.trim();
            if matcher.is_empty() {
                return None;
            }
            let servers = match servers {
                Value::String(server) => vec![server.clone()],
                Value::Sequence(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect(),
                _ => Vec::new(),
            };
            let servers = normalize_dns_optional_servers(servers);
            if servers.is_empty() {
                None
            } else {
                Some((matcher.to_owned(), servers))
            }
        })
        .collect()
}

pub(super) fn normalize_dns_servers(servers: Vec<String>) -> Vec<String> {
    let normalized = normalize_dns_optional_servers(servers);
    if normalized.is_empty() {
        VpnOptions::default().dns_servers
    } else {
        normalized
    }
}

pub(super) fn default_dns_bootstrap_servers(
    options: &VpnOptions,
) -> impl Iterator<Item = &'static str> {
    let needs_global_bootstrap = dns_config_needs_default_nameserver(options);
    DEFAULT_CHINA_DNS_SERVERS.iter().copied().chain(
        DEFAULT_GLOBAL_DNS_FALLBACKS
            .iter()
            .copied()
            .filter(move |_| needs_global_bootstrap),
    )
}

pub(super) fn normalize_dns_optional_servers(servers: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for server in servers {
        let server = server.trim();
        if server.is_empty() || normalized.iter().any(|item| item == server) {
            continue;
        }
        normalized.push(server.to_owned());
    }
    normalized
}

pub(super) fn normalize_dns_policy(
    policy: BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    policy
        .into_iter()
        .filter_map(|(matcher, servers)| {
            let matcher = matcher.trim();
            if matcher.is_empty() {
                return None;
            }
            let servers = normalize_dns_optional_servers(servers);
            if servers.is_empty() {
                None
            } else {
                Some((matcher.to_owned(), servers))
            }
        })
        .collect()
}

pub(super) fn normalize_vpn_stack(stack: String) -> String {
    VpnStack::try_from(stack.as_str())
        .unwrap_or_default()
        .as_str()
        .to_owned()
}

pub(super) fn dns_config_needs_default_nameserver(options: &VpnOptions) -> bool {
    options
        .dns_servers
        .iter()
        .chain(options.dns_fallbacks.iter())
        .chain(
            options
                .dns_nameserver_policy
                .values()
                .flat_map(|servers| servers.iter()),
        )
        .any(|server| encrypted_dns_server_uses_hostname(server))
}

pub(super) fn encrypted_dns_server_uses_hostname(server: &str) -> bool {
    let Some((scheme, _)) = server.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "https" | "tls" | "quic" | "h3") {
        return false;
    }
    Url::parse(server)
        .ok()
        .and_then(|url| url.host().map(|host| matches!(host, url::Host::Domain(_))))
        .unwrap_or(true)
}

pub(super) fn value_key(key: &str) -> Value {
    Value::String(key.to_owned())
}
