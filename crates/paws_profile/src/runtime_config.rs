use super::*;

pub const CONTROLLER_LOOPBACK_HOST: &str = "127.0.0.1";
pub const CONTROLLER_LAN_HOST: &str = "0.0.0.0";
const CONTROLLER_SECRET_BYTES: usize = 32;
const CONTROLLER_SECRET_HEX_LENGTH: usize = CONTROLLER_SECRET_BYTES * 2;

pub(super) struct ControllerSecretGenerator;

impl ControllerSecretGenerator {
    pub(super) fn generate() -> Result<String, PawsError> {
        let mut bytes = [0_u8; CONTROLLER_SECRET_BYTES];
        getrandom::getrandom(&mut bytes).map_err(|err| {
            PawsError::Core(format!("failed to generate controller secret: {err}"))
        })?;
        let mut secret = String::with_capacity(CONTROLLER_SECRET_HEX_LENGTH);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            secret.push(char::from(HEX[usize::from(byte >> 4)]));
            secret.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Ok(secret)
    }
}

pub(super) fn controller_secret_is_valid(secret: &str) -> bool {
    secret.len() == CONTROLLER_SECRET_HEX_LENGTH
        && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn controller_access_from_yaml(raw_yaml: &str) -> Result<ControllerAccessConfig, PawsError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping() else {
        return Err(PawsError::Core(
            "profile root must be a YAML map".to_owned(),
        ));
    };
    controller_access_from_mapping(root)
}

pub fn network_ports_from_yaml(raw_yaml: &str) -> Result<NetworkPortConfig, PawsError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping() else {
        return Err(PawsError::Core(
            "profile root must be a YAML map".to_owned(),
        ));
    };
    network_ports_from_mapping(root)
}

fn invalid_profile_field(field: &str, expected: &str) -> PawsError {
    PawsError::Core(format!(
        "profile field '{field}' must be {expected}; refusing to use a default"
    ))
}

fn optional_mapping<'a>(root: &'a Mapping, key: &str) -> Result<Option<&'a Mapping>, PawsError> {
    match root.get(value_key(key)) {
        None => Ok(None),
        Some(Value::Mapping(value)) => Ok(Some(value)),
        Some(_) => Err(invalid_profile_field(key, "a mapping")),
    }
}

fn strict_bool(value: &Value, field: &str) -> Result<bool, PawsError> {
    value
        .as_bool()
        .ok_or_else(|| invalid_profile_field(field, "a boolean"))
}

fn strict_string(value: &Value, field: &str) -> Result<String, PawsError> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_profile_field(field, "a string"))
}

fn strict_u16(value: &Value, field: &str) -> Result<u16, PawsError> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid_profile_field(field, "an integer between 1 and 65535"))
}

fn aliased_field<T: PartialEq>(
    map: &Mapping,
    canonical: &str,
    legacy: &str,
    parse: impl Fn(&Value, &str) -> Result<T, PawsError>,
) -> Result<Option<T>, PawsError> {
    let canonical_value = map
        .get(value_key(canonical))
        .map(|value| parse(value, canonical))
        .transpose()?;
    let legacy_value = map
        .get(value_key(legacy))
        .map(|value| parse(value, legacy))
        .transpose()?;
    match (canonical_value, legacy_value) {
        (Some(canonical_value), Some(legacy_value)) if canonical_value != legacy_value => {
            Err(PawsError::Core(format!(
                "profile fields '{canonical}' and legacy alias '{legacy}' conflict"
            )))
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn strict_string_list(map: &Mapping, key: &str) -> Result<Option<Vec<String>>, PawsError> {
    let Some(value) = map.get(value_key(key)) else {
        return Ok(None);
    };
    let values = match value {
        Value::String(value) => vec![value.clone()],
        Value::Sequence(values) => values
            .iter()
            .map(|value| strict_string(value, key))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(invalid_profile_field(key, "a string or list of strings")),
    };
    Ok(Some(values))
}

fn strict_string_list_map(
    map: &Mapping,
    key: &str,
) -> Result<Option<BTreeMap<String, Vec<String>>>, PawsError> {
    let Some(value) = map.get(value_key(key)) else {
        return Ok(None);
    };
    let Value::Mapping(entries) = value else {
        return Err(invalid_profile_field(key, "a mapping of string lists"));
    };
    let mut result = BTreeMap::new();
    for (matcher, servers) in entries {
        let matcher = strict_string(matcher, key)?;
        if matcher.trim().is_empty() {
            return Err(invalid_profile_field(key, "non-empty string keys"));
        }
        let servers = match servers {
            Value::String(server) => vec![server.clone()],
            Value::Sequence(servers) => servers
                .iter()
                .map(|server| strict_string(server, key))
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(invalid_profile_field(key, "a mapping of string lists")),
        };
        let servers = normalize_dns_optional_servers(servers);
        if servers.is_empty() {
            return Err(invalid_profile_field(key, "non-empty DNS server lists"));
        }
        result.insert(matcher.trim().to_owned(), servers);
    }
    Ok(Some(result))
}

fn validate_networks(
    values: &[String],
    field: &str,
    expected_ipv6: Option<bool>,
) -> Result<(), PawsError> {
    for value in values {
        let network = value.parse::<IpNet>().map_err(|error| {
            PawsError::Core(format!(
                "profile field '{field}' contains invalid network '{value}': {error}"
            ))
        })?;
        let is_ipv6 = matches!(network, IpNet::V6(_));
        if expected_ipv6.is_some_and(|expected| expected != is_ipv6) {
            return Err(PawsError::Core(format!(
                "profile field '{field}' contains the wrong address family: {value}"
            )));
        }
    }
    Ok(())
}

pub(super) fn network_ports_from_mapping(root: &Mapping) -> Result<NetworkPortConfig, PawsError> {
    let defaults = NetworkPortConfig::default();
    let Some(paws) = optional_mapping(root, "paws")? else {
        return Ok(defaults);
    };
    let ports = NetworkPortConfig {
        mixed_port: aliased_field(paws, "mixed-port", "mixedPort", strict_u16)?
            .unwrap_or(defaults.mixed_port),
        controller_port: aliased_field(paws, "controller-port", "controllerPort", strict_u16)?
            .unwrap_or(defaults.controller_port),
        mixed_enabled: aliased_field(paws, "mixed-enabled", "mixedEnabled", strict_bool)?
            .unwrap_or(false),
        controller_enabled: aliased_field(
            paws,
            "controller-enabled",
            "controllerEnabled",
            strict_bool,
        )?
        .unwrap_or(false),
    };
    ports.validate()
}

pub(super) fn controller_access_from_mapping(
    root: &Mapping,
) -> Result<ControllerAccessConfig, PawsError> {
    let Some(paws) = optional_mapping(root, "paws")? else {
        return Ok(ControllerAccessConfig::default());
    };
    let allow_lan = aliased_field(
        paws,
        "controller-allow-lan",
        "controllerAllowLan",
        strict_bool,
    )?
    .unwrap_or(false);
    let secret = aliased_field(paws, "controller-secret", "controllerSecret", strict_string)?;
    if secret
        .as_deref()
        .is_some_and(|secret| !controller_secret_is_valid(secret))
    {
        return Err(invalid_profile_field(
            "controller-secret",
            "a 64-character hexadecimal string",
        ));
    }
    if allow_lan && secret.is_none() {
        return Err(PawsError::Core(
            "LAN controller access requires a generated 64-character secret".to_owned(),
        ));
    }
    Ok(ControllerAccessConfig { allow_lan, secret })
}

pub(super) fn patch_controller_access(
    root: &mut Mapping,
    access: &ControllerAccessConfig,
    controller_port: u16,
) -> Result<(), PawsError> {
    let controller_addr = |host: &str| format!("{host}:{controller_port}");
    if access.allow_lan {
        let secret = access
            .secret
            .as_deref()
            .filter(|secret| controller_secret_is_valid(secret))
            .ok_or_else(|| {
                PawsError::Core(
                    "LAN controller access requires a generated 64-character secret".to_owned(),
                )
            })?;
        put_string(
            root,
            "external-controller",
            &controller_addr(CONTROLLER_LAN_HOST),
        );
        put_string(root, "secret", secret);
    } else {
        put_string(
            root,
            "external-controller",
            &controller_addr(CONTROLLER_LOOPBACK_HOST),
        );
        root.remove(value_key("secret"));
    }
    Ok(())
}

pub fn vpn_options_from_yaml(raw_yaml: &str) -> Result<VpnOptions, PawsError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping() else {
        return Err(PawsError::Core(
            "profile root must be a YAML map".to_owned(),
        ));
    };

    let mut options = VpnOptions::default();
    if let Some(ipv6) = root.get(value_key("ipv6")) {
        options.ipv6 = strict_bool(ipv6, "ipv6")?;
    }

    if let Some(dns) = optional_mapping(root, "dns")? {
        if let Some(nameservers) = strict_string_list(dns, "nameserver")? {
            let nameservers = normalize_dns_optional_servers(nameservers);
            if nameservers.is_empty() {
                return Err(invalid_profile_field(
                    "nameserver",
                    "at least one non-empty DNS server",
                ));
            }
            options.dns_servers = nameservers;
        }
        if let Some(fallbacks) = strict_string_list(dns, "fallback")? {
            options.dns_fallbacks = normalize_dns_optional_servers(fallbacks);
        }
        if let Some(policy) = strict_string_list_map(dns, "nameserver-policy")? {
            options.dns_nameserver_policy = policy;
        }
    }

    if let Some(paws) = optional_mapping(root, "paws")? {
        if let Some(system_proxy) = aliased_field(paws, "system-proxy", "systemProxy", strict_bool)?
        {
            options.system_proxy = system_proxy;
        }
        if let Some(allow_bypass) = aliased_field(paws, "allow-bypass", "allowBypass", strict_bool)?
        {
            options.allow_bypass = allow_bypass;
        }
    }

    if let Some(tun) = optional_mapping(root, "tun")? {
        if let Some(mtu) = tun.get(value_key("mtu")) {
            options.mtu = strict_u16(mtu, "mtu")?;
        }
        if let Some(stack) = tun.get(value_key("stack")) {
            options.stack = normalize_vpn_stack(strict_string(stack, "stack")?)?;
        }
        if let Some(dns_hijack) = tun.get(value_key("dns-hijack")) {
            options.dns_hijacking = match dns_hijack {
                Value::Bool(enabled) => *enabled,
                Value::Sequence(items) => {
                    for item in items {
                        strict_string(item, "dns-hijack")?;
                    }
                    !items.is_empty()
                }
                _ => {
                    return Err(invalid_profile_field(
                        "dns-hijack",
                        "a boolean or list of strings",
                    ));
                }
            };
        }

        let inet4 = strict_string_list(tun, "inet4-address")?.unwrap_or_default();
        let inet6 = strict_string_list(tun, "inet6-address")?.unwrap_or_default();
        let route_addresses = strict_string_list(tun, "route-address")?.unwrap_or_default();
        validate_networks(&inet4, "inet4-address", Some(false))?;
        validate_networks(&inet6, "inet6-address", Some(true))?;
        validate_networks(&route_addresses, "route-address", None)?;

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

/// Validates the application-owned portions of an imported profile before
/// Meow sanitization removes or rewrites them. This prevents malformed values
/// from being laundered into working defaults during import.
pub fn validate_profile_app_config(raw_yaml: &str) -> Result<(), PawsError> {
    vpn_options_from_yaml(raw_yaml)?;
    controller_access_from_yaml(raw_yaml)?;
    network_ports_from_yaml(raw_yaml)?;
    Ok(())
}

pub const DEFAULT_TCP_CONNECT_TIMEOUT_SECONDS: i64 = 10;

pub fn default_runtime_yaml() -> String {
    let ports = NetworkPortConfig::default();
    format!(
        r#"mixed-port: {}
mode: rule
log-level: info
tcp-connect-timeout: {}
external-controller: {}:{}
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
"#,
        ports.mixed_port,
        DEFAULT_TCP_CONNECT_TIMEOUT_SECONDS,
        CONTROLLER_LOOPBACK_HOST,
        ports.controller_port
    )
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

pub(super) fn patch_geodata_paths(root: &mut Mapping, store_root: &Path) -> Result<(), PawsError> {
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
    put_string(&mut tun, "device", "Paws");
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
) -> Result<(), PawsError> {
    // meow-rs 0.21 contains provider paths beneath the runtime config's
    // parent directory. Keep the security check enabled and migrate Paws'
    // older profile-scoped caches into that trusted root on first render.
    rewrite_provider_kind(
        root,
        "proxy-providers",
        store_root.join("runtime/providers/proxy").join(profile_id),
        Some(store_root.join("providers/proxy").join(profile_id)),
        false,
    )?;
    rewrite_provider_kind(
        root,
        "rule-providers",
        store_root.join("runtime/providers/rule").join(profile_id),
        Some(store_root.join("providers/rule").join(profile_id)),
        true,
    )
}

pub(super) fn rewrite_provider_kind(
    root: &mut Mapping,
    key: &str,
    cache_dir: PathBuf,
    legacy_cache_dir: Option<PathBuf>,
    trim_inline_rule_provider_fields: bool,
) -> Result<(), PawsError> {
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
        let file_name = provider_cache_file_name(name);
        let path = cache_dir.join(&file_name);
        if !path.exists() {
            if let Some(legacy_path) = legacy_cache_dir
                .as_ref()
                .map(|legacy_dir| legacy_dir.join(&file_name))
                .filter(|legacy_path| legacy_path.is_file())
            {
                fs::copy(&legacy_path, &path).map_err(io_error)?;
            }
        }
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

#[cfg(test)]
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

#[cfg(test)]
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

pub(super) fn normalize_vpn_stack(stack: String) -> Result<String, PawsError> {
    Ok(VpnStack::try_from(stack.as_str())?.as_str().to_owned())
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
