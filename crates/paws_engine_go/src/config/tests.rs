use super::*;
use paws_model::VpnEngine;
use serde_json::{json, Value};

fn public_services() -> EngineNetworkServices {
    let mut services = EngineNetworkServices::default();
    services.ports.mixed_enabled = true;
    services
}
fn render(source: &str) -> Value {
    let result = render_sing_box_runtime_json(
        source,
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &VpnOptions::default(),
        &public_services(),
    )
    .unwrap();
    serde_json::from_str(&result.config_json).unwrap()
}
fn server<'a>(config: &'a Value, tag: &str) -> &'a Value {
    config["dns"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["tag"] == tag)
        .unwrap()
}

#[test]
fn common_rule_compiler_is_used_by_the_real_renderer_in_source_order() {
    let source = r#"
dns: {nameserver: [192.0.2.53], nameserver-policy: {}}
rules:
  - 'DOMAIN-REGEX,^x{1,3}\.example$,DIRECT'
  - 'IP-CIDR,192.0.2.0/24,DIRECT,src,no-resolve'
  - 'IN-NAME,tun-in/mixed-in,DIRECT'
  - 'IN-USER,alice/bob,DIRECT'
  - 'MATCH,DIRECT'
"#;
    let config = render(source);
    let rules = config["route"]["rules"].as_array().unwrap();
    let rules = &rules[rules.len() - 5..];
    assert_eq!(rules[0]["domain_regex"][0], r"(?i)^x{1,3}\.example$");
    assert_eq!(rules[1]["source_ip_cidr"][0], "192.0.2.0/24");
    assert!(rules[1].get("paws_resolve").is_none());
    assert_eq!(rules[2]["inbound"], json!(["tun-in", "mixed-in"]));
    assert_eq!(rules[3]["auth_user"], json!(["alice", "bob"]));
    assert!(rules.iter().all(|rule| rule["outbound"] == "direct"));
    assert_eq!(rules[4], json!({"outbound": "direct"}));
}

#[test]
fn renderer_reports_safe_rule_type_and_reason_without_private_rule_text() {
    let error = render_sing_box_runtime_json(
        "rules: ['DSCP,private-payload,DIRECT']",
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &VpnOptions::default(),
        &public_services(),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("rule DSCP:"));
    assert!(error.contains("packet DSCP"));
    assert!(!error.contains("private-payload"));
}

#[test]
fn renderer_rejects_process_regex_that_cannot_preserve_the_native_empty_path_guard() {
    let error = render_sing_box_runtime_json(
        "rules: ['PROCESS-PATH-REGEX,.*,DIRECT']",
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &VpnOptions::default(),
        &public_services(),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("rule PROCESS-PATH-REGEX:"));
    assert!(error.contains("empty path"));
    assert!(!error.contains(".*"));
}
const SOURCE: &str = r#"
authentication: ['fixture:public:password']
skip-auth-prefixes: [192.0.2.0/24]
geodata:
  mmdb-path: '/synthetic/geo data/Country.mmdb'
  asn-path: '/synthetic/geo data/GeoLite2-ASN.mmdb'
  geosite-path: '/synthetic/geo data/geosite.dat'
dns:
  enable: true
  listen: '127.0.0.1:1053'
  nameserver: ['udp://192.0.2.53:5353', 'https://resolver.example.test/custom?mode=test#Proxy']
  fallback: ['tls://198.51.100.53:8853']
  default-nameserver: ['192.0.2.54']
  proxy-server-nameserver: ['tcp://192.0.2.55:8053']
  nameserver-policy:
    '+.example.test': ['https://policy.example.test/dns-query', Proxy]
    'geosite:cn': '192.0.2.56'
    'rule-set:local': ['quic://192.0.2.57']
  use-hosts: true
hosts:
  'fixture.example.test': ['192.0.2.20', '2001:db8::20']
  'alias.example.test': 'fixture.example.test'
proxies:
  - {name: node, type: socks5, server: proxy.example.test, port: 1080}
proxy-groups:
  - {name: Proxy, type: select, proxies: [node, DIRECT]}
rules: ['MATCH,Proxy']
"#;

#[test]
fn dns_compiler_preserves_real_primary_fallback_policy_proxy_bootstrap_and_hosts() {
    let config = render(SOURCE);
    assert_eq!(
        config["inbounds"][0]["stack"], "mixed",
        "public Meow stack enum is never sent to sing-box"
    );
    assert_eq!(config["dns"]["final"], "dns-paws");
    assert_eq!(config["dns"]["strategy"], "ipv4_only");
    assert_eq!(config["route"]["default_domain_resolver"], "dns-proxy");
    assert_eq!(config["outbounds"][0]["domain_resolver"], "dns-proxy");
    let main = server(&config, "dns-paws");
    assert_eq!(main["servers"].as_array().unwrap().len(), 2);
    let https = server(&config, main["servers"][1].as_str().unwrap());
    assert_eq!(https["type"], "https");
    assert_eq!(https["server"], "resolver.example.test");
    assert_eq!(https["path"], "/custom?mode=test");
    assert_eq!(https["detour"], "Proxy");
    assert_eq!(https["domain_resolver"], "dns-bootstrap");
    let udp = server(&config, main["servers"][0].as_str().unwrap());
    assert_eq!(udp["server_port"], 5353);
    assert_eq!(
        server(&config, main["fallbacks"][0].as_str().unwrap())["type"],
        "tls"
    );
    let policy = main["policies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|policy| policy["matcher"] == "+.example.test")
        .unwrap();
    let upstream = server(&config, policy["servers"][0].as_str().unwrap());
    assert_eq!(upstream["server"], "policy.example.test");
    assert_eq!(upstream["detour"], "Proxy");
    assert_eq!(
        main["hosts"]["alias.example.test"],
        json!(["fixture.example.test"])
    );
    assert_eq!(
        main["hosts"]["fixture.example.test"],
        json!(["192.0.2.20", "2001:db8::20"])
    );
    assert_eq!(main["useHosts"], true);
    let proxy = server(&config, "dns-proxy");
    assert_eq!(proxy["fallbacks"], json!([]));
    assert_eq!(proxy["policies"], json!([]));
    assert_eq!(
        server(&config, proxy["servers"][0].as_str().unwrap())["server_port"],
        8053
    );
    let bootstrap = server(&config, "dns-bootstrap");
    assert_eq!(
        server(&config, bootstrap["servers"][0].as_str().unwrap())["server"],
        "192.0.2.54"
    );
    assert_eq!(
        config["paws_runtime"]["dnsConfig"]["listen"],
        "127.0.0.1:1053"
    );
    assert_eq!(
        config["paws_runtime"]["dnsConfig"]["upstreams"],
        json!([
            "udp://192.0.2.53:5353",
            "https://resolver.example.test/custom?mode=test#Proxy"
        ])
    );
    assert_eq!(
        config["paws_runtime"]["dnsConfig"]["nameserverPolicy"]["+.example.test"],
        json!(["https://policy.example.test/dns-query#Proxy"])
    );
}

#[test]
fn geo_metadata_uses_only_prepared_paths_and_never_guesses_a_directory() {
    let config = render(SOURCE);
    assert_eq!(
        config["paws_runtime"]["geodataPaths"],
        json!({
            "countryMmdb":"/synthetic/geo data/Country.mmdb",
            "asnMmdb":"/synthetic/geo data/GeoLite2-ASN.mmdb",
            "geosite":"/synthetic/geo data/geosite.dat",
        })
    );
    let absent = render("rules: ['MATCH,DIRECT']\n");
    assert_eq!(
        absent["paws_runtime"]["geodataPaths"],
        json!({"countryMmdb":null,"asnMmdb":null,"geosite":null})
    );
}

#[test]
fn auth_is_public_only_and_each_private_lane_uses_its_actual_epoch_credential() {
    for engine in [VpnEngine::SingBox, VpnEngine::Mihomo] {
        let public = if engine == VpnEngine::SingBox {
            serde_json::to_string(&render(SOURCE)).unwrap()
        } else {
            render_mihomo_runtime_yaml(SOURCE, &public_services()).unwrap()
        };
        let secured = secure_controller_config(engine, &public, "synthetic-private-epoch").unwrap();
        let value: Value = if engine == VpnEngine::SingBox {
            serde_json::from_str(&secured).unwrap()
        } else {
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(&secured).unwrap())
                .unwrap()
        };
        let endpoint = controller_endpoint(engine, &secured).unwrap();
        assert_eq!(endpoint.secret.as_deref(), Some("synthetic-private-epoch"));
        if engine == VpnEngine::SingBox {
            let inbounds = value["inbounds"].as_array().unwrap();
            let public = inbounds
                .iter()
                .find(|inbound| inbound["tag"] == "mixed-in")
                .unwrap();
            let private = inbounds
                .iter()
                .find(|inbound| inbound["tag"] == "sing-box-benchmark")
                .unwrap();
            assert_eq!(
                public["users"],
                json!([{"username":"fixture","password":"public:password"}])
            );
            assert_eq!(
                public["paws_skip_auth_prefixes"],
                json!(["127.0.0.1/32", "::1/128", "192.0.2.0/24"])
            );
            assert_eq!(
                private["users"],
                json!([{"username":"paws-app","password":"synthetic-private-epoch"}])
            );
            assert!(private.get("paws_skip_auth_prefixes").is_none());
        } else {
            assert_eq!(value["authentication"], json!(["fixture:public:password"]));
            assert_eq!(value["skip-auth-prefixes"], json!(["192.0.2.0/24"]));
            assert_eq!(
                value["listeners"][0]["users"],
                json!([{"username":"paws-app","password":"synthetic-private-epoch"}])
            );
        }
    }
}

#[test]
fn invalid_auth_and_dns_shapes_fail_without_echoing_credentials_or_falling_back_to_local() {
    for source in [
        "authentication: [sensitive-invalid]\n",
        "authentication: [':sensitive-empty']\n",
        "skip-auth-prefixes: [sensitive-invalid]\n",
        "dns: {nameserver: [false]}\n",
        "dns: {nameserver: ['https://sensitive:secret@resolver.test/dns-query']}\n",
        "dns: {nameserver: ['udp://192.0.2.1:0']}\n",
        "dns: {nameserver: ['unsupported://sensitive.test']}\n",
        "dns: {nameserver: ['https://sensitive.test#unknown-group']}\n",
        "geodata: {mmdb-path: false}\n",
    ] {
        let error = match render_sing_box_runtime_json(
            source,
            RuntimeMode::Rule,
            &BTreeMap::new(),
            &VpnOptions::default(),
            &EngineNetworkServices::default(),
        ) {
            Ok(_) => panic!("invalid synthetic config was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(
            !error.contains("sensitive"),
            "configuration errors must not disclose credential/address values"
        );
    }
}

#[test]
fn missing_dns_uses_actual_requested_options_while_disabled_listener_reports_none() {
    let config = render("rules: ['MATCH,DIRECT']\n");
    assert_eq!(
        config["paws_runtime"]["dnsConfig"]["upstreams"],
        json!(VpnOptions::default().dns_servers)
    );
    assert_eq!(config["paws_runtime"]["dnsConfig"]["listen"], Value::Null);
    let disabled =
        render("dns: {enable: false,listen: '127.0.0.1:1053'}\nrules: ['MATCH,DIRECT']\n");
    assert_eq!(disabled["paws_runtime"]["dnsConfig"]["listen"], Value::Null);
    let proxy: serde_yaml::Value = serde_yaml::from_str(
        "{name: dynamic, type: socks5, server: provider.example.test, port: 1080}",
    )
    .unwrap();
    assert_eq!(
        sing_box_proxy_outbound(&proxy).unwrap()["domain_resolver"],
        "dns-proxy",
        "provider publications use the same production conversion boundary"
    );
}

#[test]
fn fake_ip_compiles_actual_native_pool_and_keeps_real_resolver_lanes() {
    let source="dns:\n  enhanced-mode: fake-ip\n  fake-ip-range: 198.18.0.1/16\n  fake-ip-range6: fc00::/64\n  fake-ip-filter-mode: whitelist\n  fake-ip-filter: ['+.fixture.invalid']\n  store-fake-ip: true\nrules: ['MATCH,DIRECT']\n";
    let output = render_sing_box_runtime_json(
        source,
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &VpnOptions {
            ipv6: true,
            ..Default::default()
        },
        &public_services(),
    )
    .unwrap();
    let config: Value = serde_json::from_str(&output.config_json).unwrap();
    let pool = server(&config, "dns-fakeip");
    assert_eq!(pool["type"], "fakeip");
    assert_eq!(pool["inet4_range"], "198.18.0.1/16");
    assert_eq!(pool["inet6_range"], "fc00::/64");
    let main = server(&config, "dns-paws");
    assert_eq!(main["fakeIP"], "dns-fakeip");
    assert_eq!(main["fakeIPFilterMode"], "whitelist");
    assert_eq!(main["fakeIPFilter"], json!(["+.fixture.invalid"]));
    for tag in ["dns-proxy", "dns-bootstrap"] {
        assert!(
            server(&config, tag).get("fakeIP").is_none(),
            "proxy/bootstrap resolution must never allocate fake addresses"
        );
    }
    assert_eq!(config["paws_runtime"]["dnsConfig"]["model"], "fake-ip");
    assert_eq!(
        config["experimental"]["cache_file"],
        json!({"enabled":true,"path":"paws-dns-cache.db","store_fakeip":true})
    );
    let memory = render("dns: {enhanced-mode: fake-ip}\nrules: ['MATCH,DIRECT']\n");
    assert!(memory["experimental"].get("cache_file").is_none());
    assert!(server(&memory, "dns-fakeip").get("inet6_range").is_none());
    let real = render("dns: {enhanced-mode: redir-host}\nrules: ['MATCH,DIRECT']\n");
    assert!(server(&real, "dns-paws").get("fakeIP").is_none());
    assert_eq!(real["paws_runtime"]["dnsConfig"]["model"], "redir-host");
}

#[test]
fn fake_ip_invalid_configuration_is_rejected_instead_of_silent_redir_host() {
    for dns in [
        "enhanced-mode: other",
        "enhanced-mode: fake-ip, fake-ip-range: 'fc00::/64'",
        "enhanced-mode: fake-ip, fake-ip-range: '198.18.0.1/32'",
        "enhanced-mode: fake-ip, fake-ip-filter-mode: other",
        "enhanced-mode: fake-ip, fake-ip-filter: [false]",
        "enhanced-mode: fake-ip, store-fake-ip: invalid",
    ] {
        let source = format!("dns: {{{dns}}}\nrules: ['MATCH,DIRECT']\n");
        assert!(
            render_sing_box_runtime_json(
                &source,
                RuntimeMode::Rule,
                &BTreeMap::new(),
                &VpnOptions::default(),
                &public_services()
            )
            .is_err(),
            "{dns}"
        );
    }
}

#[test]
fn renderer_validates_provider_sources_through_the_native_shared_schema_without_io() {
    let source = "proxy-providers:\n  remote:\n    type: http\n    url: http://127.0.0.1:1/unreachable\n    path: /nonexistent/provider.yaml\nrule-providers:\n  local:\n    type: inline\n    behavior: domain\n    payload: ['+.example.test']\nproxy-groups:\n  - {name: Provider, type: select, use: [remote]}\nrules: ['MATCH,Provider']\n";
    let config = render(source);
    assert_eq!(
        config["paws_runtime"]["proxyProviders"]["remote"]["url"],
        "http://127.0.0.1:1/unreachable"
    );
    assert_eq!(
        config["paws_runtime"]["ruleProviders"]["local"]["payload"],
        json!(["+.example.test"])
    );
    for invalid in [
        source.replace(
            "http://127.0.0.1:1/unreachable",
            "ftp://sensitive:secret@private.synthetic.test/provider",
        ),
        source.replace("type: http", "type: http\n    unknown-field: sensitive"),
        source.replace("type: http", "type: http\n    filter: sensitive"),
    ] {
        let result = render_sing_box_runtime_json(
            &invalid,
            RuntimeMode::Rule,
            &BTreeMap::new(),
            &VpnOptions::default(),
            &public_services(),
        );
        let error = match result {
            Ok(_) => panic!("unsupported provider declaration was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("provider definition"));
        assert!(!error.contains("sensitive"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("private.synthetic.test"));
    }
}
