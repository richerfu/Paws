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
pub(super) fn outbound(proxy: &serde_yaml::Value) -> Option<Value> {
    let p = proxy.as_mapping()?;
    let kind = text(p, "type").to_ascii_lowercase();
    let mut o = json!({"type":kind,"tag":yaml_string(p,"name")?,"server":yaml_string(p,"server")?,"server_port":yaml_u64(p,"port")?,"domain_resolver":"dns-local"});
    let mut tls_default = false;
    match kind.as_str() {
        "ss" | "shadowsocks" => {
            o["type"] = json!("shadowsocks");
            o["method"] = json!(yaml_string(p, "cipher")?);
            o["password"] = json!(yaml_string(p, "password")?);
            if let Some(plugin) = yaml_string(p, "plugin") {
                o["plugin"] = json!(match plugin.as_str() {
                    "obfs" | "simple-obfs" => "obfs-local",
                    other => other,
                });
                if let Some(opts) = sing_box_plugin_options(yaml_field(p, "plugin-opts")) {
                    o["plugin_opts"] = json!(opts)
                }
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
    let network = text(p, "network");
    match network.as_str() {
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
                        a.first().cloned().unwrap_or(json!("/"))
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
}
