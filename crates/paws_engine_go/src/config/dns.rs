//! Pure compilation of the prepared profile's DNS policy. Native transports
//! resolve and exchange on the actual engine owner; this module performs no IO.
use super::{sing_box_tag, yaml_field, yaml_string, EngineError};
use paws_engine::EngineDnsConfig;
use paws_model::VpnOptions;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};

pub(super) const PROXY_RESOLVER: &str = "dns-proxy";
const PRIMARY: &str = "dns-paws";
const BOOTSTRAP: &str = "dns-bootstrap";
const PHYSICAL: &str = "dns-local";

pub(super) struct CompiledDns {
    pub options: Value,
    pub config: EngineDnsConfig,
    pub cache_file: Option<Value>,
}
struct DnsCompiler<'a> {
    routes: &'a BTreeSet<String>,
    servers: Vec<Value>,
    registered: BTreeMap<(String, bool), String>,
}
fn invalid(field: &str) -> EngineError {
    // DNS URLs can contain authentication, paths or query credentials. Report
    // only the field/shape; never echo the full configuration in an error.
    EngineError::InvalidConfig(format!("invalid or unsupported sing-box DNS {field}"))
}
fn strings(value: Option<&serde_yaml::Value>, field: &str) -> Result<Vec<String>, EngineError> {
    match value {
        None | Some(serde_yaml::Value::Null) => Ok(Vec::new()),
        Some(serde_yaml::Value::String(value)) => Ok(vec![value.clone()]),
        Some(serde_yaml::Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| invalid(field))
            })
            .collect(),
        _ => Err(invalid(field)),
    }
}
fn boolean(root: &serde_yaml::Mapping, field: &str, default: bool) -> Result<bool, EngineError> {
    yaml_field(root, field).map_or(Ok(default), |value| {
        value.as_bool().ok_or_else(|| invalid(field))
    })
}
impl<'a> DnsCompiler<'a> {
    fn new(routes: &'a BTreeSet<String>) -> Self {
        Self {
            routes,
            servers: vec![json!({"type":"local","tag":PHYSICAL})],
            registered: BTreeMap::new(),
        }
    }
    fn detour(&self, name: &str) -> Result<Option<String>, EngineError> {
        let normalized = sing_box_tag(name);
        if self.routes.contains(&normalized) {
            return Ok(Some(normalized));
        }
        let mut aliases = self.routes.iter().filter(|route| {
            route
                .trim_start_matches(|character: char| !character.is_alphanumeric())
                .trim()
                == name
        });
        let first = aliases.next().cloned();
        if aliases.next().is_some() {
            return Err(invalid("ambiguous resolver detour"));
        }
        Ok(first)
    }
    /// Shared profile shorthand `[resolver, group]` has exactly the same
    /// meaning as `resolver#group`; a bare group is never sent as a DNS host.
    fn normalize(&self, source: Vec<String>) -> Result<Vec<String>, EngineError> {
        let mut detour = None;
        let mut urls = Vec::new();
        for entry in source {
            let entry = entry.trim();
            if entry.is_empty() {
                return Err(invalid("empty nameserver"));
            }
            if !entry.contains("://") && !entry.contains('#') {
                if let Some(route) = self.detour(entry)? {
                    if detour.as_ref().is_some_and(|old| old != &route) {
                        return Err(invalid("multiple resolver detours"));
                    }
                    detour = Some(route);
                    continue;
                }
            }
            urls.push(entry.to_owned());
        }
        if let Some(detour) = detour {
            if urls.is_empty() {
                return Err(invalid("detour without a nameserver"));
            }
            for url in &mut urls {
                if !url.contains('#') {
                    url.push('#');
                    url.push_str(&detour);
                }
            }
        }
        Ok(urls)
    }
    fn list(&mut self, source: &[String], bootstrap: bool) -> Result<Vec<String>, EngineError> {
        source
            .iter()
            .map(|source| self.register(source, bootstrap))
            .collect()
    }
    fn register(&mut self, source: &str, bootstrap: bool) -> Result<String, EngineError> {
        if matches!(source, "system" | "local" | "dhcp://system") {
            return Ok(PHYSICAL.into());
        }
        let key = (source.to_owned(), bootstrap);
        if let Some(tag) = self.registered.get(&key) {
            return Ok(tag.clone());
        }
        let (address, detour) = match source.split_once('#') {
            Some((address, detour)) => (
                address,
                Some(
                    self.detour(detour)?
                        .ok_or_else(|| invalid("unknown resolver detour"))?,
                ),
            ),
            None => (source, None),
        };
        let address = if address.contains("://") {
            address.to_owned()
        } else if let Ok(ip) = address.parse::<IpAddr>() {
            format!("udp://{}", SocketAddr::new(ip, 53))
        } else {
            format!("udp://{address}")
        };
        let url = reqwest::Url::parse(&address).map_err(|_| invalid("nameserver URL"))?;
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(invalid("nameserver credentials/fragment"));
        }
        let kind = url.scheme();
        let port = match kind {
            "udp" | "tcp" => 53,
            "tls" | "quic" => 853,
            "https" | "h3" => 443,
            _ => return Err(invalid("transport")),
        };
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| invalid("nameserver host"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let tag = format!("dns-upstream-{}", self.registered.len());
        let mut server = json!({"type":if kind=="h3" {"http3"}else{kind}, "tag":tag, "server":host, "server_port":url.port().unwrap_or(port)});
        if server["server_port"] == 0 {
            return Err(invalid("nameserver port"));
        }
        if let Some(detour) = detour {
            server["detour"] = json!(detour);
        }
        if host.parse::<IpAddr>().is_err() {
            server["domain_resolver"] = json!(if bootstrap { PHYSICAL } else { BOOTSTRAP });
        }
        if matches!(kind, "https" | "h3") {
            let mut path = if url.path().is_empty() || url.path() == "/" {
                "/dns-query".to_owned()
            } else {
                url.path().to_owned()
            };
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            server["path"] = json!(path);
        } else if !matches!(url.path(), "" | "/") || url.query().is_some() {
            return Err(invalid("non-HTTP nameserver path"));
        }
        self.registered.insert(key, tag.clone());
        self.servers.push(server);
        Ok(tag)
    }
}

pub(super) fn compile(
    root: &serde_yaml::Mapping,
    options: &VpnOptions,
    routes: &BTreeSet<String>,
) -> Result<CompiledDns, EngineError> {
    let empty = serde_yaml::Mapping::new();
    let dns = match yaml_field(root, "dns") {
        None | Some(serde_yaml::Value::Null) => &empty,
        Some(value) => value.as_mapping().ok_or_else(|| invalid("mapping"))?,
    };
    let enabled = boolean(dns, "enable", true)?;
    let mode = match yaml_field(dns, "enhanced-mode") {
        None | Some(serde_yaml::Value::Null) => "redir-host",
        Some(value) => match value.as_str() {
            Some("fake-ip") => "fake-ip",
            Some("redir-host") => "redir-host",
            _ => return Err(invalid("enhanced-mode")),
        },
    };
    let fake = enabled && mode == "fake-ip";
    let filter_mode = match yaml_field(dns, "fake-ip-filter-mode") {
        None | Some(serde_yaml::Value::Null) => "blacklist",
        Some(value) => match value.as_str() {
            Some("blacklist") => "blacklist",
            Some("whitelist") => "whitelist",
            _ => return Err(invalid("fake-ip-filter-mode")),
        },
    };
    let fake_filter = strings(yaml_field(dns, "fake-ip-filter"), "fake-ip-filter")?;
    let listen = yaml_string(dns, "listen").filter(|value| enabled && !value.is_empty());
    if listen.as_ref().is_some_and(|listen| {
        listen
            .parse::<SocketAddr>()
            .map_or(true, |address| address.port() == 0)
    }) {
        return Err(invalid("listen address"));
    }
    let mut compiler = DnsCompiler::new(routes);
    let mut primary = if yaml_field(dns, "nameserver").is_some() {
        strings(yaml_field(dns, "nameserver"), "nameserver")?
    } else {
        options.dns_servers.clone()
    };
    if !enabled {
        primary = vec!["system".into()];
    }
    let primary = compiler.normalize(primary)?;
    if primary.is_empty() {
        return Err(invalid("empty primary nameservers"));
    }
    let fallback = compiler.normalize(strings(yaml_field(dns, "fallback"), "fallback")?)?;
    let mut bootstrap = compiler.normalize(strings(
        yaml_field(dns, "default-nameserver"),
        "default-nameserver",
    )?)?;
    if bootstrap.is_empty() {
        bootstrap.push("system".into());
    }
    let bootstrap_servers = compiler.list(&bootstrap, true)?;
    compiler.servers.push(json!({"type":"paws-dns","tag":BOOTSTRAP,"servers":bootstrap_servers,"fallbacks":[],"policies":[],"resolver":PHYSICAL,"useHosts":false}));
    let primary_servers = compiler.list(&primary, false)?;
    let fallback_servers = compiler.list(&fallback, false)?;
    let proxy = compiler.normalize(strings(
        yaml_field(dns, "proxy-server-nameserver"),
        "proxy-server-nameserver",
    )?)?;
    let proxy_servers = if proxy.is_empty() {
        primary_servers.clone()
    } else {
        compiler.list(&proxy, false)?
    };
    let use_hosts = boolean(dns, "use-hosts", true)?;
    let mut hosts = BTreeMap::new();
    if let Some(value) = yaml_field(root, "hosts").filter(|value| !value.is_null()) {
        for (name, value) in value.as_mapping().ok_or_else(|| invalid("hosts map"))? {
            let name = name
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| invalid("hosts key"))?;
            let addresses = strings(Some(value), "hosts value")?;
            if addresses.is_empty() {
                return Err(invalid("empty hosts value"));
            }
            hosts.insert(name.to_owned(), addresses);
        }
    }
    compiler.servers.push(json!({"type":"paws-dns","tag":PROXY_RESOLVER,"servers":proxy_servers,"fallbacks":if proxy.is_empty(){fallback_servers.clone()}else{vec![]},"policies":[],"resolver":BOOTSTRAP,"hosts":hosts,"useHosts":use_hosts}));
    let mut nameserver_policy = BTreeMap::new();
    let mut policies = Vec::new();
    if let Some(value) = yaml_field(dns, "nameserver-policy").filter(|value| !value.is_null()) {
        for (matcher, value) in value
            .as_mapping()
            .ok_or_else(|| invalid("nameserver-policy map"))?
        {
            let matcher = matcher
                .as_str()
                .filter(|matcher| !matcher.is_empty())
                .ok_or_else(|| invalid("nameserver-policy key"))?;
            let urls = compiler.normalize(strings(Some(value), "nameserver-policy servers")?)?;
            if urls.is_empty() {
                return Err(invalid("empty nameserver-policy servers"));
            }
            let servers = compiler.list(&urls, false)?;
            policies.push(json!({"matcher":matcher,"servers":servers}));
            nameserver_policy.insert(matcher.to_owned(), urls);
        }
    }
    let mut main = json!({"type":"paws-dns","tag":PRIMARY,"servers":primary_servers,"fallbacks":fallback_servers,"policies":policies,"resolver":BOOTSTRAP,"proxyResolver":PROXY_RESOLVER,"hosts":hosts,"useHosts":use_hosts});
    let mut cache_file = None;
    if fake {
        let mut native = json!({"type":"fakeip","tag":"dns-fakeip","inet4_range":fake_range(dns,"fake-ip-range","198.18.0.1/16",false)?});
        if options.ipv6 {
            native["inet6_range"] = json!(fake_range(dns, "fake-ip-range6", "fc00::/18", true)?);
        }
        compiler.servers.push(native);
        main["fakeIP"] = json!("dns-fakeip");
        main["fakeIPFilter"] = json!(fake_filter);
        main["fakeIPFilterMode"] = json!(filter_mode);
        let profile = match yaml_field(root, "profile").filter(|value| !value.is_null()) {
            None => &empty,
            Some(value) => value
                .as_mapping()
                .ok_or_else(|| invalid("profile store-fake-ip"))?,
        };
        let legacy = boolean(profile, "store-fake-ip", false)?;
        let persist = boolean(dns, "store-fake-ip", legacy)?;
        if yaml_field(dns, "store-fake-ip").is_some()
            && yaml_field(profile, "store-fake-ip").is_some()
            && persist != legacy
        {
            return Err(invalid("conflicting store-fake-ip"));
        }
        if persist {
            cache_file =
                Some(json!({"enabled":true,"path":"paws-dns-cache.db","store_fakeip":true}));
        }
    }
    compiler.servers.push(main);
    Ok(CompiledDns {
        options: json!({"servers":compiler.servers,"final":PRIMARY,"strategy":if options.ipv6 {"prefer_ipv4"}else{"ipv4_only"},"reverse_mapping":true}),
        config: EngineDnsConfig {
            model: if fake { "fake-ip" } else { "redir-host" }.into(),
            listen,
            upstreams: primary,
            fallbacks: fallback,
            nameserver_policy,
        },
        cache_file,
    })
}

fn fake_range(
    dns: &serde_yaml::Mapping,
    key: &str,
    default: &str,
    ipv6: bool,
) -> Result<String, EngineError> {
    let value = match yaml_field(dns, key) {
        None | Some(serde_yaml::Value::Null) => default,
        Some(value) => value.as_str().ok_or_else(|| invalid(key))?,
    };
    let (address, bits) = value.split_once('/').ok_or_else(|| invalid(key))?;
    let address = address.parse::<IpAddr>().map_err(|_| invalid(key))?;
    let bits = bits.parse::<u8>().map_err(|_| invalid(key))?;
    // The native pool reserves its base/first/broadcast addresses.
    if address.is_ipv6() != ipv6 || bits > if ipv6 { 126 } else { 30 } {
        return Err(invalid(key));
    }
    Ok(value.to_owned())
}
