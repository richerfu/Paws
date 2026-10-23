use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeDomain {
    Config,
    Status,
    Summary,
    Profiles,
    Proxies,
    Rules,
    Providers,
    Geodata,
    Traffic,
    Connections,
    Requests,
    Logs,
    Dns,
    About,
}

impl RuntimeDomain {
    pub fn changed(self, previous: RuntimeRevisions, current: RuntimeRevisions) -> bool {
        let config = previous.config_revision != current.config_revision;
        let status = previous.status_revision != current.status_revision;
        let resources = previous.resource_revision != current.resource_revision;
        let telemetry = previous.telemetry_revision != current.telemetry_revision;
        match self {
            Self::Config => config,
            Self::Rules => config || resources,
            Self::Status => config || status,
            Self::Summary => config || resources || telemetry,
            Self::Profiles | Self::Dns => config || telemetry,
            Self::Proxies | Self::Providers | Self::Geodata => config || resources,
            Self::Traffic | Self::Connections | Self::Requests | Self::Logs => telemetry,
            Self::About => false,
        }
    }
    pub fn is_critical(self) -> bool {
        matches!(self, Self::Config | Self::Status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDomainKey {
    pub domain: RuntimeDomain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_key: Option<String>,
}

impl RuntimeDomainKey {
    pub fn validate(&self) -> Result<(), PawsError> {
        if self.group_key.as_ref().is_some_and(|key| {
            self.domain != RuntimeDomain::Proxies || key.is_empty() || key.len() > 4096
        }) {
            return Err(PawsError::Core(
                "groupKey requires a nonempty proxies group".into(),
            ));
        }
        Ok(())
    }
}

/// A single explicitly requested domain, captured with its fences under one
/// state lock. No aggregate UI projection or implicit collection query exists.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDomainProjection {
    #[serde(flatten)]
    pub key: RuntimeDomainKey,
    pub revision: u64,
    pub config_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<super::runtime_observation::Observation>,
    data: RuntimeDomainData,
}

#[derive(Debug, Serialize)]
#[serde(untagged, rename_all = "camelCase")]
enum RuntimeDomainData {
    Config {
        #[serde(rename = "activeProfile")]
        active_profile: Option<String>,
        mode: RuntimeMode,
        #[serde(rename = "vpnOptions")]
        vpn_options: VpnOptions,
        #[serde(rename = "controllerAccess")]
        controller_access: ControllerAccessConfig,
        #[serde(rename = "networkPorts")]
        network_ports: NetworkPortConfig,
    },
    Status(StatusData),
    Summary(SummaryData),
    Profiles {
        profiles: Vec<ProfileSummary>,
    },
    Proxies {
        #[serde(rename = "proxyGroups")]
        proxy_groups: Vec<ProxyGroup>,
    },
    Rules {
        rules: Vec<paws_model::RuleSummary>,
        #[serde(rename = "runtimeRules")]
        runtime_rules: Option<Vec<paws_engine::EngineRuntimeRule>>,
    },
    Providers {
        providers: Vec<ProviderSummary>,
        #[serde(rename = "runtimeAvailability")]
        runtime_availability: Vec<super::runtime_observation::ProviderAvailability>,
    },
    Geodata {
        geodata: Vec<GeodataFileSummary>,
    },
    Traffic(TrafficData),
    Connections {
        connections: Vec<ConnectionSummary>,
    },
    Requests {
        #[serde(rename = "requestHistory")]
        request_history: Vec<RequestSummary>,
    },
    Logs {
        logs: Vec<LogEntry>,
        #[serde(rename = "recordingError")]
        recording_error: Option<String>,
    },
    Dns {
        dns: DnsSnapshot,
    },
    About {
        about: AboutSnapshot,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusData {
    vpn_session_id: Option<String>,
    vpn_lifecycle: VpnLifecycle,
    engine_loaded: bool,
    active_engine: Option<VpnEngine>,
    running: bool,
    vpn_running: bool,
    network_protected: bool,
    network_protect_error: Option<String>,
    controller_running: bool,
    controller_addr: Option<String>,
    exit_location: ExitLocationSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryData {
    app_version: &'static str,
    profile_count: usize,
    active_profile_name: Option<String>,
    global_node_count: usize,
    node_count: usize,
    group_count: usize,
    primary_selected_node: Option<String>,
    global_selected_node: Option<String>,
    latest_rule_node: Option<String>,
    groups: Vec<GroupHeader>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupHeader {
    name: String,
    group_type: String,
    selected: Option<String>,
    fixed: Option<String>,
    member_count: usize,
    effective_selected_node: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrafficData {
    traffic: TrafficSnapshot,
    traffic_history: Vec<TrafficHistoryPoint>,
    controller_diagnostics: ControllerDiagnostics,
    active_profile_usage: Option<ProfileUsage>,
    connection_summary: ConnectionCounts,
    connection_summary_available: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileUsage {
    upload_bytes: u64,
    download_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionCounts {
    count: usize,
    upload_bytes: u64,
    download_bytes: u64,
    recent: Vec<ConnectionSummary>,
}

impl CoreHandle {
    pub fn query_runtime_domain(
        &self,
        key: RuntimeDomainKey,
    ) -> Result<RuntimeDomainProjection, PawsError> {
        key.validate()?;
        let state = self.lock_state()?;
        self.project_runtime_domain_locked(&state, key)
    }

    pub(super) fn project_runtime_domain_locked(
        &self,
        state: &CoreState,
        key: RuntimeDomainKey,
    ) -> Result<RuntimeDomainProjection, PawsError> {
        let data = match key.domain {
            RuntimeDomain::Config => RuntimeDomainData::Config {
                active_profile: state.profiles.active_profile().map(ToOwned::to_owned),
                mode: state.mode,
                vpn_options: state.vpn_options.clone(),
                controller_access: state.controller_access.clone(),
                network_ports: state.network_ports,
            },
            RuntimeDomain::Status => {
                let native = self.vpn.is_running();
                RuntimeDomainData::Status(StatusData {
                    vpn_session_id: platform_vpn_session_id(state),
                    vpn_lifecycle: vpn_lifecycle(
                        state.engine_loaded,
                        state.platform_vpn_starting,
                        state.platform_vpn_running,
                        native,
                        state.platform_network_protected,
                        state.platform_network_protect_error.as_deref(),
                    ),
                    engine_loaded: state.engine_loaded,
                    active_engine: if native {
                        Some(VpnEngine::Meow)
                    } else if state.platform_vpn_running {
                        state.platform_vpn_engine
                    } else {
                        None
                    },
                    running: state.engine_loaded,
                    vpn_running: native || state.platform_vpn_running,
                    network_protected: state.platform_network_protected,
                    network_protect_error: state.platform_network_protect_error.clone(),
                    controller_running: if state.platform_vpn_running {
                        state.platform_public_controller_addr.is_some()
                    } else {
                        state.api_controller.is_some()
                    },
                    controller_addr: if state.platform_vpn_running {
                        state
                            .platform_public_controller_addr
                            .map(|address| address.to_string())
                    } else {
                        state
                            .api_controller
                            .as_ref()
                            .map(|controller| controller.bind_addr.to_string())
                    },
                    exit_location: state.exit_location.clone(),
                })
            }
            RuntimeDomain::Summary => RuntimeDomainData::Summary(summary(state)),
            RuntimeDomain::Profiles => RuntimeDomainData::Profiles {
                profiles: profile_summaries(state),
            },
            RuntimeDomain::Proxies => RuntimeDomainData::Proxies {
                proxy_groups: match key.group_key.as_deref() {
                    Some(name) => vec![state
                        .proxy_groups
                        .iter()
                        .find(|g| g.name == name)
                        .ok_or_else(|| {
                            PawsError::Core(format!("proxy group '{name}' was not found"))
                        })?
                        .clone()],
                    None => state.proxy_groups.clone(),
                },
            },
            RuntimeDomain::Rules => RuntimeDomainData::Rules {
                rules: state.runtime_rules.clone(),
                runtime_rules: state.engine_observation.runtime_rules.clone().filter(|_| {
                    state.platform_vpn_running
                        && state.engine_observation.attempt == state.platform_start_attempt_id
                        && state.engine_observation.endpoint == state.platform_controller_endpoint
                }),
            },
            RuntimeDomain::Providers => RuntimeDomainData::Providers {
                providers: state.providers.clone(),
                runtime_availability: if state.platform_vpn_running
                    && state.engine_observation.attempt == state.platform_start_attempt_id
                    && state.engine_observation.endpoint == state.platform_controller_endpoint
                {
                    state.engine_observation.providers.clone()
                } else if state.engine_loaded && !state.meow_runtime_ready {
                    state.engine_observation.providers.clone()
                } else {
                    Vec::new()
                },
            },
            RuntimeDomain::Geodata => RuntimeDomainData::Geodata {
                geodata: state.geodata.clone(),
            },
            RuntimeDomain::Traffic => {
                let usage = state.profiles.active_profile().and_then(|id| {
                    state
                        .platform_profile_traffic
                        .as_ref()
                        .filter(|(remote, _, _)| remote == id)
                        .map(|(_, upload, download)| ProfileUsage {
                            upload_bytes: *upload,
                            download_bytes: *download,
                        })
                        .or_else(|| {
                            state.profiles.profile(id).ok().map(|p| ProfileUsage {
                                upload_bytes: p.upload_bytes,
                                download_bytes: p.download_bytes,
                            })
                        })
                });
                RuntimeDomainData::Traffic(TrafficData {
                    traffic: state.traffic.clone(),
                    traffic_history: state.traffic_history.iter().cloned().collect(),
                    controller_diagnostics: state.controller_diagnostics.clone(),
                    active_profile_usage: usage,
                    connection_summary_available: !state.platform_vpn_running
                        || state.engine_observation.connection_summary.is_some(),
                    connection_summary: state
                        .engine_observation
                        .connection_summary
                        .as_ref()
                        .filter(|_| {
                            state.engine_observation.attempt == state.platform_start_attempt_id
                                && state.engine_observation.endpoint
                                    == state.platform_controller_endpoint
                                && state.platform_vpn_running
                        })
                        .map(|summary| ConnectionCounts {
                            count: summary.count,
                            upload_bytes: summary.upload_bytes,
                            download_bytes: summary.download_bytes,
                            recent: summary.recent.clone(),
                        })
                        .unwrap_or_else(|| ConnectionCounts {
                            count: state.connections.len(),
                            upload_bytes: state
                                .connections
                                .iter()
                                .fold(0u64, |sum, c| sum.saturating_add(c.upload_bytes)),
                            download_bytes: state
                                .connections
                                .iter()
                                .fold(0u64, |sum, c| sum.saturating_add(c.download_bytes)),
                            recent: state.connections.iter().take(5).cloned().collect(),
                        }),
                })
            }
            RuntimeDomain::Connections => RuntimeDomainData::Connections {
                connections: state.connections.clone(),
            },
            RuntimeDomain::Requests => RuntimeDomainData::Requests {
                request_history: state.request_history.iter().cloned().rev().collect(),
            },
            RuntimeDomain::Logs => RuntimeDomainData::Logs {
                logs: projected_logs(state),
                recording_error: log_recording_error(state),
            },
            RuntimeDomain::Dns => RuntimeDomainData::Dns {
                dns: state.dns.clone(),
            },
            RuntimeDomain::About => RuntimeDomainData::About {
                about: about_snapshot(),
            },
        };
        let observation = (state.engine_loaded
            && !state.meow_runtime_ready
            && !state.platform_vpn_running
            && matches!(
                key.domain,
                RuntimeDomain::Summary
                    | RuntimeDomain::Proxies
                    | RuntimeDomain::Providers
                    | RuntimeDomain::Rules
            ))
        .then(runtime_observation::definition_observation);
        Ok(RuntimeDomainProjection {
            key,
            revision: state.revision,
            config_revision: state.config_revision,
            observation,
            data,
        })
    }
}

fn selected(group: &ProxyGroup) -> Option<&str> {
    group
        .selected
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            group
                .proxies
                .iter()
                .find(|p| p.selected)
                .map(|p| p.name.as_str())
        })
}

fn effective_leaf(groups: &[ProxyGroup], root: &str) -> Option<String> {
    let mut name = root;
    for _ in 0..=groups.len() {
        let group = groups.iter().find(|g| g.name == name)?;
        let next = selected(group)?;
        if !groups.iter().any(|g| g.name == next) {
            return Some(next.to_owned());
        }
        name = next;
    }
    None
}

fn visible_members(groups: &[ProxyGroup], group: &ProxyGroup) -> usize {
    if !group.name.eq_ignore_ascii_case("GLOBAL") {
        return group.proxies.len();
    }
    let nodes = group
        .proxies
        .iter()
        .filter(|p| {
            !groups.iter().any(|g| g.name == p.name)
                && !["direct", "reject", "rejectdrop"].into_iter().any(|kind| {
                    p.proxy_type
                        .bytes()
                        .filter(u8::is_ascii_alphanumeric)
                        .map(|b| b.to_ascii_lowercase())
                        .eq(kind.bytes())
                })
        })
        .count();
    if nodes > 0 {
        nodes
    } else {
        group.proxies.iter().filter(|p| p.name == "DIRECT").count()
    }
}

fn summary(state: &CoreState) -> SummaryData {
    let groups = &state.proxy_groups;
    let primary = groups
        .iter()
        .find(|g| {
            !g.name.eq_ignore_ascii_case("GLOBAL")
                && g.name.eq_ignore_ascii_case("proxy")
                && selected(g).is_some()
        })
        .or_else(|| {
            groups
                .iter()
                .find(|g| !g.name.eq_ignore_ascii_case("GLOBAL") && selected(g).is_some())
        });
    let global = groups
        .iter()
        .find(|g| g.name.eq_ignore_ascii_case("GLOBAL"));
    let mut summary = SummaryData {
        app_version: APP_VERSION,
        profile_count: state.profiles.profile_count(),
        active_profile_name: state
            .profiles
            .active_profile()
            .and_then(|id| state.profiles.profile(id).ok())
            .map(|p| p.name.clone()),
        global_node_count: global.map_or(0, |g| visible_members(groups, g)),
        node_count: groups
            .iter()
            .filter(|g| !g.name.eq_ignore_ascii_case("GLOBAL"))
            .map(|g| g.proxies.len())
            .sum(),
        group_count: groups
            .iter()
            .filter(|g| !g.name.eq_ignore_ascii_case("GLOBAL"))
            .count(),
        primary_selected_node: primary.and_then(|g| effective_leaf(groups, &g.name)),
        global_selected_node: global.and_then(|g| effective_leaf(groups, &g.name)),
        latest_rule_node: state
            .connections
            .iter()
            .filter(|c| !c.chains.is_empty())
            .max_by(|a, b| a.started_at.cmp(&b.started_at))
            .and_then(|c| c.chains.last())
            .cloned(),
        groups: groups
            .iter()
            .map(|g| GroupHeader {
                name: g.name.clone(),
                group_type: g.group_type.clone(),
                selected: selected(g).map(ToOwned::to_owned),
                fixed: g.fixed.clone(),
                member_count: visible_members(groups, g),
                effective_selected_node: effective_leaf(groups, &g.name),
            })
            .collect(),
    };
    if state.platform_vpn_running
        && state.engine_observation.attempt == state.platform_start_attempt_id
        && state.engine_observation.endpoint == state.platform_controller_endpoint
    {
        if let Some(groups) = &state.engine_observation.groups {
            let leaf = |root: &str| {
                let mut name = root;
                for _ in 0..=groups.len() {
                    let next = groups
                        .iter()
                        .find(|group| group.name == name)?
                        .selected
                        .as_deref()?;
                    if !groups.iter().any(|group| group.name == next) {
                        return Some(next.to_owned());
                    }
                    name = next;
                }
                None
            };
            let primary = groups
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case("proxy"))
                .or_else(|| {
                    groups.iter().find(|group| {
                        !group.name.eq_ignore_ascii_case("GLOBAL") && group.selected.is_some()
                    })
                });
            summary.primary_selected_node = primary.and_then(|group| leaf(&group.name));
            summary.global_selected_node = groups
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case("GLOBAL"))
                .and_then(|group| leaf(&group.name));
            summary.group_count = groups
                .iter()
                .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
                .count();
            summary.node_count = groups
                .iter()
                .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
                .map(|group| group.member_count)
                .sum();
            summary.global_node_count = groups
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case("GLOBAL"))
                .map_or(0, |group| group.member_count);
            summary.groups = groups
                .iter()
                .map(|group| GroupHeader {
                    name: group.name.clone(),
                    group_type: group.group_type.clone(),
                    selected: group.selected.clone(),
                    fixed: group.fixed.clone(),
                    member_count: group.member_count,
                    effective_selected_node: leaf(&group.name),
                })
                .collect();
        }
        summary.latest_rule_node = state
            .engine_observation
            .connection_summary
            .as_ref()
            .and_then(|summary| summary.latest_rule_node.clone());
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn core() -> CoreHandle {
        CoreHandle::new_with_profile_root(
            std::env::temp_dir().join(format!("paws-domain-query-{}", now_unix_nanos())),
        )
    }
    fn read(core: &CoreHandle, domain: RuntimeDomain, group_key: Option<&str>) -> Value {
        serde_json::to_value(
            core.query_runtime_domain(RuntimeDomainKey {
                domain,
                group_key: group_key.map(ToOwned::to_owned),
            })
            .unwrap(),
        )
        .unwrap()
    }
    fn group(name: &str, selected: &str, members: &[(&str, &str)]) -> ProxyGroup {
        ProxyGroup {
            name: name.into(),
            group_type: "Selector".into(),
            selected: Some(selected.into()),
            fixed: None,
            proxies: members
                .iter()
                .map(|(name, kind)| paws_model::ProxyItem {
                    name: (*name).into(),
                    proxy_type: (*kind).into(),
                    delay_ms: None,
                    selected: *name == selected,
                })
                .collect(),
        }
    }

    #[test]
    fn critical_domains_and_summary_never_copy_large_rules_or_node_rosters() {
        let core = core();
        {
            let mut state = core.lock_state().unwrap();
            state.runtime_rules = (0..12_000)
                .map(|index| paws_model::RuleSummary {
                    id: format!("rule-{index}"),
                    profile_id: "large".into(),
                    line: format!("DOMAIN-SUFFIX,resource-{index}.example.test,DIRECT"),
                    enabled: true,
                    order: index,
                    source: "subscription".into(),
                })
                .collect();
            state.proxy_groups = vec![
                group(
                    "GLOBAL",
                    "Proxy",
                    &[
                        ("Proxy", "Selector"),
                        ("DIRECT", "Direct"),
                        ("node-a", "ss"),
                    ],
                ),
                group("Proxy", "node-a", &[("node-a", "ss"), ("node-b", "ss")]),
            ];
        }
        for domain in [
            RuntimeDomain::Config,
            RuntimeDomain::Status,
            RuntimeDomain::Summary,
        ] {
            let value = read(&core, domain, None);
            assert!(serde_json::to_string(&value).unwrap().len() < 16_000);
            for key in ["rules", "proxyGroups", "providers", "connections", "about"] {
                assert!(value["data"].get(key).is_none());
            }
        }
        let summary = read(&core, RuntimeDomain::Summary, None);
        assert_eq!(summary["data"]["globalNodeCount"], 1);
        assert_eq!(summary["data"]["nodeCount"], 2);
        assert_eq!(summary["data"]["groupCount"], 1);
        assert_eq!(summary["data"]["primarySelectedNode"], "node-a");
        assert_eq!(summary["data"]["globalSelectedNode"], "node-a");
        assert!(summary["data"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g.get("proxies").is_none()));
        let rules = read(&core, RuntimeDomain::Rules, None);
        assert!(
            serde_json::to_string(&rules).unwrap().len() > 1_600_000,
            "large rules are available only through their explicit query"
        );
    }

    #[test]
    fn group_queries_are_scoped_and_fenced_and_traffic_recent_connections_are_bounded() {
        let core = core();
        {
            let mut state = core.lock_state().unwrap();
            state.proxy_groups = vec![
                group("A", "node-a", &[("node-a", "ss")]),
                group("B", "node-b", &[("node-b", "ss")]),
            ];
            state.connections = (0..20)
                .map(|i| ConnectionSummary {
                    domain_source: String::new(),
                    id: format!("{i}"),
                    host: "example.test".into(),
                    domain: String::new(),
                    destination_ip: String::new(),
                    destination_port: 443,
                    network: "tcp".into(),
                    rule: String::new(),
                    rule_payload: String::new(),
                    proxy: "node-a".into(),
                    chains: vec!["A".into(), "node-a".into()],
                    started_at: format!("{i:02}"),
                    upload_bytes: 1,
                    download_bytes: 2,
                })
                .collect();
            core.publish_runtime_change_locked(&mut state, true, false);
        }
        let scoped = read(&core, RuntimeDomain::Proxies, Some("B"));
        assert_eq!(scoped["data"]["proxyGroups"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["data"]["proxyGroups"][0]["name"], "B");
        assert_eq!(scoped["revision"], core.snapshot().unwrap().revision);
        assert_eq!(
            scoped["configRevision"],
            core.snapshot().unwrap().config_revision
        );
        assert!(core
            .query_runtime_domain(RuntimeDomainKey {
                domain: RuntimeDomain::Rules,
                group_key: Some("B".into())
            })
            .is_err());
        assert!(core
            .query_runtime_domain(RuntimeDomainKey {
                domain: RuntimeDomain::Proxies,
                group_key: Some("missing".into())
            })
            .is_err());
        let traffic = read(&core, RuntimeDomain::Traffic, None);
        assert_eq!(traffic["data"]["connectionSummary"]["count"], 20);
        assert_eq!(traffic["data"]["connectionSummary"]["uploadBytes"], 20);
        assert_eq!(traffic["data"]["connectionSummary"]["downloadBytes"], 40);
        assert_eq!(
            traffic["data"]["connectionSummary"]["recent"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let summary = read(&core, RuntimeDomain::Summary, None);
        assert_eq!(summary["data"]["latestRuleNode"], "node-a");
    }
}
