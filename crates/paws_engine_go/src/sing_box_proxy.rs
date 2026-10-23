//! Explicit Clash-to-sing-box outbound conversion. Unknown transports fail the
//! capability audit instead of becoming a silently dropped node.
use super::*;
use serde_json::{json, Value};

fn text(p: &serde_yaml::Mapping, name: &str) -> String {
    yaml_string(p, name).unwrap_or_default()
}
fn flag(p: &serde_yaml::Mapping, name: &str) -> bool {
    yaml_field(p, name)
        .and_then(serde_yaml::Value::as_bool)
        .unwrap_or(false)
}
fn field(p: &serde_yaml::Mapping, name: &str) -> Option<Value> {
    yaml_field(p, name).and_then(|v| serde_json::to_value(v).ok())
}

// Match the pinned transport/sip003 registry and constructors. Unknown plugin
// names otherwise survive JSON compilation and fail only during native start.
fn plugin_name(name: &str) -> Option<&'static str> {
    match name {
        "obfs" | "simple-obfs" | "obfs-local" => Some("obfs-local"),
        "v2ray-plugin" => Some("v2ray-plugin"),
        _ => None,
    }
}

fn plugin_options(plugin: &str, value: Option<&serde_yaml::Value>) -> Option<String> {
    let mut options = BTreeMap::new();
    let mut remember = |key: String, value: String| {
        let key = if plugin == "obfs-local" {
            match key.as_str() {
                "mode" => "obfs".to_owned(),
                "host" => "obfs-host".to_owned(),
                _ => key,
            }
        } else {
            key
        };
        if options.get(&key).is_some_and(|old| old != &value) {
            return None;
        }
        options.insert(key, value);
        Some(())
    };
    match value {
        None | Some(serde_yaml::Value::Null) => {}
        Some(serde_yaml::Value::Mapping(values)) => {
            for (key, value) in values {
                let key = key.as_str()?.to_owned();
                let value = match value {
                    serde_yaml::Value::String(value) => value.clone(),
                    serde_yaml::Value::Number(value)
                        if plugin == "v2ray-plugin" && key == "mux" =>
                    {
                        value.to_string()
                    }
                    serde_yaml::Value::Bool(value) if plugin == "v2ray-plugin" && key == "tls" => {
                        if !value {
                            continue;
                        }
                        "1".to_owned()
                    }
                    _ => return None,
                };
                remember(key, value)?;
            }
        }
        Some(serde_yaml::Value::String(source)) => {
            // SIP003 escapes the next character; delimiters inside escaped
            // values must not turn into a second option after normalization.
            let mut chars = source.chars().peekable();
            while chars.peek().is_some() {
                let mut key = String::new();
                let mut value = String::new();
                let mut has_value = false;
                while let Some(character) = chars.next() {
                    match character {
                        '\\' => {
                            let escaped = chars.next()?;
                            if has_value {
                                value.push(escaped);
                            } else {
                                key.push(escaped);
                            }
                        }
                        ';' => break,
                        '=' if !has_value => has_value = true,
                        _ if has_value => value.push(character),
                        _ => key.push(character),
                    }
                }
                if key.is_empty() {
                    return None;
                }
                if !has_value {
                    value = "1".to_owned();
                }
                remember(key, value)?;
            }
        }
        _ => return None,
    }
    match plugin {
        "obfs-local" => {
            if options
                .keys()
                .any(|key| !matches!(key.as_str(), "obfs" | "obfs-host"))
            {
                return None;
            }
            if let Some(mode) = options.get_mut("obfs") {
                if mode.is_empty() {
                    *mode = "http".to_owned();
                }
                if !matches!(mode.as_str(), "http" | "tls") {
                    return None;
                }
            }
        }
        "v2ray-plugin" => {
            if options.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "mode" | "host" | "path" | "tls" | "cert" | "certRaw" | "mux"
                )
            }) || options
                .get("mode")
                .is_some_and(|mode| !matches!(mode.as_str(), "websocket" | "quic"))
                || options
                    .get("mux")
                    .is_some_and(|mux| mux.parse::<i64>().is_err())
            {
                return None;
            }
        }
        _ => return None,
    }
    let escape = |value: &str| {
        let mut result = String::with_capacity(value.len());
        for character in value.chars() {
            if matches!(character, '\\' | ';' | '=') {
                result.push('\\');
            }
            result.push(character);
        }
        result
    };
    Some(
        options
            .into_iter()
            .map(|(key, value)| format!("{}={}", escape(&key), escape(&value)))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

pub(super) fn outbound(proxy: &serde_yaml::Value) -> Option<Value> {
    let p = proxy.as_mapping()?;
    let kind = text(p, "type").to_ascii_lowercase();
    let plugin = match yaml_field(p, "plugin") {
        None | Some(serde_yaml::Value::Null) => None,
        Some(serde_yaml::Value::String(name)) if name.is_empty() => None,
        Some(serde_yaml::Value::String(name)) => Some(plugin_name(name)?),
        _ => return None,
    };
    let network = match yaml_field(p, "network") {
        None | Some(serde_yaml::Value::Null) => "",
        Some(value) => value.as_str()?,
    };
    // Only these native outbound option types own V2RayTransportOptions.
    // Adding `transport` to SS/SOCKS/HTTP/QUIC protocols is not equivalent.
    if (!matches!(network, "" | "tcp") && !matches!(kind.as_str(), "vless" | "vmess" | "trojan"))
        || (plugin.is_some() && !matches!(kind.as_str(), "ss" | "shadowsocks"))
        || (plugin.is_none() && yaml_field(p, "plugin-opts").is_some_and(|value| !value.is_null()))
        || (flag(p, "tls") && matches!(kind.as_str(), "ss" | "shadowsocks" | "socks" | "socks5"))
    {
        return None;
    }
    if matches!(
        kind.as_str(),
        "direct" | "reject" | "reject-drop" | "compatible"
    ) {
        return Some(
            json!({"type":if matches!(kind.as_str(), "reject" | "reject-drop") { "block" } else { "direct" }, "tag":yaml_string(p,"name")?}),
        );
    }
    let port = u16::try_from(yaml_u64(p, "port")?)
        .ok()
        .filter(|port| *port != 0)?;
    let mut o = json!({"type":kind,"tag":yaml_string(p,"name")?,"server":yaml_string(p,"server")?,"server_port":port,"domain_resolver":"dns-local"});
    let mut tls_default = false;
    match kind.as_str() {
        "ss" | "shadowsocks" => {
            o["type"] = json!("shadowsocks");
            o["method"] = json!(yaml_string(p, "cipher")?);
            o["password"] = json!(yaml_string(p, "password")?);
            if let Some(plugin) = plugin {
                o["plugin"] = json!(plugin);
                o["plugin_opts"] = json!(plugin_options(plugin, yaml_field(p, "plugin-opts"))?);
            }
        }
        "socks" | "socks5" | "http" => {
            if kind != "http" {
                o["type"] = json!("socks")
            }
            for key in ["username", "password"] {
                if let Some(v) = yaml_string(p, key) {
                    o[key] = json!(v)
                }
            }
        }
        "trojan" => {
            o["password"] = json!(yaml_string(p, "password")?);
            tls_default = true
        }
        "vless" => {
            o["uuid"] = json!(yaml_string(p, "uuid")?);
            if let Some(flow) = yaml_string(p, "flow") {
                if !flow.is_empty() {
                    o["flow"] = json!(flow)
                }
            }
        }
        "vmess" => {
            o["uuid"] = json!(yaml_string(p, "uuid")?);
            o["security"] = json!(yaml_string(p, "cipher").unwrap_or_else(|| "auto".into()));
            o["alter_id"] = json!(yaml_u64(p, "alterId").unwrap_or(0));
        }
        "hysteria2" | "hy2" => {
            o["type"] = json!("hysteria2");
            o["password"] = json!(yaml_string(p, "password")?);
            tls_default = true;
            if let Some(obfs) = yaml_string(p, "obfs") {
                o["obfs"] = json!({"type":obfs,"password":text(p,"obfs-password")})
            }
            for (from, to) in [("up", "up_mbps"), ("down", "down_mbps")] {
                if let Some(v) = yaml_u64(p, from) {
                    o[to] = json!(v)
                }
            }
        }
        "tuic" => {
            o["uuid"] = json!(yaml_string(p, "uuid")?);
            o["password"] = json!(yaml_string(p, "password")?);
            tls_default = true;
            for (from, to) in [
                ("congestion-controller", "congestion_control"),
                ("udp-relay-mode", "udp_relay_mode"),
            ] {
                if let Some(v) = yaml_string(p, from) {
                    o[to] = json!(v)
                }
            }
            o["zero_rtt_handshake"] = json!(flag(p, "reduce-rtt"));
        }
        _ => return None,
    }
    if yaml_field(p, "udp").and_then(serde_yaml::Value::as_bool) == Some(false)
        && matches!(
            kind.as_str(),
            "ss" | "shadowsocks" | "socks" | "socks5" | "vmess" | "trojan" | "vless"
        )
    {
        o["network"] = json!("tcp")
    }
    if tls_default || flag(p, "tls") {
        let mut tls = json!({"enabled":true,"insecure":flag(p,"skip-cert-verify")});
        let sni = yaml_string(p, "sni").or_else(|| yaml_string(p, "servername"));
        if let Some(sni) = sni {
            tls["server_name"] = json!(sni)
        }
        if let Some(alpn) = field(p, "alpn") {
            tls["alpn"] = alpn
        }
        if let Some(fp) = yaml_string(p, "client-fingerprint") {
            if !fp.is_empty() {
                tls["utls"] = json!({"enabled":true,"fingerprint":fp})
            }
        }
        if let Some(reality) = yaml_field(p, "reality-opts").and_then(serde_yaml::Value::as_mapping)
        {
            tls["reality"] = json!({"enabled":true,"public_key":text(reality,"public-key"),"short_id":text(reality,"short-id")})
        }
        o["tls"] = tls;
    }
    match network {
        "" | "tcp" => {}
        "ws" => {
            let opts = yaml_field(p, "ws-opts").and_then(serde_yaml::Value::as_mapping);
            let mut transport = json!({"type":"ws"});
            if let Some(opts) = opts {
                transport["path"] = json!(text(opts, "path"));
                if let Some(headers) = field(opts, "headers") {
                    transport["headers"] = headers
                }
                if let Some(n) = yaml_u64(opts, "max-early-data") {
                    transport["max_early_data"] = json!(n)
                }
                if let Some(name) = yaml_string(opts, "early-data-header-name") {
                    transport["early_data_header_name"] = json!(name)
                }
            }
            o["transport"] = transport;
        }
        "grpc" => {
            let opts = yaml_field(p, "grpc-opts").and_then(serde_yaml::Value::as_mapping);
            o["transport"] = json!({"type":"grpc","service_name":opts.map(|p|text(p,"grpc-service-name")).unwrap_or_default()});
        }
        "http" | "h2" => {
            let opts = yaml_field(
                p,
                if network == "h2" {
                    "h2-opts"
                } else {
                    "http-opts"
                },
            )
            .and_then(serde_yaml::Value::as_mapping);
            let mut transport = json!({"type":"http"});
            if let Some(opts) = opts {
                if let Some(v) = field(opts, "host") {
                    transport["host"] = v
                }
                if let Some(v) = field(opts, "path") {
                    transport["path"] = if let Some(a) = v.as_array() {
                        if a.len() != 1 {
                            return None;
                        }
                        a[0].clone()
                    } else {
                        v
                    }
                }
            }
            o["transport"] = transport;
        }
        _ => return None,
    }
    Some(o)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_standard_protocols_translate() {
        for (kind, extra) in [
            (
                "vless",
                "uuid: id\ntls: true\nnetwork: ws\nws-opts: {path: /socket}",
            ),
            ("vmess", "uuid: id\ncipher: auto\nalterId: 0"),
            ("trojan", "password: test"),
            (
                "hysteria2",
                "password: test\nobfs: salamander\nobfs-password: test",
            ),
            ("tuic", "uuid: id\npassword: test"),
        ] {
            let raw: serde_yaml::Value = serde_yaml::from_str(&format!(
                "name: example\ntype: {kind}\nserver: example.test\nport: 443\n{extra}"
            ))
            .unwrap();
            let actual = outbound(&raw).unwrap();
            assert_eq!(actual["tag"], "example");
            assert_eq!(actual["server_port"], 443);
            if kind == "vless" {
                assert_eq!(actual["transport"]["type"], "ws");
                assert_eq!(actual["tls"]["enabled"], true)
            }
        }
    }
    #[test]
    fn unknown_transport_cannot_be_omitted_silently() {
        let p = serde_yaml::from_str(
            "name: bad\ntype: vless\nserver: example.test\nport: 443\nuuid: id\nnetwork: unknown",
        )
        .unwrap();
        assert!(outbound(&p).is_none());
    }

    #[test]
    fn native_plugin_registry_and_protocol_transport_ownership_are_enforced() {
        for extra in [
            "plugin: missing-native-plugin",
            "plugin: obfs\nplugin-opts: {mode: udp}",
            "plugin: obfs\nplugin-opts: {mode: http, mux: true}",
            "plugin: obfs\nplugin-opts: {mode: http, obfs: tls}",
            "plugin: obfs\nplugin-opts: 'obfs=http;obfs=tls'",
            "plugin: obfs\nplugin-opts: 'obfs-host=trailing\\'",
            "plugin: v2ray-plugin\nplugin-opts: {mode: grpc}",
            "plugin: v2ray-plugin\nplugin-opts: {mode: websocket, mux: invalid}",
            "plugin-opts: {mode: tls}",
            "network: ws",
            "tls: true",
        ] {
            let proxy = serde_yaml::from_str(&format!("name: invalid\ntype: ss\nserver: example.test\nport: 443\ncipher: aes-128-gcm\npassword: secret\n{extra}\n")).unwrap();
            assert!(
                outbound(&proxy).is_none(),
                "unsupported SS field must not survive compilation: {extra}"
            );
        }
        for kind in ["socks5", "http", "hysteria2", "tuic"] {
            let proxy = serde_yaml::from_str(&format!("name: invalid\ntype: {kind}\nserver: example.test\nport: 443\nuuid: id\npassword: secret\nnetwork: ws\n")).unwrap();
            assert!(
                outbound(&proxy).is_none(),
                "{kind} does not own native V2RayTransportOptions"
            );
        }
        for kind in ["trojan", "vless", "vmess"] {
            let proxy = serde_yaml::from_str(&format!("name: transport\ntype: {kind}\nserver: example.test\nport: 443\nuuid: id\npassword: secret\nnetwork: grpc\ngrpc-opts: {{grpc-service-name: svc}}\n")).unwrap();
            assert_eq!(
                outbound(&proxy).unwrap()["transport"]["service_name"],
                "svc"
            );
        }
    }

    #[test]
    fn plugin_options_preserve_native_meaning_and_escape_delimiters() {
        let obfs: serde_yaml::Value =
            serde_yaml::from_str("{mode: tls, host: 'semi;equal=back\\slash'}").unwrap();
        let encoded = plugin_options("obfs-local", Some(&obfs)).unwrap();
        assert_eq!(encoded, r"obfs=tls;obfs-host=semi\;equal\=back\\slash");
        assert_eq!(
            plugin_options(
                "obfs-local",
                Some(&serde_yaml::Value::String(encoded.clone()))
            ),
            Some(encoded)
        );
        let v2ray: serde_yaml::Value = serde_yaml::from_str(
            "{mode: websocket, host: cdn.example.test, path: /socket, tls: true, mux: 2}",
        )
        .unwrap();
        let encoded = plugin_options("v2ray-plugin", Some(&v2ray)).unwrap();
        assert_eq!(
            encoded,
            "host=cdn.example.test;mode=websocket;mux=2;path=/socket;tls=1"
        );
        assert!(
            !encoded.contains("obfs="),
            "v2ray mode must not be rewritten as obfs"
        );
        let disabled: serde_yaml::Value = serde_yaml::from_str("{tls: false}").unwrap();
        assert_eq!(
            plugin_options("v2ray-plugin", Some(&disabled)),
            Some(String::new())
        );
    }

    #[test]
    fn native_single_http_path_is_not_a_silent_multi_path_projection() {
        let source = "name: transport\ntype: vmess\nserver: example.test\nport: 443\nuuid: id\nnetwork: http\nhttp-opts: {path: [/one, /two]}\n";
        assert!(outbound(&serde_yaml::from_str(source).unwrap()).is_none());
        let single = source.replace("[/one, /two]", "[/one]");
        assert_eq!(
            outbound(&serde_yaml::from_str(&single).unwrap()).unwrap()["transport"]["path"],
            "/one"
        );
    }
}
