//! Capability decisions and version/profile-scoped health records.
use super::*;
use sha2::{Digest, Sha256};

pub const fn benchmark_port(engine: VpnEngine) -> u16 {
    match engine {
        VpnEngine::Meow => 18491,
        VpnEngine::SingBox => 18492,
        _ => 18490,
    }
}
fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn build_fingerprint() -> &'static str {
    option_env!("PAWS_ENGINE_BUILD_ID").unwrap_or("development-unattested")
}
pub(super) fn selected_node(state: &CoreState) -> String {
    let selections = state
        .profiles
        .active_profile()
        .and_then(|id| state.profiles.selected_proxies(id).ok())
        .unwrap_or_default();
    selected_node_from_selections(&selections)
}
fn selected_node_from_selections(selections: &BTreeMap<String, String>) -> String {
    let mut node = selections.get("GLOBAL").cloned().unwrap_or_default();
    for _ in 0..8 {
        if let Some(next) = selections.get(&node).filter(|next| **next != node) {
            node = next.clone()
        } else {
            break;
        }
    }
    node
}

struct DiagnosticRuntime {
    yaml: String,
    peer_fingerprint: Option<String>,
    peer_address_fingerprint: Option<String>,
}
pub(super) fn pin_benchmark_runtime_yaml(
    yaml: &str,
    selections: &BTreeMap<String, String>,
) -> Result<String, PawsError> {
    Ok(compiled_diagnostic_runtime(yaml, selections)?.yaml)
}
fn compiled_diagnostic_runtime(
    yaml: &str,
    selections: &BTreeMap<String, String>,
) -> Result<DiagnosticRuntime, PawsError> {
    diagnostic_runtime(
        yaml,
        selections,
        option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC") == Some("1"),
        option_env!("PAWS_BENCH_PEER_IP"),
        option_env!("PAWS_BENCH_NODE_FINGERPRINT"),
    )
}
fn diagnostic_pin_error(message: &str) -> PawsError {
    PawsError::Core(format!("diagnostic peer pin: {message}"))
}
fn global_peer_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(a == 0
                || ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (18..=19).contains(&b))
                || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10))
        }
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            // Restrict diagnostic peers to allocated global unicast space.
            // Exclude protocol assignments, 6to4 and documentation ranges.
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && s[1] < 0x200)
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && s[1] == 0xdb8)
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}
fn explicit_http_obfs_host(proxy: &serde_yaml::Mapping) -> Result<String, PawsError> {
    if !matches!(
        yaml_string(proxy, "plugin").as_deref(),
        Some("obfs" | "simple-obfs")
    ) {
        return Err(diagnostic_pin_error("requires built-in HTTP obfs"));
    }
    let mut mode = None;
    let mut host = None;
    let mut remember = |key: &str, value: &str| -> Result<(), PawsError> {
        let target = match key {
            "mode" | "obfs" => &mut mode,
            "host" | "obfs-host" => &mut host,
            _ => return Ok(()),
        };
        if target.as_deref().is_some_and(|old| old != value) {
            return Err(diagnostic_pin_error("conflicting obfs option aliases"));
        }
        *target = Some(value.to_owned());
        Ok(())
    };
    match yaml_field(proxy, "plugin-opts") {
        Some(serde_yaml::Value::Mapping(options)) => {
            for (key, value) in options {
                let Some(key) = key.as_str() else { continue };
                if matches!(key, "mode" | "obfs" | "host" | "obfs-host") {
                    remember(
                        key,
                        value.as_str().ok_or_else(|| {
                            diagnostic_pin_error("obfs mode and host must be strings")
                        })?,
                    )?;
                }
            }
        }
        Some(serde_yaml::Value::String(options)) => {
            for option in options.split(';') {
                // Meow trims values; SIP003 preserves whitespace and unescapes
                // backslashes. Only admit the representation both interpret alike.
                if option != option.trim() || option.contains('\\') {
                    return Err(diagnostic_pin_error(
                        "requires canonical unescaped obfs options",
                    ));
                }
                if let Some((key, value)) = option.split_once('=') {
                    if key != key.trim() || value != value.trim() {
                        return Err(diagnostic_pin_error(
                            "requires canonical obfs option spacing",
                        ));
                    }
                    if matches!(key, "mode" | "host") {
                        return Err(diagnostic_pin_error(
                            "string options require canonical obfs and obfs-host keys",
                        ));
                    }
                    remember(key, value)?;
                } else if matches!(option, "obfs" | "obfs-host" | "mode" | "host") {
                    return Err(diagnostic_pin_error(
                        "obfs mode and host require explicit values",
                    ));
                }
            }
        }
        _ => return Err(diagnostic_pin_error("requires explicit HTTP obfs options")),
    }
    if mode.as_deref() != Some("http") {
        return Err(diagnostic_pin_error("requires explicit HTTP obfs mode"));
    }
    let host = host.ok_or_else(|| diagnostic_pin_error("requires explicit obfs host"))?;
    if host.is_empty()
        || host
            .bytes()
            .any(|b| b <= b' ' || b >= 127 || matches!(b, b';' | b'\\' | b'='))
    {
        return Err(diagnostic_pin_error("requires a nonempty valid obfs host"));
    }
    Ok(host)
}
fn diagnostic_runtime(
    yaml: &str,
    selections: &BTreeMap<String, String>,
    raw_diagnostic: bool,
    peer_ip: Option<&str>,
    node_fingerprint: Option<&str>,
) -> Result<DiagnosticRuntime, PawsError> {
    let unchanged = || DiagnosticRuntime {
        yaml: yaml.to_owned(),
        peer_fingerprint: None,
        peer_address_fingerprint: None,
    };
    let (Some(peer_ip), Some(node_fingerprint)) = (peer_ip, node_fingerprint) else {
        return Ok(unchanged());
    };
    if !raw_diagnostic {
        return Ok(unchanged());
    }
    let ip: std::net::IpAddr = peer_ip
        .parse()
        .map_err(|_| diagnostic_pin_error("PAWS_BENCH_PEER_IP must be an IP literal"))?;
    if !global_peer_ip(ip) {
        return Err(diagnostic_pin_error(
            "PAWS_BENCH_PEER_IP must be global unicast",
        ));
    }
    if node_fingerprint.len() != 64 || !node_fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(diagnostic_pin_error(
            "PAWS_BENCH_NODE_FINGERPRINT must contain exactly 64 hex digits",
        ));
    }
    let node_fingerprint = node_fingerprint.to_ascii_lowercase();
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|_| diagnostic_pin_error("runtime YAML is invalid"))?;
    let proxies = value
        .get_mut("proxies")
        .and_then(serde_yaml::Value::as_sequence_mut)
        .ok_or_else(|| diagnostic_pin_error("requires inline proxies"))?;
    let matches = proxies
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.get("name")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|name| fingerprint(name) == node_fingerprint)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(diagnostic_pin_error(
            "node fingerprint must match exactly one inline proxy",
        ));
    }
    let proxy = proxies[matches[0]]
        .as_mapping_mut()
        .ok_or_else(|| diagnostic_pin_error("matched proxy is not a mapping"))?;
    if !matches!(
        yaml_string(proxy, "type").as_deref(),
        Some("ss" | "shadowsocks")
    ) {
        return Err(diagnostic_pin_error("matched proxy must be Shadowsocks"));
    }
    if fingerprint(&selected_node_from_selections(selections)) != node_fingerprint {
        return Err(diagnostic_pin_error(
            "fingerprint is not the selected GLOBAL node",
        ));
    }
    let _ = explicit_http_obfs_host(proxy)?;
    let port = yaml_u64(proxy, "port")
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| diagnostic_pin_error("matched proxy requires a valid port"))?;
    proxy.insert(
        serde_yaml::Value::String("server".into()),
        serde_yaml::Value::String(ip.to_string()),
    );
    Ok(DiagnosticRuntime {
        yaml: serde_yaml::to_string(&value)
            .map_err(|_| diagnostic_pin_error("cannot serialize runtime YAML"))?,
        peer_fingerprint: Some(fingerprint(
            &std::net::SocketAddr::new(ip, port).to_string(),
        )),
        peer_address_fingerprint: Some(fingerprint(&ip.to_string())),
    })
}
fn engine_version(engine: VpnEngine) -> &'static str {
    match engine {
        VpnEngine::Meow => "meow-rs 0.22.0 / OHOS socket bridge 2",
        VpnEngine::SingBox => "sing-box 1.13.21 / TinyPaws OpenHarmony bridge 2",
        _ => "mihomo 1.10.0",
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineQualification {
    pub engine: String,
    pub version: String,
    pub profile_fingerprint: String,
    pub build_fingerprint: String,
    pub node_fingerprint: String,
    pub requests100: u32,
    pub successes100: u32,
    pub requests10000: u32,
    pub successes10000: u32,
    pub rss_drift_mib: f64,
    pub pid_stable: bool,
    pub route_proven: bool,
    pub apps_passed: bool,
    pub lock_screen_minutes: u32,
    pub handover_passed: bool,
}
impl EngineQualification {
    fn admitted(&self) -> bool {
        self.requests100 == 100
            && self.successes100 >= 99
            && self.successes100 <= 100
            && self.requests10000 == 10000
            && self.successes10000 >= 9950
            && self.successes10000 <= 10000
            && self.rss_drift_mib.is_finite()
            && self.rss_drift_mib < 20.0
            && self.rss_drift_mib >= -1024.0
            && self.pid_stable
            && self.route_proven
            && self.apps_passed
            && self.lock_screen_minutes >= 480
            && self.handover_passed
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct History {
    records: Vec<HealthRecord>,
    qualifications: Vec<EngineQualification>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HealthRecord {
    engine: String,
    version: String,
    profile_fingerprint: String,
    time: u64,
    success: bool,
    stage: String,
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read_history(root: &Path) -> History {
    process_evidence::read_bounded(&root.join("engine-health.json"), 65536)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}
fn write_history(root: &Path, h: &History) -> Result<(), PawsError> {
    write_atomic_file(
        &root.join("engine-health.json"),
        &serde_json::to_vec(h).map_err(|e| PawsError::Core(e.to_string()))?,
    )
}

pub fn compatibility_issues(raw: &str, engine: VpnEngine) -> Result<Vec<String>, PawsError> {
    if engine == VpnEngine::Mihomo {
        return Ok(Vec::new());
    }
    let value: serde_yaml::Value =
        serde_yaml::from_str(raw).map_err(|e| PawsError::Core(e.to_string()))?;
    let mut issues = Vec::new();
    if value
        .get("proxy-providers")
        .and_then(serde_yaml::Value::as_mapping)
        .is_some_and(|p| !p.is_empty())
    {
        issues.push("proxy-providers：远程节点能力尚未逐项验收".into())
    }
    if let Some(proxies) = value
        .get("proxies")
        .and_then(serde_yaml::Value::as_sequence)
    {
        for (index, proxy) in proxies.iter().enumerate() {
            let kind = proxy
                .get("type")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("unknown");
            if engine == VpnEngine::SingBox
                && !matches!(
                    kind,
                    "ss" | "shadowsocks"
                        | "http"
                        | "socks"
                        | "socks5"
                        | "trojan"
                        | "vmess"
                        | "vless"
                        | "hysteria2"
                        | "hy2"
                        | "tuic"
                )
            {
                issues.push(format!(
                    "proxies[{index}].type：sing-box 尚无 {kind} 配置转换"
                ))
            }
            if let Some(plugin) = proxy.get("plugin").and_then(serde_yaml::Value::as_str) {
                if !matches!(plugin, "obfs" | "simple-obfs" | "obfs-local") {
                    issues.push(format!(
                        "proxies[{index}].plugin：{plugin} 尚无当前平台的兼容转换"
                    ));
                }
            }
            if engine == VpnEngine::SingBox && crate::sing_box_proxy::outbound(proxy).is_none() {
                issues.push(format!("proxies[{index}]：配置字段或传输方式无法完整转换"));
            }
        }
    }
    if engine == VpnEngine::SingBox {
        if let Some(rules) = value.get("rules").and_then(serde_yaml::Value::as_sequence) {
            for (index, rule) in rules.iter().enumerate() {
                let Some(rule) = rule.as_str() else { continue };
                let parts = rule.split(',').collect::<Vec<_>>();
                let kind = parts.first().copied().unwrap_or("");
                if !matches!(
                    kind,
                    "DOMAIN"
                        | "DOMAIN-SUFFIX"
                        | "DOMAIN-KEYWORD"
                        | "IP-CIDR"
                        | "IP-CIDR6"
                        | "DST-PORT"
                        | "MATCH"
                ) && !(kind == "GEOIP"
                    && parts.get(1).is_some_and(|v| {
                        v.eq_ignore_ascii_case("LAN") || v.eq_ignore_ascii_case("PRIVATE")
                    }))
                {
                    issues.push(format!(
                        "rules[{index}]：{kind} 尚无等价的 sing-box 规则转换"
                    ));
                    if issues.len() >= 12 {
                        break;
                    }
                }
            }
        }
    }
    Ok(issues)
}

impl CoreHandle {
    pub fn engine_decision(&self, options_json: &str) -> Result<serde_json::Value, PawsError> {
        let options: VpnOptions = from_json(options_json)?;
        let (raw, root, node_fp) = {
            let state = self.lock_state()?;
            (
                state.profiles.active_raw_yaml()?,
                state.profiles.root().to_path_buf(),
                fingerprint(&selected_node(&state)),
            )
        };
        let fp = fingerprint(&raw);
        let history = read_history(&root);
        let qualified = |engine: VpnEngine| {
            history.qualifications.iter().any(|q| {
                q.engine == engine.as_str()
                    && q.version == engine_version(engine)
                    && q.profile_fingerprint == fp
                    && q.build_fingerprint == build_fingerprint()
                    && q.node_fingerprint == node_fp
                    && q.admitted()
            })
        };
        let cooling = |engine: VpnEngine| {
            history
                .records
                .iter()
                .rev()
                .find(|r| {
                    r.engine == engine.as_str()
                        && r.profile_fingerprint == fp
                        && r.version == engine_version(engine)
                })
                .is_some_and(|r| !r.success && now_secs().saturating_sub(r.time) < 300)
        };
        let requested = options.engine;
        let raw_diagnostic = option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC") == Some("1");
        let candidates = match requested {
            VpnEngine::Auto => vec![VpnEngine::SingBox, VpnEngine::Meow],
            other => vec![other],
        };
        let mut resolved = VpnEngine::Mihomo;
        let mut reasons = Vec::new();
        for engine in candidates {
            if engine == VpnEngine::Mihomo {
                break;
            }
            let issues = compatibility_issues(&raw, engine)?;
            if raw_diagnostic && requested != VpnEngine::Auto {
                resolved = engine;
                reasons.push("诊断构建：绕过发布验收，结果不代表生产准入".into());
                break;
            }
            if !issues.is_empty() {
                reasons.extend(issues);
                continue;
            }
            if cooling(engine) {
                reasons.push(format!(
                    "{}：最近健康检查失败，五分钟内保留正常内核",
                    engine.as_str()
                ));
                continue;
            }
            if !qualified(engine) {
                if requested == VpnEngine::Auto || crate::profile_requires_mihomo(&raw)? {
                    reasons.push(format!(
                        "{}：缺少匹配内核版本和配置指纹的完整验收",
                        engine.as_str()
                    ));
                    continue;
                }
                reasons.push(format!(
                    "{}：已手动指定内核并通过配置兼容检查",
                    engine.as_str()
                ));
            }
            resolved = engine;
            break;
        }
        // Plugin qualification is scoped to this exact configuration and core
        // revision. A future validated plugin can be admitted deliberately;
        // unsupported configuration paths never become silent omissions.
        Ok(
            serde_json::json!({"requested":requested.as_str(),"selected":resolved.as_str(),"version":engine_version(resolved),"profileFingerprint":fp,"buildFingerprint":build_fingerprint(),"nodeFingerprint":node_fp,"reasons":reasons,"productionDefault":"mihomo","diagnosticBypass":raw_diagnostic,"history":history.records.iter().rev().take(8).collect::<Vec<_>>() }),
        )
    }
    pub fn record_engine_qualification(&self, text: &str) -> Result<(), PawsError> {
        if text.len() > 8192 {
            return Err(PawsError::Core("engine qualification is too large".into()));
        }
        let q: EngineQualification =
            serde_json::from_str(text).map_err(|e| PawsError::Core(e.to_string()))?;
        let engine = VpnEngine::try_from(q.engine.as_str())?;
        if engine == VpnEngine::Auto
            || !q.admitted()
            || q.version != engine_version(engine)
            || q.build_fingerprint != build_fingerprint()
        {
            return Err(PawsError::Core(
                "qualification does not meet release gates".into(),
            ));
        }
        let state = self.lock_state()?;
        let root = state.profiles.root();
        if q.profile_fingerprint != fingerprint(&state.profiles.active_raw_yaml()?)
            || q.node_fingerprint != fingerprint(&selected_node(&state))
        {
            return Err(PawsError::Core(
                "qualification belongs to another configuration".into(),
            ));
        }
        let mut h = read_history(root);
        h.qualifications.retain(|old| {
            old.engine != q.engine || old.profile_fingerprint != q.profile_fingerprint
        });
        h.qualifications.push(q);
        if h.qualifications.len() > 12 {
            h.qualifications.remove(0);
        }
        write_history(root, &h)
    }
    pub fn record_engine_health(
        &self,
        session: &str,
        success: bool,
        stage: &str,
    ) -> Result<(), PawsError> {
        if !matches!(
            stage,
            "bootstrap-dns" | "node" | "https" | "return-traffic" | "startup"
        ) {
            return Err(PawsError::Core("invalid engine health stage".into()));
        }
        let state = self.lock_state()?;
        let root = state.profiles.root();
        let record: serde_json::Value =
            process_evidence::read_bounded(&root.join("engine-session.json"), 8192)
                .and_then(|s| serde_json::from_str(&s).ok())
                .ok_or_else(|| PawsError::Core("engine session evidence is unavailable".into()))?;
        if record.get("session").and_then(serde_json::Value::as_str) != Some(session) {
            return Err(PawsError::Core(
                "engine health belongs to a stale session".into(),
            ));
        }
        let engine = VpnEngine::try_from(record["engine"].as_str().unwrap_or(""))?;
        let mut h = read_history(root);
        h.records.push(HealthRecord {
            engine: engine.as_str().into(),
            version: engine_version(engine).into(),
            profile_fingerprint: fingerprint(&state.profiles.active_raw_yaml()?),
            time: now_secs(),
            success,
            stage: stage.into(),
        });
        if h.records.len() > 64 {
            h.records.remove(0);
        }
        write_history(root, &h)
    }
    pub async fn persist_engine_session(
        &self,
        session: &str,
        engine: &str,
        net_id: i32,
    ) -> Result<(), PawsError> {
        let engine = VpnEngine::try_from(engine)?;
        let (root, fp, node_fingerprint, protocol, diagnostic_peer, diagnostic_peer_address) = {
            let state = self.lock_state()?;
            if !state.platform_vpn_running || state.platform_vpn_engine != Some(engine) {
                return Err(PawsError::Core("engine has not started".into()));
            };
            let raw = state.profiles.active_raw_yaml()?;
            let selected = state
                .profiles
                .active_profile()
                .and_then(|id| state.profiles.selected_proxies(id).ok())
                .unwrap_or_default();
            let node = selected_node_from_selections(&selected);
            let diagnostic = compiled_diagnostic_runtime(&raw, &selected)?;
            let parsed: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap_or_default();
            let protocol = parsed
                .get("proxies")
                .and_then(serde_yaml::Value::as_sequence)
                .and_then(|nodes| {
                    nodes.iter().find(|n| {
                        n.get("name").and_then(serde_yaml::Value::as_str) == Some(node.as_str())
                    })
                })
                .and_then(|n| n.get("type"))
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("unresolved")
                .to_owned();
            (
                state.profiles.root().to_path_buf(),
                fingerprint(&raw),
                fingerprint(&node),
                protocol,
                diagnostic.peer_fingerprint,
                diagnostic.peer_address_fingerprint,
            )
        };
        let version = if engine == VpnEngine::Mihomo {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(|e| PawsError::Core(e.to_string()))?;
            match client.get("http://127.0.0.1:9090/version").send().await {
                Ok(r) => r
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v["version"].as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "unknown".into()),
                Err(_) => "unknown".into(),
            }
        } else {
            engine_version(engine).into()
        };
        // Dedicated listener ports are constructed by each actual engine. The
        // benchmark cannot relabel a request sent to another engine's ingress.
        let diagnostic_peer_pinned = diagnostic_peer.is_some();
        let value = serde_json::json!({"format":"TinyPaws-engine-session-v1","buildFingerprint":build_fingerprint(),"mode":self.lock_state()?.mode.as_str(),"diagnosticBypass":option_env!("PAWS_RAW_ENGINE_DIAGNOSTIC")==Some("1"),"diagnosticPeerPinned":diagnostic_peer_pinned,"diagnosticPeerFingerprint":diagnostic_peer.unwrap_or_default(),"diagnosticPeerAddressFingerprint":diagnostic_peer_address.unwrap_or_default(),"session":session,"engine":engine.as_str(),"version":version,"profileFingerprint":fp,"nodeFingerprint":node_fingerprint,"protocol":protocol,"physicalNetId":net_id,"ingressPort":benchmark_port(engine),"pid":std::process::id(),"startedAt":now_secs(),"physicalNetwork":crate::network_binding::physical_network_snapshot()});
        write_atomic_file(
            &root.join("engine-session.json"),
            &serde_json::to_vec(&value).map_err(|e| PawsError::Core(e.to_string()))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PIN_YAML: &str = "# preserve the subscription source\nproxies:\n  - name: bench-node\n    type: ss\n    server: origin.example.invalid\n    port: 443\n    cipher: aes-128-gcm\n    password: test-only\n    plugin: obfs\n    plugin-opts: {mode: http, host: front.example.invalid}\n  - name: other-node\n    type: ss\n    server: other.example.invalid\n    port: 8443\n    cipher: aes-128-gcm\n    password: test-only\nproxy-groups:\n  - name: GLOBAL\n    type: select\n    proxies: [bench-node, other-node]\nrules: ['MATCH,GLOBAL']\n";
    fn pin_selections() -> BTreeMap<String, String> {
        BTreeMap::from([("GLOBAL".into(), "bench-node".into())])
    }
    fn pin_test_runtime(yaml: &str) -> Result<DiagnosticRuntime, PawsError> {
        diagnostic_runtime(
            yaml,
            &pin_selections(),
            true,
            Some("1.1.1.1"),
            Some(&fingerprint("bench-node")),
        )
    }
    #[test]
    fn diagnostic_pin_disabled_or_incomplete_is_an_exact_noop() {
        let malformed = "# no parsing when disabled\n[:";
        for (enabled, ip, node) in [
            (false, Some("1.1.1.1"), Some(fingerprint("bench-node"))),
            (false, Some("invalid"), Some("invalid".into())),
            (true, None, Some(fingerprint("bench-node"))),
            (true, Some("1.1.1.1"), None),
        ] {
            let result =
                diagnostic_runtime(malformed, &pin_selections(), enabled, ip, node.as_deref())
                    .unwrap();
            assert_eq!(result.yaml, malformed);
            assert!(result.peer_fingerprint.is_none());
            assert!(result.peer_address_fingerprint.is_none());
        }
    }
    #[test]
    fn diagnostic_pin_changes_only_one_server_and_preserves_host_and_groups() {
        let original: serde_yaml::Value = serde_yaml::from_str(PIN_YAML).unwrap();
        let result = pin_test_runtime(PIN_YAML).unwrap();
        let pinned: serde_yaml::Value = serde_yaml::from_str(&result.yaml).unwrap();
        let mut expected = original.clone();
        expected["proxies"][0]["server"] = serde_yaml::Value::String("1.1.1.1".into());
        assert_eq!(pinned, expected);
        assert_eq!(result.peer_fingerprint, Some(fingerprint("1.1.1.1:443")));
        assert_eq!(
            result.peer_address_fingerprint,
            Some(fingerprint("1.1.1.1"))
        );
        assert_eq!(original["proxies"][0]["server"], "origin.example.invalid");
    }
    #[test]
    fn diagnostic_pin_preserves_mapping_aliases_and_string_obfs_options() {
        for options in [
            "{mode: http, host: front.example.invalid}",
            "{obfs: http, obfs-host: front.example.invalid}",
            "'obfs=http;obfs-host=front.example.invalid'",
        ] {
            let yaml = PIN_YAML.replace("{mode: http, host: front.example.invalid}", options);
            let result = pin_test_runtime(&yaml).unwrap();
            let original: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
            let pinned: serde_yaml::Value = serde_yaml::from_str(&result.yaml).unwrap();
            assert_eq!(
                pinned["proxies"][0]["plugin-opts"],
                original["proxies"][0]["plugin-opts"]
            );
            let mihomo: serde_yaml::Value =
                serde_yaml::from_str(&render_mihomo_runtime_yaml(&result.yaml).unwrap()).unwrap();
            assert_eq!(mihomo["proxies"], pinned["proxies"]);
            let sing = render_sing_box_runtime_json(
                &result.yaml,
                RuntimeMode::Global,
                &pin_selections(),
                &VpnOptions::default(),
            )
            .unwrap();
            let sing: serde_json::Value = serde_json::from_str(&sing.config_json).unwrap();
            let node = sing["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["tag"] == "bench-node")
                .unwrap();
            assert_eq!(node["server"], "1.1.1.1");
            assert_eq!(
                node["plugin_opts"],
                "obfs=http;obfs-host=front.example.invalid"
            );
        }
    }
    #[test]
    fn diagnostic_pin_ipv6_fingerprints_canonical_address_and_socket() {
        let result = diagnostic_runtime(
            PIN_YAML,
            &pin_selections(),
            true,
            Some("2606:4700:4700:0:0:0:0:1111"),
            Some(&fingerprint("bench-node")),
        )
        .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&result.yaml).unwrap();
        assert_eq!(yaml["proxies"][0]["server"], "2606:4700:4700::1111");
        assert_eq!(
            result.peer_fingerprint,
            Some(fingerprint("[2606:4700:4700::1111]:443"))
        );
        assert_eq!(
            result.peer_address_fingerprint,
            Some(fingerprint("2606:4700:4700::1111"))
        );
    }
    #[test]
    fn diagnostic_pin_rejects_invalid_and_nonglobal_addresses() {
        for ip in [
            "example.invalid",
            "127.0.0.1",
            "0.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.0.1",
            "100.64.0.1",
            "169.254.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "3fff::1",
            "::ffff:1.1.1.1",
        ] {
            assert!(diagnostic_runtime(
                PIN_YAML,
                &pin_selections(),
                true,
                Some(ip),
                Some(&fingerprint("bench-node")),
            )
            .is_err());
        }
    }
    #[test]
    fn diagnostic_pin_requires_exact_unique_selected_ss_node() {
        for fp in [
            "bad".into(),
            fingerprint("bench-node")[..16].into(),
            fingerprint("BENCH-node"),
        ] {
            assert!(diagnostic_runtime(
                PIN_YAML,
                &pin_selections(),
                true,
                Some("1.1.1.1"),
                Some(&fp),
            )
            .is_err());
        }
        let non_ss = PIN_YAML.replace("type: ss", "type: trojan");
        assert!(pin_test_runtime(&non_ss).is_err());
        let mut duplicate: serde_yaml::Value = serde_yaml::from_str(PIN_YAML).unwrap();
        let first = duplicate["proxies"][0].clone();
        duplicate["proxies"].as_sequence_mut().unwrap().push(first);
        assert!(pin_test_runtime(&serde_yaml::to_string(&duplicate).unwrap()).is_err());
        assert!(diagnostic_runtime(
            PIN_YAML,
            &BTreeMap::from([("GLOBAL".into(), "other-node".into())]),
            true,
            Some("1.1.1.1"),
            Some(&fingerprint("bench-node")),
        )
        .is_err());
    }
    #[test]
    fn diagnostic_pin_rejects_missing_or_ambiguous_http_obfs_host() {
        for options in [
            "{mode: http}",
            "{mode: http, host: ''}",
            "{mode: tls, host: front.example.invalid}",
            "{mode: http, host: front.example.invalid, obfs-host: other.example.invalid}",
            "{mode: http, host: 'front.example.invalid;injected=1'}",
            "'obfs=http;obfs-host=front.example.invalid;obfs-host'",
            "'obfs=http;obfs-host=front.example.invalid;host'",
            "'obfs=http; obfs-host=front.example.invalid'",
            "'obfs=http;obfs-host=front.example.invalid\\suffix'",
        ] {
            let yaml = PIN_YAML.replace("{mode: http, host: front.example.invalid}", options);
            assert!(pin_test_runtime(&yaml).is_err());
        }
        assert!(pin_test_runtime(&PIN_YAML.replace("    plugin: obfs\n", "")).is_err());
    }
    #[test]
    fn diagnostic_pin_runtime_write_leaves_profile_source_and_selector_untouched() {
        let root = std::env::temp_dir().join(format!("paws-peer-pin-{}", now_unix_nanos()));
        let mut store = paws_profile::ProfileStore::open(&root).unwrap();
        let id = store
            .import_profile_content("test", "test", PIN_YAML, None)
            .unwrap();
        store
            .set_selected_proxy(&id, "GLOBAL", "bench-node")
            .unwrap();
        let raw_before = store.raw_yaml(&id).unwrap();
        let selections_before = store.selected_proxies(&id).unwrap();
        let runtime = store
            .render_runtime_yaml(&id, RuntimeMode::Global, &VpnOptions::default())
            .unwrap();
        let pinned = pin_test_runtime(&runtime).unwrap();
        store.write_runtime_yaml(&id, &pinned.yaml).unwrap();
        assert_eq!(store.raw_yaml(&id).unwrap(), raw_before);
        assert_eq!(store.selected_proxies(&id).unwrap(), selections_before);
        assert_eq!(
            fs::read_to_string(store.runtime_yaml_path(&id)).unwrap(),
            pinned.yaml
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn qualification_requires_every_gate() {
        let mut q = EngineQualification {
            requests100: 100,
            successes100: 99,
            requests10000: 10000,
            successes10000: 9950,
            rss_drift_mib: 19.9,
            pid_stable: true,
            route_proven: true,
            apps_passed: true,
            lock_screen_minutes: 480,
            handover_passed: true,
            ..Default::default()
        };
        assert!(q.admitted());
        q.lock_screen_minutes = 479;
        assert!(!q.admitted());
        q.lock_screen_minutes = 480;
        q.rss_drift_mib = 20.0;
        assert!(!q.admitted());
        q.rss_drift_mib = f64::NAN;
        assert!(!q.admitted());
    }
    #[test]
    fn ports_cannot_alias() {
        assert_ne!(
            benchmark_port(VpnEngine::Mihomo),
            benchmark_port(VpnEngine::Meow)
        );
        assert_ne!(
            benchmark_port(VpnEngine::Meow),
            benchmark_port(VpnEngine::SingBox)
        );
    }
    #[test]
    fn unsupported_nodes_and_providers_are_explained() {
        assert!(
            compatibility_issues("proxies: [{type: snell}]", VpnEngine::SingBox)
                .unwrap()
                .iter()
                .any(|r| r.contains("snell"))
        );
        assert!(
            compatibility_issues("proxy-providers: {remote: {type: http}}", VpnEngine::Meow)
                .unwrap()
                .iter()
                .any(|r| r.contains("proxy-providers"))
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    #[tokio::test]
    async fn qualified_selection_and_failed_probe_cooldown_are_profile_scoped() {
        let root =
            std::env::temp_dir().join(format!("tinypaws-engine-policy-{}", now_unix_nanos()));
        let core = CoreHandle::new_with_profile_root(&root);
        core.import_profile_from_content(
            "local",
            "test",
            &paws_profile::default_runtime_yaml(),
            None,
        )
        .await
        .unwrap();
        let options = to_json(&VpnOptions::default()).unwrap();
        let decision = core.engine_decision(&options).unwrap();
        assert_eq!(decision["selected"], "mihomo");
        let mut q = EngineQualification {
            engine: "sing-box".into(),
            version: engine_version(VpnEngine::SingBox).into(),
            profile_fingerprint: decision["profileFingerprint"].as_str().unwrap().into(),
            build_fingerprint: build_fingerprint().into(),
            node_fingerprint: decision["nodeFingerprint"].as_str().unwrap().into(),
            requests100: 100,
            successes100: 100,
            requests10000: 10000,
            successes10000: 10000,
            rss_drift_mib: 1.0,
            pid_stable: true,
            route_proven: true,
            apps_passed: true,
            lock_screen_minutes: 480,
            handover_passed: true,
        };
        core.record_engine_qualification(&serde_json::to_string(&q).unwrap())
            .unwrap();
        assert_eq!(core.resolve_vpn_engine(&options).unwrap(), "sing-box");
        fs::write(
            root.join("engine-session.json"),
            r#"{"session":"current","engine":"sing-box"}"#,
        )
        .unwrap();
        assert!(core.record_engine_health("stale", false, "https").is_err());
        core.record_engine_health("current", false, "https")
            .unwrap();
        assert_eq!(core.resolve_vpn_engine(&options).unwrap(), "mihomo");
        q.profile_fingerprint = "another-profile".into();
        assert!(core
            .record_engine_qualification(&serde_json::to_string(&q).unwrap())
            .is_err());
        let _ = fs::remove_dir_all(root);
    }
}
