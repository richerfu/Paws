//! Diagnostic exports use an allowlist: no raw YAML, URLs, passwords or log text.
use super::*;
use serde_json::{json, Value};

impl CoreHandle {
    pub fn export_portable_backup(&self) -> Result<String, PawsError> {
        let state = self.lock_state()?;
        serde_json::to_string_pretty(&state.profiles.portable_backup()?)
            .map_err(|e| PawsError::Core(e.to_string()))
    }

    pub async fn restore_portable_backup(&self, text: &str) -> Result<usize, PawsError> {
        if text.len() > 20 * 1024 * 1024 {
            return Err(PawsError::Core("Backup too large".into()));
        }
        let backup: paws_profile::PortableBackup =
            serde_json::from_str(text).map_err(|e| PawsError::Core(e.to_string()))?;
        if backup.format != "TinyPaws-backup-v1" || backup.profiles.len() > 50 {
            return Err(PawsError::Core("Unsupported backup".into()));
        }
        for p in &backup.profiles {
            let mut effective: serde_yaml::Value =
                serde_yaml::from_str(&p.content).map_err(|e| PawsError::Core(e.to_string()))?;
            let root = effective
                .as_mapping_mut()
                .ok_or_else(|| PawsError::Core("Backup profile root must be a map".into()))?;
            for (key, value) in &p.overrides {
                if !["dns", "paws", "tun"].contains(&key.as_str()) {
                    return Err(PawsError::Core("Invalid override section".into()));
                }
                let patch = value
                    .as_mapping()
                    .ok_or_else(|| PawsError::Core("Override must be a map".into()))?;
                let mut base = root
                    .get(serde_yaml::Value::String(key.clone()))
                    .and_then(serde_yaml::Value::as_mapping)
                    .cloned()
                    .unwrap_or_default();
                for (k, v) in patch {
                    if v.is_null() {
                        base.remove(k);
                    } else {
                        base.insert(k.clone(), v.clone());
                    }
                }
                root.insert(
                    serde_yaml::Value::String(key.clone()),
                    serde_yaml::Value::Mapping(base),
                );
            }
            let effective =
                serde_yaml::to_string(&effective).map_err(|e| PawsError::Core(e.to_string()))?;
            self.validate_profile_content(&effective).await?;
            if !p.rules.is_empty() {
                paws_profile::parse_imported_rule_lines(&p.rules.join("\n"))?;
            }
        }
        let _guard = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        let count = state.profiles.restore_portable_backup(backup)?;
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(count)
    }

    pub fn diagnostic_report(&self) -> Result<String, PawsError> {
        let snapshot = self.snapshot()?;
        let preflight = self.configuration_preflight()?;
        let recent: Vec<_> = snapshot.request_history.iter().rev().take(80).map(|r| json!({
            "id":r.id, "domain":r.domain, "domainSource":r.domain_source, "dnsResolver":"not-recorded-for-this-flow", "destinationIp":r.destination_ip,
            "port":r.destination_port, "network":r.network, "rule":r.rule,
            "proxy":r.proxy, "uploadBytes":r.upload_bytes, "downloadBytes":r.download_bytes,
            "active":r.active,
            "endReason":snapshot.logs.iter().rev().find(|l| l.message.contains(&r.id) && (l.message.contains("BrokenPipe") || l.message.contains("UnexpectedEof"))).map(|l| {
                if l.message.contains("BrokenPipe") && l.message.contains("proxy-to-client") && l.message.contains("write") { "client-side-write-broken-pipe" }
                else if l.message.contains("UnexpectedEof") { "unexpected-eof-see-local-log" }
                else { "broken-pipe-direction-unconfirmed" }
            }).unwrap_or(if r.active { "active" } else { "ended-cause-not-recorded" }),
            "observation": if r.download_bytes > 0 { "return-traffic-observed" } else { "no-return-observed-not-proof-of-failure" }
        })).collect();
        let log_counts = json!({
            "unexpectedEof":snapshot.logs.iter().filter(|l| l.message.contains("UnexpectedEof") || l.message.contains("unexpected end of file")).count(),
            "brokenPipe":snapshot.logs.iter().filter(|l| l.message.contains("BrokenPipe") || l.message.contains("broken pipe")).count(),
            "domainRecovered":snapshot.logs.iter().filter(|l| l.message.contains("unmapped flow recovered") && !l.message.contains("<not available>")).count(),
            "scope":"bounded current log window; not lifetime totals"
        });
        let lifecycle = {
            let state = self.lock_state()?;
            let path = state.profiles.root().join("lifecycle.json");
            fs::metadata(&path)
                .ok()
                .filter(|m| m.len() <= 32768)
                .and_then(|_| fs::read_to_string(path).ok())
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        };
        let (system_exits, vpn_system_exits, process_summary, process_samples) = {
            let root = { self.lock_state()?.profiles.root().to_path_buf() };
            let samples: Vec<crate::process_evidence::ProcessSample> =
                crate::process_evidence::read_bounded(
                    &root.join("vpn-process-evidence.json"),
                    65536,
                )
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            (
                safe_exit_records(&root.join("system-exits.json")),
                safe_exit_records(&root.join("system-exits-vpn.json")),
                crate::process_evidence::summarize(&samples),
                samples.into_iter().rev().take(120).collect::<Vec<_>>(),
            )
        };
        serde_json::to_string_pretty(&json!({
            "product":"Paws", "version":APP_VERSION, "observedAtUnixNanos":snapshot.observed_at_unix_nanos.to_string(),
            "session":snapshot.vpn_session_id, "vpnLifecycle":snapshot.vpn_lifecycle,
            "vpnEstablished":snapshot.vpn_running, "networkProtected":snapshot.network_protected,
            "mode":snapshot.mode,
            "dns":{"handledPackets":snapshot.dns.handled_packets,"cacheHits":snapshot.dns.cache_hits,"cacheMisses":snapshot.dns.cache_misses,
                "note":"Counters show activity, not independent DNS health verification"},
            "physicalNetwork":crate::network_binding::physical_network_snapshot(),
            "engineDecision":self.engine_decision(&to_json(&snapshot.vpn_options)?)?,
            "engineEvidence":safe_engine_evidence(&self.lock_state()?.profiles.root().to_path_buf()),
            "preflight":preflight, "recentConnections":recent, "logCounts":log_counts,
            "lifecycle":lifecycle,
            "systemExitEvidence":{"ui":system_exits,"vpn":vpn_system_exits},
            "vpnProcessSummary":process_summary,
            "vpnProcessEvidence":process_samples,
            "limits":["Probe success does not prove every app works", "TCP hostname recovery does not cover all ECH/QUIC traffic", "An unclean exit alone does not identify SIGKILL sender"],
            "privacy":"Subscription URLs, credentials, raw config and raw logs excluded. Destination domains/IPs remain for troubleshooting."
        })).map_err(|e| PawsError::Core(e.to_string()))
    }

    pub fn configuration_preflight(&self) -> Result<Value, PawsError> {
        let state = self.lock_state()?;
        let Some(id) = state.profiles.active_profile() else {
            return Ok(json!({"status":"no-profile"}));
        };
        let profile = state.profiles.profile(id)?;
        let raw = state.profiles.raw_yaml(id)?;
        let source = fs::read_to_string(state.profiles.root().join(&profile.raw_yaml_path))
            .map_err(|e| PawsError::Core(e.to_string()))?;
        let source: serde_yaml::Value =
            serde_yaml::from_str(&source).map_err(|e| PawsError::Core(e.to_string()))?;
        let runtime = state
            .profiles
            .build_runtime_yaml(id, state.mode, &state.vpn_options)?;
        let runtime: serde_yaml::Value =
            serde_yaml::from_str(&runtime).map_err(|e| PawsError::Core(e.to_string()))?;
        let mut paths = Vec::new();
        collect_changed_paths("", &source, &runtime, &mut paths);
        let mut warnings = Vec::<String>::new();
        if let Some(dns) = source.get("dns") {
            if let Some(list) = dns
                .get("proxy-server-nameserver")
                .and_then(serde_yaml::Value::as_sequence)
            {
                if list
                    .iter()
                    .filter_map(serde_yaml::Value::as_str)
                    .any(|s| s.contains("127.0.0.1") || s.contains("localhost"))
                {
                    warnings.push("代理节点 DNS 含本地地址：运行配置会处理指向原监听端口的自引用；请检查其他本地解析服务是否真实存在。".into());
                }
            }
        }
        if source.get("sniffer").is_some() {
            warnings.push(
                "订阅自带 sniffer：使用其配置；通用无映射域名兜底并非强制覆盖所有嗅探设置。".into(),
            );
        }
        if !state.vpn_options.dns_hijacking {
            warnings.push("DNS 劫持关闭，应用自带 DNS/缓存 IP 可能绕过域名映射。".into());
        }
        if state.vpn_options.ipv6 {
            warnings.push("IPv6 已启用，需要验证节点、DNS 与当前网络的 IPv6 路径。".into());
        }
        let validated = paws_profile::validate_profile_app_config(&raw).is_ok();
        Ok(
            json!({"status":if validated {"app-fields-valid"}else{"invalid"},
            "layers":["subscription source", "persistent user overrides", "Harmony runtime adaptation"],
            "overrideSections":profile.user_overrides.keys().collect::<Vec<_>>(),
            "changedPaths":paths, "warnings":warnings,
            "note":"Only field paths are shown; unknown fields are not certified compatible by this check."}),
        )
    }
}

fn collect_changed_paths(
    prefix: &str,
    before: &serde_yaml::Value,
    after: &serde_yaml::Value,
    paths: &mut Vec<String>,
) {
    if paths.len() >= 128 || before == after {
        return;
    }
    if let (Some(a), Some(b)) = (before.as_mapping(), after.as_mapping()) {
        let keys: BTreeSet<_> = a
            .keys()
            .chain(b.keys())
            .filter_map(|k| k.as_str())
            .collect();
        for key in keys {
            // Do not disclose node names, provider identities or credentials as paths.
            let p = if prefix.is_empty() {
                key.to_owned()
            } else {
                format!("{prefix}.{key}")
            };
            if [
                "proxies",
                "proxy-groups",
                "proxy-providers",
                "rule-providers",
                "nameserver-policy",
            ]
            .contains(&key)
            {
                if a.get(key) != b.get(key) {
                    paths.push(p);
                }
            } else {
                collect_changed_paths(
                    &p,
                    a.get(key).unwrap_or(&serde_yaml::Value::Null),
                    b.get(key).unwrap_or(&serde_yaml::Value::Null),
                    paths,
                );
            }
        }
    } else {
        paths.push(prefix.into());
    }
}

fn safe_exit_records(path: &std::path::Path) -> Vec<Value> {
    let records: Vec<Value> = crate::process_evidence::read_bounded(path, 65536)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    records
        .into_iter()
        .rev()
        .take(64)
        .filter_map(|record| {
            let source = record.as_object()?;
            let mut safe = serde_json::Map::new();
            for key in [
                "receivedAt",
                "eventTime",
                "event",
                "reason",
                "subReason",
                "killId",
                "rss",
                "pss",
                "uid",
                "importance",
                "desc",
                "process",
                "pid",
                "foreground",
            ] {
                if let Some(value) = source.get(key) {
                    match value {
                        Value::String(text) => {
                            safe.insert(
                                key.into(),
                                Value::String(text.chars().take(512).collect()),
                            );
                        }
                        Value::Number(_) | Value::Bool(_) => {
                            safe.insert(key.into(), value.clone());
                        }
                        _ => {}
                    }
                }
            }
            Some(Value::Object(safe))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diff_never_exports_values_or_provider_keys() {
        let a: serde_yaml::Value = serde_yaml::from_str(
            "secret: old\nproxy-providers:\n  token-secret: {url: secret-url}\n",
        )
        .unwrap();
        let b: serde_yaml::Value =
            serde_yaml::from_str("secret: new\nproxy-providers: {}\n").unwrap();
        let mut out = Vec::new();
        collect_changed_paths("", &a, &b, &mut out);
        assert_eq!(out, vec!["proxy-providers", "secret"]);
    }

    #[test]
    fn system_evidence_export_is_allowlisted_and_bounded() {
        let path = std::env::temp_dir().join(format!("tinypaws-exits-{}.json", std::process::id()));
        let record = json!({"event":"APP_KILLED","reason":"Memory Pressure","password":"private-marker", "rawLog":"private-marker", "process":{"unexpected":"private-marker"}});
        fs::write(&path, serde_json::to_vec(&vec![record; 70]).unwrap()).unwrap();
        let result = safe_exit_records(&path);
        assert_eq!(result.len(), 64);
        assert_eq!(result[0]["reason"], "Memory Pressure");
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("private-marker"));
        fs::remove_file(path).unwrap();
    }
}

fn safe_engine_evidence(root: &Path) -> Value {
    let read = |name: &str| {
        process_evidence::read_bounded(&root.join(name), 8192)
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or(Value::Null)
    };
    let session = read("engine-session.json");
    let health = read("engine-startup-health.json");
    let mut metadata = serde_json::Map::new();
    for key in ["pid", "physicalNetId", "ingressPort", "startedAt"] {
        if let Some(n) = session[key].as_u64() {
            metadata.insert(key.into(), json!(n));
        }
    }
    for key in ["profileFingerprint", "nodeFingerprint", "buildFingerprint"] {
        if let Some(s) = session[key]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            metadata.insert(key.into(), json!(s));
        }
    }
    if let Some(s) = session["engine"]
        .as_str()
        .filter(|s| matches!(*s, "mihomo" | "sing-box" | "meow"))
    {
        metadata.insert("engine".into(), json!(s));
    }
    if let Some(s) = session["mode"]
        .as_str()
        .filter(|s| matches!(*s, "rule" | "global" | "direct"))
    {
        metadata.insert("mode".into(), json!(s));
    }
    let mut probe = serde_json::Map::new();
    for key in ["elapsedMs", "httpsStatus", "applicationReturnBytes"] {
        if let Some(n) = health[key].as_u64() {
            probe.insert(key.into(), json!(n));
        }
    }
    for key in ["success", "rollbackToBaseline"] {
        if let Some(b) = health[key].as_bool() {
            probe.insert(key.into(), json!(b));
        }
    }
    if let Some(s) = health["stage"].as_str().filter(|s| {
        matches!(
            *s,
            "return-traffic"
                | "timeout"
                | "dns-or-node-or-tls"
                | "https"
                | "https-status-or-return-traffic"
                | "response-too-large"
                | "proxy-configuration"
                | "probe-configuration"
        )
    }) {
        probe.insert("stage".into(), json!(s));
    }
    for (key, statuses) in [
        (
            "dns",
            &[
                "selected-node-unresolved",
                "literal-no-dns-required",
                "physical-bootstrap-available",
                "physical-bootstrap-failed-or-timed-out",
            ][..],
        ),
        (
            "nodeTcp",
            &[
                "not-tested",
                "not-applicable-udp-protocol",
                "reachable",
                "failed-or-timed-out",
            ][..],
        ),
    ] {
        let mut stage = serde_json::Map::new();
        if let Some(status) = health[key]["status"]
            .as_str()
            .filter(|s| statuses.contains(s))
        {
            stage.insert("status".into(), json!(status));
        }
        if let Some(flag) = health[key]["independentProbe"].as_bool() {
            stage.insert("independentProbe".into(), json!(flag));
        }
        for field in ["elapsedMs", "addressCount"] {
            if let Some(n) = health[key][field].as_u64() {
                stage.insert(field.into(), json!(n));
            }
        }
        probe.insert(key.into(), json!(stage));
    }
    // Export only the bridge's structured byte counts and direction events,
    // never arbitrary native log lines or destination identifiers.
    let raw = process_evidence::read_bounded(&root.join("sing-box/last-session.log"), 256 * 1024)
        .unwrap_or_default();
    let mut events = Vec::new();
    for line in raw
        .lines()
        .rev()
        .filter(|l| l.contains("obfs-session "))
        .take(80)
    {
        let mut event = serde_json::Map::new();
        for token in line.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            match key {
                "id" | "sent" | "received" | "writeBytes" => {
                    if let Ok(n) = value.parse::<u64>() {
                        event.insert(key.into(), json!(n));
                    }
                }
                "peerEnded" => {
                    if let Ok(b) = value.parse::<bool>() {
                        event.insert(key.into(), json!(b));
                    }
                }
                "direction" if matches!(value, "client-to-proxy" | "proxy-to-client") => {
                    event.insert(key.into(), json!(value));
                }
                "event"
                    if matches!(
                        value,
                        "peer-fin"
                            | "peer-reset"
                            | "io-error"
                            | "first-write"
                            | "write-error"
                            | "local-close"
                    ) =>
                {
                    event.insert(key.into(), json!(value));
                }
                _ => {}
            }
        }
        if !event.is_empty() {
            events.push(Value::Object(event));
        }
    }
    json!({"session":metadata,"startupProbe":probe,"startupProbeMatchesSession":session["session"].as_str().is_some() && session["session"]==health["session"],"lastStoppedSingBoxEvents":events,"limits":["Physical bootstrap DNS and node TCP are separate probes; proxy authentication is confirmed by successful HTTPS return traffic","Stopped-session events are not attributed to the current session"]})
}

#[cfg(test)]
mod engine_evidence_tests {
    use super::*;
    #[test]
    fn engine_evidence_never_exports_unrecognized_log_or_metadata_fields() {
        let root = std::env::temp_dir().join(format!("paws-safe-engine-{}", now_unix_nanos()));
        fs::create_dir_all(root.join("sing-box")).unwrap();
        fs::write(root.join("engine-session.json"),r#"{"engine":"sing-box","pid":42,"password":"secret-marker","nodeFingerprint":"secret-marker"}"#).unwrap();
        fs::write(root.join("sing-box/last-session.log"),"INFO private secret-marker\nINFO obfs-session id=7 event=peer-fin sent=241 received=0 password=secret-marker\n").unwrap();
        let report = safe_engine_evidence(&root);
        assert_eq!(report["session"]["pid"], 42);
        assert_eq!(report["lastStoppedSingBoxEvents"][0]["received"], 0);
        assert!(!report.to_string().contains("secret-marker"));
        fs::remove_dir_all(root).unwrap();
    }
}
