//! Session-fenced, individually requested live engine observations. The UI
//! query owns the work; no collection is added to the periodic IPC payload.
use super::*;
use paws_engine::{
    EngineConnectionPage, EngineConnectionSummary, EngineDnsConfig, EngineDnsStatistics,
    EngineGroupSummary, EngineLogPage, EngineMemory, EngineProviderKind, EngineProviderState,
    EngineRuntimeRule,
};
#[cfg(test)]
#[path = "runtime_observation_tests.rs"]
mod tests;

/// One cancelable observation job per native owner. Timer ticks never queue
/// behind an existing job; traffic has a completely separate publication path.
#[derive(Default)]
pub(super) struct ObservationLane {
    pub(super) gate: tokio::sync::Mutex<()>,
    generation: AtomicU64,
    suspended: AtomicBool,
    cancelled: tokio::sync::Notify,
}
impl ObservationLane {
    pub(super) fn ticket(&self) -> Option<u64> {
        (!self.suspended.load(Ordering::Acquire)).then(|| self.generation.load(Ordering::Acquire))
    }
    pub(super) fn current(&self, ticket: u64) -> bool {
        !self.suspended.load(Ordering::Acquire) && self.generation.load(Ordering::Acquire) == ticket
    }
    pub(super) fn resume(&self) {
        self.suspended.store(false, Ordering::Release);
    }
    pub(super) async fn cancel_and_join(&self) {
        self.suspended.store(true, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.cancelled.notify_waiters();
        // An in-progress synchronous archive transaction drains before this
        // guard becomes available; only pending async IO is cancelled.
        drop(self.gate.lock().await);
    }
    pub(super) async fn cancelled(&self, ticket: u64) {
        loop {
            let notified = self.cancelled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.current(ticket) {
                return;
            }
            notified.await;
        }
    }
}

/// Archive-only outcome. Control/on/off and application writer errors remain
/// independent; an archive success cannot erase them. The nonce is opaque and
/// the endpoint epoch is a digest, never the native credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArchiveRecordingStatus {
    pub(super) source: log_recording::NativeLogSource,
    pub(super) nonce: String,
    pub(super) error: Option<String>,
}
impl ArchiveRecordingStatus {
    pub(super) fn matches(&self, state: &CoreState) -> bool {
        state.platform_vpn_running
            && state.platform_start_attempt_id == self.source.attempt
            && state
                .platform_controller_endpoint
                .as_ref()
                .is_some_and(|endpoint| engine::management_epoch(endpoint) == self.source.epoch)
            && state.logs.session_id() == Some(self.nonce.as_str())
    }
}

pub(super) fn bounded_recording_error(error: &str) -> String {
    let mut end = error.len().min(1024);
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_owned()
}

#[derive(Default)]
pub(super) struct EngineObservationState {
    pub attempt: String,
    pub groups: Option<Vec<EngineGroupSummary>>,
    pub runtime_rules: Option<Vec<EngineRuntimeRule>>,
    pub connection_summary: Option<EngineConnectionSummary>,
    pub connection_error: Option<String>,
    pub revisions: Option<paws_engine::EngineRuntimeRevisions>,
    pub endpoint: Option<paws_engine::EngineControllerEndpoint>,
    pub providers: Vec<ProviderAvailability>,
    pub log_cursor: u64,
    pub connection_cursor: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    engine: Option<VpnEngine>,
    session: Option<String>,
    issues: Vec<ObservationIssue>,
}

pub(super) fn definition_observation() -> Observation {
    Observation {
        engine: None,
        session: None,
        issues: vec![ObservationIssue {
            field: "configuration",
            reason:
                "离线配置/缓存声明预览；成员过滤与当前路由以实际运行内核为准，尚未测量延迟/健康"
                    .into(),
        }],
    }
}
#[derive(Debug, Serialize)]
struct ObservationIssue {
    field: &'static str,
    reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderAvailability {
    name: String,
    provider_type: String,
    available: bool,
    reason: Option<String>,
    rule_count: Option<usize>,
    updated_at: Option<String>,
}

pub(super) fn definition_provider_availability(
    providers: &[ProviderSummary],
    uncached: &[String],
) -> Vec<ProviderAvailability> {
    providers
        .iter()
        .map(|provider| ProviderAvailability {
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            available: false,
            reason: Some(
                if uncached.contains(&provider.name) {
                    "配置声明；Provider 缓存尚未加载，未进行当前内核运行检测"
                } else {
                    "配置/缓存声明；未进行当前内核运行检测"
                }
                .into(),
            ),
            rule_count: None,
            updated_at: None,
        })
        .collect()
}

enum Sample {
    None,
    Summary(
        Vec<EngineGroupSummary>,
        Result<EngineConnectionSummary, PawsError>,
    ),
    Proxies(Vec<ProxyGroup>),
    Rules(Vec<EngineRuntimeRule>),
    Providers(
        Vec<(
            String,
            EngineProviderKind,
            Result<EngineProviderState, PawsError>,
        )>,
    ),
    Traffic(
        Result<EngineMemory, PawsError>,
        Result<EngineConnectionSummary, PawsError>,
    ),
    Connections(Vec<ConnectionSummary>),
    Requests(EngineConnectionPage, Vec<ConnectionSummary>),
    Logs(EngineLogPage),
    Dns(
        Result<EngineDnsConfig, PawsError>,
        Result<EngineDnsStatistics, PawsError>,
    ),
}

async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, paws_engine::EngineError>>,
) -> Result<T, PawsError> {
    tokio::time::timeout(Duration::from_secs(8), future)
        .await
        .map_err(|_| PawsError::Core("engine observation timed out after 8 seconds".into()))?
        .map_err(engine_error)
}

struct QueryScope<'a, F> {
    core: &'a CoreHandle,
    attempt: &'a str,
    revision: u64,
    endpoint: &'a Option<paws_engine::EngineControllerEndpoint>,
    deadline: tokio::time::Instant,
    alive: F,
}
impl<F: Fn() -> bool> QueryScope<'_, F> {
    fn checkpoint(&self) -> Result<(), PawsError> {
        if !(self.alive)() {
            return Err(PawsError::Core(
                "runtime domain query subscription was closed or superseded".into(),
            ));
        }
        let mut state = self.core.lock_state()?;
        self.core.sync_platform_vpn_state_locked(&mut state);
        if !state.platform_vpn_running
            || state.platform_start_attempt_id != self.attempt
            || &state.platform_controller_endpoint != self.endpoint
        {
            return Err(PawsError::Core(
                "runtime domain query belongs to an obsolete engine session".into(),
            ));
        }
        self.core.ensure_captured_management_locked(
            &state,
            self.attempt,
            self.revision,
            self.endpoint,
        )
    }
    async fn read<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, paws_engine::EngineError>>,
    ) -> Result<T, PawsError> {
        self.checkpoint()?;
        if tokio::time::Instant::now() >= self.deadline {
            return Err(PawsError::Core(
                "runtime domain query exceeded its 8 second total budget".into(),
            ));
        }
        let deadline = self
            .deadline
            .min(tokio::time::Instant::now() + Duration::from_secs(4));
        let result = tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| {
                PawsError::Core("engine observation exceeded the domain query deadline".into())
            })?
            .map_err(engine_error);
        self.checkpoint()?;
        result
    }
}

impl CoreHandle {
    /// Store only a still-current archive outcome. A completed Off or a new
    /// recording nonce wins over any in-flight read/error/clear from before it.
    pub(super) fn record_native_archive_outcome(
        &self,
        target: &engine::ManagementTarget,
        nonce: &str,
        error: Option<&PawsError>,
    ) -> Result<(), PawsError> {
        let source = target.log_source()?;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(
            &state,
            &target.attempt,
            target.revision,
            &target.endpoint,
        )?;
        state.logs.sync_session();
        if state.logs.session_id() != Some(nonce) {
            return Ok(());
        }
        state.archive_status = Some(ArchiveRecordingStatus {
            source,
            nonce: nonce.to_owned(),
            error: error.map(|error| bounded_recording_error(&error.to_string())),
        });
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }

    pub async fn clear_runtime_history(&self, logs: bool) -> Result<(), PawsError> {
        let Some(engine::ManagementTarget {
            attempt,
            revision,
            endpoint: epoch,
            management,
        }) = self.runtime_management()?
        else {
            return if logs {
                self.clear_logs()
            } else {
                self.clear_request_history()
            };
        };
        // A clear operation cuts at the actual engine journal watermark, not
        // the last page the UI happened to read.
        let watermark = if logs {
            bounded(management.runtime_logs(0, 1)).await?.high_watermark
        } else {
            bounded(management.connection_events(0, 1))
                .await?
                .high_watermark
        };
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !state.platform_vpn_running
            || state.platform_start_attempt_id != attempt
            || state.platform_controller_endpoint != epoch
        {
            return Err(PawsError::Core(
                "history clear belongs to an obsolete session".into(),
            ));
        }
        self.ensure_captured_management_locked(&state, &attempt, revision, &epoch)?;
        if state.engine_observation.attempt != attempt || state.engine_observation.endpoint != epoch
        {
            state.engine_observation = EngineObservationState {
                attempt,
                endpoint: epoch,
                ..Default::default()
            };
        }
        if logs {
            state.engine_observation.log_cursor = watermark;
            state.logs.clear();
            state.platform_logs = None;
            if let Ok(mut runtime) = RUNTIME_LOGS.lock() {
                runtime.clear();
            }
        } else {
            state.engine_observation.connection_cursor = watermark;
            state.request_history.clear();
        }
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }

    /// The native journal's archive cursor is independent from every UI read.
    pub async fn set_engine_log_recording_enabled(
        &self,
        enabled: bool,
    ) -> Result<LogRecordingStatus, PawsError> {
        if !enabled {
            return self.set_log_recording_enabled(false);
        }
        let Some(target) = self.runtime_management()? else {
            return self.set_log_recording_enabled(true);
        };
        let source = target.log_source()?;
        let (root, control, nonce) = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(
                &state,
                &target.attempt,
                target.revision,
                &target.endpoint,
            )?;
            let root = state.profiles.root().to_path_buf();
            if let Some(boundary) = log_recording::native_recording_boundary(&root)? {
                if boundary.source.attempt != source.attempt
                    || boundary.source.epoch != source.epoch
                {
                    return Err(PawsError::Core("log recording is waiting for the current native owner to publish its boundary; retry reading its status".into()));
                }
                state.logs.sync_session();
                let mut status = log_recording::recording_status(&root)?;
                status.last_error = log_recording_error(&state);
                return Ok(status);
            }
            let control = log_recording::recording_control_token(&root)?;
            let nonce = log_recording::active_recording_session(&root)?;
            (root, control, nonce)
        };
        let watermark = bounded(target.management.runtime_logs(0, 1))
            .await?
            .high_watermark;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(
            &state,
            &target.attempt,
            target.revision,
            &target.endpoint,
        )?;
        let boundary = match nonce.as_deref() {
            Some(nonce) => log_recording::attach_native_recording_source(
                &root, nonce, None, &source, watermark, None,
            )?,
            None => log_recording::enable_native_recording_if_current(
                &root, &control, &source, watermark,
            )?,
        };
        if boundary.is_none() {
            return Err(PawsError::Core(
                "log recording request was superseded by a newer on/off operation".into(),
            ));
        }
        if nonce.is_none() {
            state.logs.clear();
            if let Ok(mut runtime) = RUNTIME_LOGS.lock() {
                runtime.clear();
            }
        }
        state.logs.sync_session();
        state.logs.clear_control_error();
        let mut status = log_recording::recording_status(&root)?;
        status.last_error = log_recording_error(&state);
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(status)
    }

    /// Called only after a real native start/reload was successfully published.
    /// A recording which already existed before the source starts includes its
    /// initial events; a later enable cuts at a measured native watermark.
    pub(super) async fn initialize_native_log_recording(
        &self,
        started_at: u128,
    ) -> Result<(), PawsError> {
        let Some(target) = self.runtime_management()? else {
            return Ok(());
        };
        let source = target.log_source()?;
        let (root, nonce, previous_source) = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(
                &state,
                &target.attempt,
                target.revision,
                &target.endpoint,
            )?;
            let root = state.profiles.root().to_path_buf();
            if log_recording::ensure_native_recording_source(&root, &source, started_at)?.is_some()
            {
                return Ok(());
            }
            let Some(nonce) = log_recording::active_recording_session(&root)? else {
                return Ok(());
            };
            let previous_source =
                log_recording::native_recording_boundary(&root)?.map(|boundary| boundary.source);
            (root, nonce, previous_source)
        };
        let watermark = bounded(target.management.runtime_logs(0, 1))
            .await?
            .high_watermark;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(
            &state,
            &target.attempt,
            target.revision,
            &target.endpoint,
        )?;
        log_recording::attach_native_recording_source(
            &root,
            &nonce,
            previous_source.as_ref(),
            &source,
            watermark,
            Some(started_at),
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) async fn consume_engine_runtime_logs(&self) -> Result<(), PawsError> {
        let Some(target) = self.runtime_management()? else {
            return Ok(());
        };
        self.consume_engine_runtime_logs_for(&target).await
    }
    pub(super) async fn consume_engine_runtime_logs_for(
        &self,
        target: &engine::ManagementTarget,
    ) -> Result<(), PawsError> {
        let source = target.log_source()?;
        let boundary = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(
                &state,
                &target.attempt,
                target.revision,
                &target.endpoint,
            )?;
            let Some(boundary) = log_recording::native_recording_boundary(state.profiles.root())?
            else {
                return Ok(());
            };
            if boundary.source.attempt != source.attempt || boundary.source.epoch != source.epoch {
                return Err(PawsError::Core(
                    "native log recording is waiting for its current engine boundary".into(),
                ));
            }
            boundary
        };
        let scope = QueryScope {
            core: self,
            attempt: &target.attempt,
            revision: target.revision,
            endpoint: &target.endpoint,
            deadline: tokio::time::Instant::now() + Duration::from_secs(8),
            alive: || true,
        };
        let mut cursor = boundary.cursor;
        let mut watermark = None;
        for _ in 0..32 {
            let page = scope
                .read(target.management.runtime_logs(cursor, 512))
                .await?;
            let limit = *watermark.get_or_insert(page.high_watermark);
            let next = page.next_cursor.min(limit);
            if next < cursor {
                return Err(PawsError::Core(
                    "native archive cursor moved backwards".into(),
                ));
            }
            let entries = page
                .entries
                .into_iter()
                .filter(|entry| entry.sequence <= limit)
                .map(|entry| {
                    (
                        entry.sequence,
                        LogEntry {
                            level: entry.level,
                            message: entry.message,
                            timestamp: entry.timestamp,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(
                &state,
                &target.attempt,
                target.revision,
                &target.endpoint,
            )?;
            if !state
                .logs
                .append_native(&boundary, &entries, next, page.dropped_before)?
            {
                return Ok(());
            }
            drop(state);
            if next >= limit {
                return Ok(());
            }
            if next == cursor {
                return Err(PawsError::Core(
                    "native archive cursor made no progress".into(),
                ));
            }
            cursor = next;
        }
        // Another periodic archive pass continues from the persisted cursor;
        // unseen entries are never skipped to the native high watermark.
        Ok(())
    }

    /// Sampling can await native work, but projection and ownership validation
    /// share the final state lock. A replacement session cannot inherit data
    /// fetched from its predecessor, even if the configuration is unchanged.
    pub async fn refresh_and_query_runtime_domain(
        &self,
        key: RuntimeDomainKey,
    ) -> Result<RuntimeDomainProjection, PawsError> {
        self.refresh_and_query_runtime_domain_with_guard(key, || true)
            .await
    }

    /// Hosts supply their actual query lease, so a departed page stops further
    /// native requests instead of only dropping the eventual JSON response.
    pub async fn refresh_and_query_runtime_domain_with_guard(
        &self,
        key: RuntimeDomainKey,
        alive: impl Fn() -> bool,
    ) -> Result<RuntimeDomainProjection, PawsError> {
        key.validate()?;
        if !alive() {
            return Err(PawsError::Core(
                "runtime domain query subscription was closed".into(),
            ));
        }
        if matches!(
            key.domain,
            RuntimeDomain::Config
                | RuntimeDomain::Status
                | RuntimeDomain::Profiles
                | RuntimeDomain::Geodata
                | RuntimeDomain::About
        ) {
            return self.query_runtime_domain(key);
        }
        let target = self.runtime_management()?;
        let Some(engine::ManagementTarget {
            attempt,
            revision,
            endpoint: epoch,
            management,
        }) = target
        else {
            return self.query_runtime_domain(key);
        };
        let engine = management.capabilities().engine;
        let (providers, log_cursor, connection_cursor) = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(&state, &attempt, revision, &epoch)?;
            let same_owner = state.engine_observation.attempt == attempt
                && state.engine_observation.endpoint == epoch;
            (
                if key.domain == RuntimeDomain::Providers {
                    state
                        .providers
                        .iter()
                        .map(|provider| (provider.provider_type.clone(), provider.name.clone()))
                        .collect()
                } else {
                    Vec::new()
                },
                if same_owner {
                    state.engine_observation.log_cursor
                } else {
                    0
                },
                if same_owner {
                    state.engine_observation.connection_cursor
                } else {
                    0
                },
            )
        };
        let scope = QueryScope {
            core: self,
            attempt: &attempt,
            revision,
            endpoint: &epoch,
            deadline: tokio::time::Instant::now() + Duration::from_secs(8),
            alive,
        };
        let sample = match key.domain {
            RuntimeDomain::Summary => Sample::Summary(
                scope.read(management.group_summaries()).await?,
                scope.read(management.connection_summary()).await,
            ),
            RuntimeDomain::Proxies => Sample::Proxies(match key.group_key.as_deref() {
                Some(name) => vec![scope.read(management.proxy_group(name)).await?],
                None => scope.read(management.proxy_groups()).await?,
            }),
            RuntimeDomain::Rules => Sample::Rules(scope.read(management.runtime_rules()).await?),
            RuntimeDomain::Providers => {
                let observed = futures::stream::iter(providers)
                    .map(|(kind, name)| {
                        let management = management.clone();
                        let scope = &scope;
                        async move {
                            let kind = if kind == "proxy" {
                                EngineProviderKind::Proxy
                            } else {
                                EngineProviderKind::Rule
                            };
                            let state = scope.read(management.provider_state(kind, &name)).await;
                            (name, kind, state)
                        }
                    })
                    .buffered(4)
                    .collect()
                    .await;
                Sample::Providers(observed)
            }
            RuntimeDomain::Traffic => Sample::Traffic(
                scope.read(management.memory()).await,
                scope.read(management.connection_summary()).await,
            ),
            RuntimeDomain::Connections => {
                Sample::Connections(scope.read(management.connections()).await?)
            }
            RuntimeDomain::Requests => Sample::Requests(
                drain_connections(&scope, management.as_ref(), connection_cursor).await?,
                scope.read(management.connections()).await?,
            ),
            RuntimeDomain::Logs => {
                Sample::Logs(drain_logs(&scope, management.as_ref(), log_cursor).await?)
            }
            RuntimeDomain::Dns => Sample::Dns(
                scope.read(management.dns_config()).await,
                scope.read(management.dns_statistics()).await,
            ),
            _ => Sample::None,
        };
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        if !state.platform_vpn_running
            || state.platform_start_attempt_id != attempt
            || state.platform_vpn_engine != Some(engine)
            || state.platform_controller_endpoint != epoch
        {
            return Err(PawsError::Core(
                "engine observation belongs to an obsolete session".into(),
            ));
        }
        self.ensure_captured_management_locked(&state, &attempt, revision, &epoch)?;
        if !(scope.alive)() {
            return Err(PawsError::Core(
                "runtime domain query subscription was superseded".into(),
            ));
        }
        if state.engine_observation.attempt != attempt || state.engine_observation.endpoint != epoch
        {
            state.engine_observation = EngineObservationState {
                attempt: attempt.clone(),
                endpoint: epoch.clone(),
                ..Default::default()
            };
            state.platform_logs = None;
        }
        let mut issues = Vec::new();
        match sample {
            Sample::None => {}
            Sample::Summary(groups, connections) => {
                let changed = state.engine_observation.groups.as_ref() != Some(&groups);
                state.engine_observation.groups = Some(groups);
                apply_connection_summary(&mut state, connections, &mut issues);
                if changed {
                    self.publish_resource_change_locked(&mut state);
                }
            }
            Sample::Proxies(groups) => {
                if key.group_key.is_none() {
                    state.proxy_groups = groups;
                } else {
                    for group in groups {
                        if let Some(existing) = state
                            .proxy_groups
                            .iter_mut()
                            .find(|existing| existing.name == group.name)
                        {
                            *existing = group;
                        } else {
                            state.proxy_groups.push(group);
                        }
                    }
                }
                state.engine_resources_authoritative = true;
            }
            Sample::Rules(rules) => state.engine_observation.runtime_rules = Some(rules),
            Sample::Providers(providers) => {
                state.engine_observation.providers.clear();
                for (name, kind, result) in providers {
                    let provider_type = if kind == EngineProviderKind::Proxy {
                        "proxy"
                    } else {
                        "rule"
                    }
                    .to_owned();
                    match result {
                        Ok(observed) => {
                            if let Some(provider) = state.providers.iter_mut().find(|provider| {
                                provider.name == name && provider.provider_type == provider_type
                            }) {
                                if let Some(members) = observed.members {
                                    provider.members = members;
                                }
                                provider.vehicle_type = observed.vehicle;
                            }
                            state
                                .engine_observation
                                .providers
                                .push(ProviderAvailability {
                                    name,
                                    provider_type,
                                    available: true,
                                    reason: None,
                                    rule_count: observed.rule_count,
                                    updated_at: observed.updated_at,
                                });
                        }
                        Err(error) => {
                            issues.push(ObservationIssue {
                                field: "provider",
                                reason: format!("{name}: {error}"),
                            });
                            state
                                .engine_observation
                                .providers
                                .push(ProviderAvailability {
                                    name,
                                    provider_type,
                                    available: false,
                                    reason: Some(error.to_string()),
                                    rule_count: None,
                                    updated_at: None,
                                });
                        }
                    }
                }
            }
            Sample::Traffic(memory, connections) => {
                apply_connection_summary(&mut state, connections, &mut issues);
                let diagnostics = &mut state.controller_diagnostics;
                match memory {
                    Ok(memory) => {
                        diagnostics.memory_available = true;
                        diagnostics.memory_in_use_bytes = memory.in_use_bytes;
                        diagnostics.memory_limit_available = memory.limit_bytes.is_some();
                        diagnostics.memory_limit_bytes = memory.limit_bytes.unwrap_or_default();
                        diagnostics.memory_scope = Some(
                            match memory.scope {
                                paws_engine::EngineMemoryScope::ProcessResident => {
                                    "processResident"
                                }
                                paws_engine::EngineMemoryScope::EngineHeap => "engineHeap",
                            }
                            .into(),
                        );
                        diagnostics.memory_error = None;
                    }
                    Err(error) => {
                        diagnostics.memory_available = false;
                        diagnostics.memory_limit_available = false;
                        diagnostics.memory_scope = None;
                        diagnostics.memory_error = Some(error.to_string());
                        issues.push(ObservationIssue {
                            field: "memory",
                            reason: error.to_string(),
                        });
                    }
                }
            }
            Sample::Connections(connections) => {
                record_request_history(&mut state, &connections);
                state.connections = connections;
            }
            Sample::Requests(page, active) => {
                if page.next_cursor < state.engine_observation.connection_cursor {
                    return Err(PawsError::Core(
                        "engine connection event cursor moved backwards".into(),
                    ));
                }
                for event in page.entries {
                    if event.sequence <= state.engine_observation.connection_cursor {
                        continue;
                    }
                    let connection = event.connection;
                    if event.closed {
                        state
                            .connections
                            .retain(|existing| existing.id != connection.id);
                    } else if let Some(existing) = state
                        .connections
                        .iter_mut()
                        .find(|existing| existing.id == connection.id)
                    {
                        *existing = connection.clone();
                    } else {
                        state.connections.push(connection.clone());
                    }
                    // History merge is id-based and bounded. Closed events are
                    // recorded even when no UI page observed the short flow.
                    merge_request_event(&mut state, connection, event.closed);
                }
                state.engine_observation.connection_cursor = page.next_cursor;
                if page.dropped_before.is_some() {
                    issues.push(ObservationIssue {
                        field: "requestHistory",
                        reason:
                            "older connection events were evicted from the bounded engine buffer"
                                .into(),
                    });
                }
                // Open/close events preserve short flows. The explicitly
                // observed Requests page also needs fresh byte counters for
                // long-lived connections which have no terminal event yet.
                record_request_history(&mut state, &active);
                state.connections = active;
            }
            Sample::Logs(page) => {
                state.logs.sync_session();
                if page.next_cursor < state.engine_observation.log_cursor {
                    return Err(PawsError::Core("engine log cursor moved backwards".into()));
                }
                for entry in page.entries {
                    if entry.sequence > state.engine_observation.log_cursor {
                        let entry = LogEntry {
                            level: entry.level,
                            message: entry.message,
                            timestamp: entry.timestamp,
                        };
                        // Display queries never archive or consume the
                        // independent recording boundary/cursor.
                        let logs = state.platform_logs.get_or_insert_with(Vec::new);
                        logs.push(entry);
                        if logs.len() > MAX_IN_MEMORY_LOGS {
                            logs.remove(0);
                        }
                    }
                }
                state.engine_observation.log_cursor = page.next_cursor;
                if page.dropped_before.is_some() {
                    issues.push(ObservationIssue {
                        field: "logs",
                        reason: "older log records were evicted from the bounded engine buffer"
                            .into(),
                    });
                }
            }
            Sample::Dns(config, statistics) => {
                match config {
                    Ok(config) => {
                        state.dns.model = config.model;
                        state.dns.listen = config.listen.unwrap_or_default();
                        state.dns.upstreams = config.upstreams;
                        state.dns.fallbacks = config.fallbacks;
                        state.dns.nameserver_policy = config.nameserver_policy;
                        state.dns.configuration_error = None;
                    }
                    Err(error) => {
                        state.dns.configuration_error = Some(error.to_string());
                        issues.push(ObservationIssue {
                            field: "dnsConfiguration",
                            reason: error.to_string(),
                        });
                    }
                }
                match statistics {
                    Ok(statistics) => {
                        state.dns.statistics_available = true;
                        state.dns.handled_packets = statistics.handled_packets;
                        state.dns.cache_hits = statistics.cache_hits;
                        state.dns.cache_misses = statistics.cache_misses;
                        state.dns.recent_queries = statistics.recent_queries;
                        state.dns.statistics_error = None;
                    }
                    Err(error) => {
                        state.dns.statistics_available = false;
                        state.dns.statistics_error = Some(error.to_string());
                        issues.push(ObservationIssue {
                            field: "dnsStatistics",
                            reason: error.to_string(),
                        });
                    }
                }
            }
        }
        if !(scope.alive)() {
            return Err(PawsError::Core(
                "runtime domain query subscription was superseded".into(),
            ));
        }
        let mut projection = self.project_runtime_domain_locked(&state, key)?;
        projection.observation = Some(Observation {
            engine: Some(engine),
            session: Some(attempt),
            issues,
        });
        Ok(projection)
    }
}

fn apply_connection_summary(
    state: &mut CoreState,
    result: Result<EngineConnectionSummary, PawsError>,
    issues: &mut Vec<ObservationIssue>,
) {
    match result {
        Ok(summary) => {
            state.engine_observation.connection_summary = Some(summary);
            state.engine_observation.connection_error = None;
        }
        Err(error) => {
            state.engine_observation.connection_summary = None;
            state.engine_observation.connection_error = Some(error.to_string());
            issues.push(ObservationIssue {
                field: "connectionSummary",
                reason: error.to_string(),
            });
        }
    }
}

fn merge_request_event(state: &mut CoreState, connection: ConnectionSummary, closed: bool) {
    let timestamp = unix_timestamp_string();
    let entry = RequestSummary {
        domain_source: connection.domain_source,
        id: connection.id.clone(),
        host: connection.host,
        domain: connection.domain,
        destination_ip: connection.destination_ip,
        destination_port: connection.destination_port,
        network: connection.network,
        rule: connection.rule,
        rule_payload: connection.rule_payload,
        chains: connection.chains,
        started_at: connection.started_at,
        proxy: connection.proxy,
        upload_bytes: connection.upload_bytes,
        download_bytes: connection.download_bytes,
        active: !closed,
        updated_at: timestamp,
    };
    if let Some(existing) = state
        .request_history
        .iter_mut()
        .find(|entry| entry.id == connection.id)
    {
        *existing = entry;
    } else {
        state.request_history.push_back(entry);
    }
    while state.request_history.len() > MAX_REQUEST_HISTORY {
        state.request_history.pop_front();
    }
}

async fn drain_logs<F: Fn() -> bool>(
    scope: &QueryScope<'_, F>,
    management: &dyn paws_engine::EngineManagement,
    after: u64,
) -> Result<EngineLogPage, PawsError> {
    let mut page = scope.read(management.runtime_logs(after, 512)).await?;
    let watermark = page.high_watermark;
    for _ in 0..32 {
        if page.next_cursor >= watermark {
            return Ok(page);
        }
        let next = scope
            .read(management.runtime_logs(page.next_cursor, 512))
            .await?;
        if next.next_cursor <= page.next_cursor {
            return Err(PawsError::Core("engine log cursor made no progress".into()));
        }
        page.next_cursor = next.next_cursor.min(watermark);
        page.dropped_before = page.dropped_before.or(next.dropped_before);
        page.entries.extend(
            next.entries
                .into_iter()
                .filter(|entry| entry.sequence <= watermark),
        );
    }
    Err(PawsError::Core(
        "engine log backlog exceeds the bounded query budget; no cursor was advanced".into(),
    ))
}

async fn drain_connections<F: Fn() -> bool>(
    scope: &QueryScope<'_, F>,
    management: &dyn paws_engine::EngineManagement,
    after: u64,
) -> Result<EngineConnectionPage, PawsError> {
    let mut page = scope.read(management.connection_events(after, 512)).await?;
    let watermark = page.high_watermark;
    for _ in 0..32 {
        if page.next_cursor >= watermark {
            return Ok(page);
        }
        let next = scope
            .read(management.connection_events(page.next_cursor, 512))
            .await?;
        if next.next_cursor <= page.next_cursor {
            return Err(PawsError::Core(
                "engine connection event cursor made no progress".into(),
            ));
        }
        page.next_cursor = next.next_cursor.min(watermark);
        page.dropped_before = page.dropped_before.or(next.dropped_before);
        page.entries.extend(
            next.entries
                .into_iter()
                .filter(|entry| entry.sequence <= watermark),
        );
    }
    Err(PawsError::Core(
        "engine connection backlog exceeds the bounded query budget; no cursor was advanced".into(),
    ))
}
