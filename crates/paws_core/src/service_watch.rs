//! Opt-in service-aware recovery. One bounded loop in the VPN owner process.
use super::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceWatchPolicy {
    pub enabled: bool,
    pub group: String,
    pub url: String,
}

impl CoreHandle {
    pub fn service_watch_policy(&self) -> Result<ServiceWatchPolicy, PawsError> {
        let state = self.lock_state()?;
        let path = state.profiles.root().join("service-watch.json");
        if !path.exists() {
            return Ok(ServiceWatchPolicy::default());
        }
        if fs::metadata(&path)
            .map_err(|e| PawsError::Core(e.to_string()))?
            .len()
            > 8192
        {
            return Err(PawsError::Core("Service policy too large".into()));
        }
        serde_json::from_str(&fs::read_to_string(path).map_err(|e| PawsError::Core(e.to_string()))?)
            .map_err(|e| PawsError::Core(e.to_string()))
    }

    pub fn save_service_watch_policy(&self, policy: ServiceWatchPolicy) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        if policy.enabled {
            let url = reqwest::Url::parse(&policy.url)
                .map_err(|_| PawsError::Core("Invalid probe URL".into()))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || policy.url.len() > 2048
            {
                return Err(PawsError::Core(
                    "Probe requires an HTTPS URL without credentials/query".into(),
                ));
            }
            if !state
                .proxy_groups
                .iter()
                .any(|g| g.name == policy.group && g.group_type.eq_ignore_ascii_case("selector"))
            {
                return Err(PawsError::Core(
                    "Service recovery requires a manual Selector group".into(),
                ));
            }
        }
        let path = state.profiles.root().join("service-watch.json");
        let temp = path.with_extension("json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec(&policy).map_err(|e| PawsError::Core(e.to_string()))?,
        )
        .map_err(|e| PawsError::Core(e.to_string()))?;
        fs::rename(temp, path).map_err(|e| PawsError::Core(e.to_string()))?;
        state.logs.push(info_log(if policy.enabled {
            "service recovery enabled (60s interval, 2 failures, 5min cooldown)"
        } else {
            "service recovery disabled"
        }));
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }
}

pub(super) async fn service_watch_loop(weak: std::sync::Weak<CoreHandle>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut failures = 0u8;
    let mut last_policy = String::new();
    let mut last_switch: Option<Instant> = None;
    loop {
        interval.tick().await;
        let Some(core) = weak.upgrade() else {
            break;
        };
        // UI never owns failover; this also avoids duplicate probes across processes.
        if !matches!(core.platform_ipc(), Ok(Some(p)) if !p.is_ui()) {
            continue;
        }
        let Ok(policy) = core.service_watch_policy() else {
            continue;
        };
        let Ok(snapshot) = core.snapshot() else {
            continue;
        };
        let signature = format!(
            "{}:{}:{:?}",
            policy.group, policy.url, snapshot.vpn_session_id
        );
        if signature != last_policy {
            failures = 0;
            last_switch = None;
            last_policy = signature;
        }
        if !policy.enabled || !snapshot.vpn_running {
            failures = 0;
            continue;
        }
        if last_switch.is_some_and(|time| time.elapsed() < Duration::from_secs(300)) {
            continue;
        }
        let Some(group) = snapshot
            .proxy_groups
            .iter()
            .find(|g| g.name == policy.group && g.group_type.eq_ignore_ascii_case("selector"))
        else {
            continue;
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            core.test_proxy_delay(&policy.group, Some(&policy.url), Some(8000)),
        )
        .await;
        if matches!(result, Ok(Ok(_))) {
            failures = 0;
            continue;
        }
        failures = failures.saturating_add(1);
        if failures < 2 {
            continue;
        }
        failures = 0;
        // Limit each recovery round to three leaf candidates; no unbounded subscription scan.
        let candidates: Vec<_> = group
            .proxies
            .iter()
            .filter(|p| {
                Some(&p.name) != group.selected.as_ref()
                    && ![
                        "direct",
                        "reject",
                        "selector",
                        "urltest",
                        "fallback",
                        "loadbalance",
                        "relay",
                    ]
                    .contains(&p.proxy_type.to_ascii_lowercase().as_str())
            })
            .take(3)
            .map(|p| p.name.clone())
            .collect();
        for candidate in candidates {
            let valid = matches!(core.snapshot(), Ok(now) if now.vpn_session_id == snapshot.vpn_session_id && now.config_revision == snapshot.config_revision && now.vpn_running);
            if !valid
                || !matches!(core.service_watch_policy(), Ok(now) if now.enabled && now.group == policy.group && now.url == policy.url)
            {
                break;
            }
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                core.test_proxy_delay(&candidate, Some(&policy.url), Some(8000)),
            )
            .await;
            if matches!(result, Ok(Ok(_))) {
                if !matches!(core.service_watch_policy(), Ok(now) if now.enabled && now.group == policy.group && now.url == policy.url)
                {
                    break;
                }
                if !matches!(core.snapshot(), Ok(now) if now.vpn_session_id == snapshot.vpn_session_id && now.vpn_running)
                {
                    break;
                }
                // Revision-checked selection prevents recovery crossing a user's newer route change.
                if core
                    .select_proxy_at_revision(
                        &policy.group,
                        &candidate,
                        Some(snapshot.config_revision),
                    )
                    .await
                    .is_ok()
                {
                    last_switch = Some(Instant::now());
                    tracing::info!(
                        "service health recovery selected a verified candidate; cooldown=300s"
                    );
                }
                break;
            }
        }
    }
}
