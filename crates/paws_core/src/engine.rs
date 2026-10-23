//! Application-owned engine selection and transaction coordination. Backends
//! own native execution; profile, attempt and route-proof state stay in Core.
use super::*;
use paws_engine::{
    EngineCapabilities, EngineCapability, EngineConfig, EngineError, EngineSession, EngineStart,
};

pub(super) struct ManagementTarget {
    pub(super) attempt: String,
    pub(super) revision: u64,
    pub(super) endpoint: Option<paws_engine::EngineControllerEndpoint>,
    pub(super) management: Arc<dyn paws_engine::EngineManagement>,
}

impl ManagementTarget {
    pub(super) fn log_source(&self) -> Result<log_recording::NativeLogSource, PawsError> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| PawsError::Core("native recording endpoint is unavailable".into()))?;
        Ok(log_recording::NativeLogSource {
            attempt: self.attempt.clone(),
            epoch: management_epoch(endpoint),
            config_revision: self.revision,
        })
    }
}

/// Opaque native configuration identity. Credentials never enter status IPC.
pub(super) fn management_epoch(endpoint: &paws_engine::EngineControllerEndpoint) -> String {
    use sha2::{Digest, Sha256};
    let mut epoch = Sha256::new();
    epoch.update(endpoint.address.to_string().as_bytes());
    epoch.update([0]);
    epoch.update(endpoint.secret.as_deref().unwrap_or_default().as_bytes());
    format!("{:x}", epoch.finalize())
}

#[derive(Clone)]
pub(super) struct ActiveEngineSession {
    pub(super) attempt: String,
    pub(super) session: Arc<dyn EngineSession>,
}

pub fn register_engine_process_resolver(resolver: Option<Arc<dyn paws_engine::ProcessResolver>>) {
    paws_engine_go::register_process_resolver(resolver);
}

pub(super) fn engine_error(error: EngineError) -> PawsError {
    match error {
        EngineError::ObservationUnavailable {
            engine,
            domain,
            reason,
        } => PawsError::EngineObservationUnavailable {
            engine,
            domain: format!("{domain:?}"),
            reason,
        },
        EngineError::Unsupported { engine, capability } => PawsError::UnsupportedEngineCapability {
            engine,
            capability: format!("{capability:?}"),
        },
        other => PawsError::Core(other.to_string()),
    }
}

use paws_engine_meow::MeowEngineSession;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparedEngine {
    engine: VpnEngine,
    attempt_id: String,
    config_revision: u64,
    options: VpnOptions,
    #[serde(default)]
    controller_endpoint: Option<paws_engine::EngineControllerEndpoint>,
    #[serde(default)]
    config_json: String,
    #[serde(default)]
    work_dir: PathBuf,
    #[serde(default)]
    init_params: String,
    #[serde(default)]
    setup_params: String,
    #[serde(default)]
    proxy_count: usize,
    #[serde(default)]
    group_count: usize,
    #[serde(default)]
    rule_count: usize,
    #[serde(default)]
    skipped_rule_count: usize,
}
impl PreparedEngine {
    fn native(
        engine: VpnEngine,
        attempt_id: &str,
        config_revision: u64,
        options: VpnOptions,
    ) -> Self {
        Self {
            engine,
            attempt_id: attempt_id.into(),
            config_revision,
            options,
            controller_endpoint: None,
            config_json: String::new(),
            work_dir: PathBuf::new(),
            init_params: String::new(),
            setup_params: String::new(),
            proxy_count: 0,
            group_count: 0,
            rule_count: 0,
            skipped_rule_count: 0,
        }
    }
    fn config(self) -> Result<EngineConfig, PawsError> {
        match self.engine {
            VpnEngine::Meow => Ok(EngineConfig::Meow),
            VpnEngine::Mihomo if !self.init_params.is_empty() && !self.setup_params.is_empty() => {
                Ok(EngineConfig::Mihomo {
                    init_params: self.init_params,
                    setup_params: self.setup_params,
                    controller_endpoint: self.controller_endpoint.ok_or_else(|| {
                        PawsError::Core("prepared Mihomo management endpoint is missing".into())
                    })?,
                })
            }
            VpnEngine::SingBox
                if !self.config_json.is_empty() && !self.work_dir.as_os_str().is_empty() =>
            {
                Ok(EngineConfig::SingBox {
                    config_json: self.config_json,
                    work_dir: self.work_dir,
                })
            }
            _ => Err(PawsError::Core("engine startup plan is incomplete".into())),
        }
    }
}

impl CoreHandle {
    pub(super) async fn select_managed_proxy(
        &self,
        group: &str,
        proxy: &str,
        ManagementTarget {
            attempt,
            revision,
            endpoint,
            management,
        }: ManagementTarget,
    ) -> Result<(), PawsError> {
        self.ensure_management_owner(&attempt, revision, &endpoint)?;
        management
            .select_proxy(group, Some(proxy))
            .await
            .map_err(engine_error)?;
        // Do not make another request after the original owner was replaced.
        self.ensure_management_owner(&attempt, revision, &endpoint)?;
        let observed = management
            .proxy_selection(group)
            .await
            .map_err(engine_error)?;
        let committed = self.record_managed_selection(
            &attempt,
            revision,
            &endpoint,
            group,
            Some(proxy),
            observed,
        )?;
        self.ensure_management_owner(&attempt, committed, &endpoint)?;
        management
            .close_connections(None)
            .await
            .map_err(engine_error)?;
        self.ensure_management_owner(&attempt, committed, &endpoint)
    }

    pub(super) fn record_managed_action(
        &self,
        attempt: &str,
        revision: u64,
        endpoint: &Option<paws_engine::EngineControllerEndpoint>,
        message: String,
        closed_connection: Option<Option<&str>>,
    ) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(&state, attempt, revision, endpoint)?;
        if let Some(connection) = closed_connection {
            match connection {
                Some(id) => state.connections.retain(|connection| connection.id != id),
                None => state.connections.clear(),
            }
        }
        state.logs.push(info_log(message));
        self.publish_runtime_change_locked(&mut state, false, true);
        Ok(())
    }
    pub(super) fn record_managed_selection(
        &self,
        attempt: &str,
        revision: u64,
        endpoint: &Option<paws_engine::EngineControllerEndpoint>,
        group_name: &str,
        proxy: Option<&str>,
        observed: paws_engine::EngineProxySelection,
    ) -> Result<u64, PawsError> {
        let _transaction = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(&state, attempt, revision, endpoint)?;
        if let Some(profile) = state.profiles.active_profile().map(ToOwned::to_owned) {
            state
                .profiles
                .set_selected_proxy(&profile, group_name, proxy.unwrap_or_default())?;
        }
        if let Some(group) = state
            .proxy_groups
            .iter_mut()
            .find(|group| group.name == group_name)
        {
            group.selected = observed.selected;
            group.fixed = observed.fixed;
            for item in &mut group.proxies {
                item.selected = group.selected.as_deref() == Some(item.name.as_str());
            }
        }
        let mode = state.mode;
        let global_proxy = current_global_proxy(&state);
        self.persist_platform_vpn_control_locked(&mut state, mode, global_proxy)?;
        persist_runtime_ui_cache_best_effort(&mut state);
        state.engine_resources_authoritative = true;
        invalidate_exit_location(&mut state);
        state.logs.push(info_log(format!(
            "active engine selection updated for {group_name}"
        )));
        Ok(self
            .publish_config_and_resource_change_locked(&mut state)
            .config_revision)
    }
    pub(super) fn record_managed_delays(
        &self,
        attempt: &str,
        revision: u64,
        endpoint: &Option<paws_engine::EngineControllerEndpoint>,
        provider: Option<&str>,
        delays: &[(String, u16)],
    ) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(&state, attempt, revision, endpoint)?;
        let native_meow =
            state.platform_vpn_engine == Some(VpnEngine::Meow) && self.vpn.is_running();
        for (name, delay) in delays {
            if provider.is_none() {
                if let Some(proxy) = state
                    .tunnel
                    .as_ref()
                    .filter(|_| native_meow)
                    .and_then(|tunnel| tunnel.proxy(name))
                {
                    proxy.health().record_delay(*delay);
                }
                for group in &mut state.proxy_groups {
                    for proxy in &mut group.proxies {
                        if proxy.name == *name {
                            proxy.delay_ms = (*delay > 0).then_some(u32::from(*delay));
                        }
                    }
                }
            }
            if let Some(provider_name) = provider {
                if let Some(proxies) = state
                    .controller_start
                    .as_ref()
                    .filter(|_| native_meow)
                    .and_then(|config| config.proxy_providers.get(provider_name))
                {
                    for proxy in proxies.proxies() {
                        if proxy.name() == name {
                            proxy.health().record_delay(*delay);
                        }
                    }
                }
                for provider in &mut state.providers {
                    if provider.name == provider_name && provider.provider_type == "proxy" {
                        for proxy in &mut provider.members {
                            if proxy.name == *name {
                                proxy.delay_ms = (*delay > 0).then_some(u32::from(*delay));
                                // This branch consumes an actual probe receipt,
                                // not an unmeasured roster/cache projection.
                                proxy.alive = Some(*delay > 0);
                            }
                        }
                    }
                }
            }
        }
        state.engine_resources_authoritative = true;
        self.publish_resource_change_locked(&mut state);
        Ok(())
    }
    pub(super) fn native_controller_secret(&self, attempt: &str) -> Result<String, PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        ensure_platform_attempt_active(&state, attempt)?;
        // Each prepared native configuration gets a fresh management epoch.
        // Reusing a credential across reloads would let an already-issued
        // request mutate the new configuration despite the Core receipt fence.
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|error| {
            PawsError::Core(format!(
                "native controller credential generation failed: {error}"
            ))
        })?;
        use std::fmt::Write;
        let mut secret = String::with_capacity(64);
        for byte in bytes {
            let _ = write!(secret, "{byte:02x}");
        }
        Ok(secret)
    }
    pub(super) fn runtime_management(&self) -> Result<Option<ManagementTarget>, PawsError> {
        let (engine, endpoint, attempt, revision) = {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            if !state.platform_vpn_running {
                return Ok(None);
            }
            (
                state
                    .platform_vpn_engine
                    .ok_or_else(|| PawsError::Core("active engine is unknown".into()))?,
                state.platform_controller_endpoint.clone(),
                state.platform_start_attempt_id.clone(),
                state.config_revision,
            )
        };
        let active = self
            .active_engine
            .lock()
            .map_err(|_| PawsError::Core("active engine lock poisoned".into()))?
            .clone();
        if let Some(active) = active.filter(|active| {
            active.attempt == attempt
                && active.session.engine() == engine
                && active.session.is_running()
        }) {
            if let Some(binding) = active.session.management_binding() {
                if Some(&binding.endpoint) != endpoint.as_ref() {
                    return Err(PawsError::Core(
                        "engine management configuration is being replaced".into(),
                    ));
                }
                self.ensure_management_owner(&attempt, revision, &endpoint)?;
                return Ok(Some(ManagementTarget {
                    attempt,
                    revision,
                    endpoint,
                    management: binding.client,
                }));
            }
            // Standalone Rust callers still own an actual Meow session, but
            // do not start Extension-only HTTP services. Use that session's
            // typed port; never a loaded preview tunnel or a Go fallback.
            if engine == VpnEngine::Meow && endpoint.is_none() && self.platform_ipc()?.is_none() {
                if let Some(management) = active.session.management() {
                    self.ensure_management_owner(&attempt, revision, &endpoint)?;
                    return Ok(Some(ManagementTarget {
                        attempt,
                        revision,
                        endpoint,
                        management,
                    }));
                }
            }
        }
        let controller = endpoint.clone().ok_or_else(|| {
            PawsError::Core("active engine management endpoint unavailable".into())
        })?;
        let management = Arc::new(
            paws_engine_go::ControllerClient::new(engine, controller).map_err(engine_error)?,
        );
        self.ensure_management_owner(&attempt, revision, &endpoint)?;
        Ok(Some(ManagementTarget {
            attempt,
            revision,
            endpoint,
            management,
        }))
    }
    pub(super) fn ensure_management_owner(
        &self,
        attempt: &str,
        revision: u64,
        endpoint: &Option<paws_engine::EngineControllerEndpoint>,
    ) -> Result<(), PawsError> {
        let mut state = self.lock_state()?;
        self.sync_platform_vpn_state_locked(&mut state);
        self.ensure_captured_management_locked(&state, attempt, revision, endpoint)
    }
    pub(super) fn ensure_captured_management_locked(
        &self,
        state: &CoreState,
        attempt: &str,
        revision: u64,
        endpoint: &Option<paws_engine::EngineControllerEndpoint>,
    ) -> Result<(), PawsError> {
        if !state.platform_vpn_running
            || state.platform_start_attempt_id != attempt
            || &state.platform_controller_endpoint != endpoint
        {
            return Err(PawsError::Core(
                "native management result belongs to an obsolete session or configuration epoch"
                    .into(),
            ));
        }
        Self::ensure_config_revision_locked(state, revision)?;
        // Native reload publishes its committed binding before Core publishes
        // the new platform endpoint. Also fence that small publication window.
        // CoreState -> active binding -> backend binding is the sole lock
        // order; start/stop never acquire CoreState while holding those locks.
        let active = self
            .active_engine
            .lock()
            .map_err(|_| PawsError::Core("active engine lock poisoned".into()))?
            .clone();
        if let Some(active) =
            active.filter(|active| active.attempt == attempt && active.session.is_running())
        {
            if active
                .session
                .management_binding()
                .as_ref()
                .map(|binding| &binding.endpoint)
                != endpoint.as_ref()
            {
                return Err(PawsError::Core(
                    "native management configuration epoch was replaced".into(),
                ));
            }
        }
        Ok(())
    }
    pub(super) fn reject_external_direct_operation(
        &self,
        capability: EngineCapability,
    ) -> Result<(), PawsError> {
        let state = self.lock_state()?;
        if state.platform_vpn_running {
            if let Some(engine @ (VpnEngine::Mihomo | VpnEngine::SingBox)) =
                state.platform_vpn_engine
            {
                return Err(engine_error(EngineError::Unsupported {
                    engine,
                    capability,
                }));
            }
        }
        Ok(())
    }
    pub fn engine_catalog(&self) -> Result<Vec<EngineCapabilities>, PawsError> {
        Ok(vec![
            MeowEngineSession::descriptor(),
            paws_engine_go::GoEngineSession::descriptor(VpnEngine::Mihomo).map_err(engine_error)?,
            paws_engine_go::GoEngineSession::descriptor(VpnEngine::SingBox)
                .map_err(engine_error)?,
        ])
    }
    pub fn engine_tun_mtu(&self, options_json: &str) -> Result<u16, PawsError> {
        let options: VpnOptions = from_json(options_json)?;
        // Native TUN configs consume the same requested value. A larger OS
        // MTU can truncate valid packets in an MTU-sized native read buffer.
        Ok(options.mtu)
    }
    pub async fn prepare_engine(
        &self,
        options_json: &str,
        attempt_id: &str,
    ) -> Result<String, PawsError> {
        let engine = VpnEngine::try_from(self.resolve_vpn_engine(options_json)?.as_str())?;
        if engine == VpnEngine::Meow {
            self.prepare_platform_vpn(attempt_id).await?;
        }
        let _config = self.config_reload_lock.lock().await;
        let revision = self.lock_state()?.config_revision;
        let mut plan =
            PreparedEngine::native(engine, attempt_id, revision, from_json(options_json)?);
        match engine {
            VpnEngine::Meow => {}
            VpnEngine::Mihomo => {
                let prepared = self.prepare_mihomo_vpn(options_json, attempt_id)?;
                plan.controller_endpoint = Some(prepared.controller_endpoint);
                plan.init_params = prepared.init_params;
                plan.setup_params = prepared.setup_params;
            }
            VpnEngine::SingBox => {
                let prepared = self.prepare_sing_box_vpn(options_json, attempt_id)?;
                plan.controller_endpoint = Some(prepared.controller_endpoint);
                plan.config_json = prepared.config_json;
                plan.work_dir = prepared.work_dir;
                plan.proxy_count = prepared.proxy_count;
                plan.group_count = prepared.group_count;
                plan.rule_count = prepared.rule_count;
                plan.skipped_rule_count = prepared.skipped_rule_count;
            }
            VpnEngine::Auto => {
                return Err(PawsError::Core(
                    "engine selection remained unresolved".into(),
                ))
            }
        }
        serde_json::to_string(&plan).map_err(|error| PawsError::Core(error.to_string()))
    }
    fn validate_engine_plan(
        &self,
        options_json: &str,
        attempt_id: &str,
        plan_json: &str,
    ) -> Result<
        (
            VpnEngine,
            EngineConfig,
            u64,
            Option<paws_engine::EngineControllerEndpoint>,
        ),
        PawsError,
    > {
        let plan: PreparedEngine = from_json(plan_json)?;
        let selected = VpnEngine::try_from(self.resolve_vpn_engine(options_json)?.as_str())?;
        if plan.engine != selected || plan.attempt_id != attempt_id {
            return Err(PawsError::Core(
                "prepared engine plan belongs to a different session".into(),
            ));
        }
        if plan.options != from_json::<VpnOptions>(options_json)? {
            return Err(PawsError::Core(
                "prepared engine plan has stale VPN options".into(),
            ));
        }
        let revision = plan.config_revision;
        let controller = plan.controller_endpoint.clone();
        Ok((selected, plan.config()?, revision, controller))
    }
    pub async fn start_engine(
        self: &Arc<Self>,
        fd: i32,
        net_id: i32,
        options_json: &str,
        attempt_id: &str,
        plan_json: &str,
    ) -> Result<(), PawsError> {
        let (engine, config, expected_revision, controller) =
            self.validate_engine_plan(options_json, attempt_id, plan_json)?;
        let source_started_at = now_unix_nanos();
        if engine == VpnEngine::Meow {
            self.start_platform_vpn_checked(fd, options_json, attempt_id, Some(expected_revision))
                .await?;
            self.engine_observation_lane.resume();
            self.initialize_native_recording_best_effort(source_started_at)
                .await;
            return Ok(());
        }
        let _operation = self.vpn_operation_lock.lock().await;
        let _config = self.config_reload_lock.lock().await;
        {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            ensure_platform_attempt_active(&state, attempt_id)?;
            Self::ensure_config_revision_locked(&state, expected_revision)?;
        }
        self.stop_active_engine().await?;
        let session: Arc<dyn EngineSession> = Arc::new(
            paws_engine_go::GoEngineSession::with_controller(
                engine,
                controller.ok_or_else(|| {
                    PawsError::Core("prepared native controller endpoint missing".into())
                })?,
            )
            .map_err(engine_error)?,
        );
        *self
            .active_engine
            .lock()
            .map_err(|_| PawsError::Core("active engine lock poisoned".into()))? =
            Some(ActiveEngineSession {
                attempt: attempt_id.to_owned(),
                session: session.clone(),
            });
        set_engine_log_sink(session.runtime_log_sink());
        let options: VpnOptions = from_json(options_json)?;
        validate_supported_vpn_options(&options)?;
        session
            .start(EngineStart {
                tun_fd: fd,
                underlying_network: net_id,
                options,
                config,
            })
            .await
            .map_err(engine_error)?;
        if let Err(error) = self.start_external_platform_vpn(options_json, attempt_id) {
            return match self.stop_active_engine().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(PawsError::Core(format!(
                    "{error}; engine cleanup failed: {cleanup}"
                ))),
            };
        }
        self.engine_observation_lane.resume();
        self.initialize_native_recording_best_effort(source_started_at)
            .await;
        Ok(())
    }
    pub async fn stop_engine(&self, attempt_id: &str) -> Result<(), PawsError> {
        self.stop_platform_vpn(attempt_id).await.map(|_| ())
    }
    pub async fn reload_engine(
        &self,
        options_json: &str,
        attempt_id: &str,
        plan_json: &str,
        net_id: i32,
    ) -> Result<(), PawsError> {
        let (engine, config, expected_revision, controller) =
            self.validate_engine_plan(options_json, attempt_id, plan_json)?;
        let source_started_at = now_unix_nanos();
        let _operation = self.vpn_operation_lock.lock().await;
        let _config = self.config_reload_lock.lock().await;
        {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            ensure_platform_attempt_active(&state, attempt_id)?;
            Self::ensure_config_revision_locked(&state, expected_revision)?;
        }
        if engine == VpnEngine::Meow {
            // Session provider leases are retired on stop. Every real reload
            // builds fresh Live objects, never rebinds the previous registry.
            let profile = self
                .lock_state()?
                .profiles
                .active_profile()
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    PawsError::Core("active profile missing during Meow reload".into())
                })?;
            self.reload_config_for_purpose(&profile, MeowLoadPurpose::Runtime)
                .await?;
            let fd = self
                .vpn
                .fd()
                .ok_or_else(|| engine_error(EngineError::NotRunning))?;
            let (tunnel, sniffer, options) = {
                let state = self.lock_state()?;
                (
                    state.tunnel.clone(),
                    state.sniffer_config.clone(),
                    from_json(options_json)?,
                )
            };
            let tunnel =
                tunnel.ok_or_else(|| PawsError::Core("active meow tunnel is not loaded".into()))?;
            self.start_meow_session(fd, options, tunnel, sniffer)
                .await?;
            let services = self.active_engine_services()?;
            {
                let mut state = self.lock_state()?;
                ensure_platform_attempt_active(&state, attempt_id)?;
                Self::ensure_config_revision_locked(&state, expected_revision)?;
                state.platform_controller_endpoint = services.controller_endpoint;
                state.platform_app_http_proxy = services.app_http_proxy;
                state.platform_public_controller_addr = services.public_controller_addr;
                self.persist_platform_vpn_state_locked(&mut state)?;
            }
            self.engine_observation_lane.resume();
            self.initialize_native_recording_best_effort(source_started_at)
                .await;
            return Ok(());
        }
        let session = self
            .active_engine_session()?
            .ok_or_else(|| engine_error(EngineError::NotRunning))?;
        if session.engine() != engine {
            return Err(PawsError::Core(
                "cannot switch an engine during a configuration reload".into(),
            ));
        }
        self.engine_observation_lane.cancel_and_join().await;
        session.reload(config, net_id).await.map_err(engine_error)?;
        if controller != session.controller_endpoint() {
            return Err(PawsError::Core(
                "native reload did not publish its prepared management epoch".into(),
            ));
        }
        let services = session.services();
        {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            ensure_platform_attempt_active(&state, attempt_id)?;
            Self::ensure_config_revision_locked(&state, expected_revision)?;
            state.platform_controller_endpoint = controller;
            state.platform_app_http_proxy = services.app_http_proxy;
            state.platform_public_controller_addr = services.public_controller_addr;
            state.vpn_options = from_json(options_json)?;
            self.persist_platform_vpn_state_locked(&mut state)?;
        }
        self.engine_observation_lane.resume();
        self.initialize_native_recording_best_effort(source_started_at)
            .await;
        Ok(())
    }
    async fn initialize_native_recording_best_effort(&self, started_at: u128) {
        let captured = (|| -> Result<_, PawsError> {
            let target = self.runtime_management()?;
            let mut state = self.lock_state()?;
            state.logs.sync_session();
            Ok(target.zip(state.logs.session_id().map(ToOwned::to_owned)))
        })();
        let outcome = self.initialize_native_log_recording(started_at).await;
        if let Ok(Some((target, nonce))) = captured {
            let _ = self.record_native_archive_outcome(&target, &nonce, outcome.as_ref().err());
        }
    }
    /// Counter sampling publishes immediately and never awaits management IO
    /// or the archive writer. It is the Extension's one-second traffic lane.
    pub async fn poll_engine_traffic(&self) -> Result<(), PawsError> {
        let session = self
            .active_engine_session()?
            .ok_or_else(|| engine_error(EngineError::NotRunning))?;
        let ManagementTarget {
            attempt,
            revision,
            endpoint,
            ..
        } = self.runtime_management()?.ok_or(PawsError::VpnNotRunning)?;
        let sample = session.traffic().map_err(engine_error);
        let mut failure = None;
        {
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(&state, &attempt, revision, &endpoint)?;
            if !self
                .active_engine_session()?
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &session))
            {
                return Err(PawsError::Core(
                    "late traffic belongs to an obsolete native session".into(),
                ));
            }
            match sample {
                Ok(sample) => {
                    let upload_delta = sample
                        .upload_total
                        .saturating_sub(state.traffic.upload_bytes);
                    let download_delta = sample
                        .download_total
                        .saturating_sub(state.traffic.download_bytes);
                    if let Some(profile) = state.profiles.active_profile().map(ToOwned::to_owned) {
                        state.profiles.add_profile_traffic(
                            &profile,
                            upload_delta,
                            download_delta,
                        )?;
                    }
                    state.traffic.upload_speed = sample.upload_speed;
                    state.traffic.download_speed = sample.download_speed;
                    state.traffic.upload_bytes = sample.upload_total;
                    state.traffic.download_bytes = sample.download_total;
                    state.traffic.tun_upload_speed = sample.upload_speed;
                    state.traffic.tun_download_speed = sample.download_speed;
                    state.traffic.tun_upload_bytes = sample.upload_total;
                    state.traffic.tun_download_bytes = sample.download_total;
                    if session.engine() == VpnEngine::Meow {
                        state.traffic.meow_upload_speed = sample.upload_speed;
                        state.traffic.meow_download_speed = sample.download_speed;
                        state.traffic.meow_upload_bytes = sample.upload_total;
                        state.traffic.meow_download_bytes = sample.download_total;
                    }
                    record_traffic_history(&mut state);
                }
                Err(error) => failure = Some(error),
            }
            self.publish_runtime_change_locked(&mut state, false, true);
        }
        self.persist_vpn_telemetry()?;
        failure.map_or(Ok(()), Err)
    }

    /// Independent, non-queuing metadata/archive lane. Stop/reload suspends,
    /// cancels and joins it before retiring the native owner.
    pub async fn poll_engine_observations(&self) -> Result<(), PawsError> {
        let Ok(_gate) = self.engine_observation_lane.gate.try_lock() else {
            return Ok(());
        };
        let Some(ticket) = self.engine_observation_lane.ticket() else {
            return Ok(());
        };
        let Some(target) = self.runtime_management()? else {
            return Ok(());
        };
        tokio::select! {
            biased;
            _=self.engine_observation_lane.cancelled(ticket)=>Ok(()),
            result=self.poll_engine_observations_for(&target,ticket)=>result,
        }
    }
    async fn poll_engine_observations_for(
        &self,
        target: &ManagementTarget,
        ticket: u64,
    ) -> Result<(), PawsError> {
        let revisions = tokio::time::timeout(
            Duration::from_secs(4),
            target.management.runtime_revisions(),
        )
        .await
        .map_err(|_| PawsError::Core("engine revision observation timed out".into()))
        .and_then(|result| result.map_err(engine_error));
        let mut failure = None;
        let recording = {
            if !self.engine_observation_lane.current(ticket) {
                return Ok(());
            }
            let mut state = self.lock_state()?;
            self.sync_platform_vpn_state_locked(&mut state);
            self.ensure_captured_management_locked(
                &state,
                &target.attempt,
                target.revision,
                &target.endpoint,
            )?;
            if state.engine_observation.attempt != target.attempt
                || state.engine_observation.endpoint != target.endpoint
            {
                state.engine_observation = runtime_observation::EngineObservationState {
                    attempt: target.attempt.clone(),
                    endpoint: target.endpoint.clone(),
                    ..Default::default()
                };
            }
            match revisions {
                Ok(revisions) => {
                    let changed = state
                        .engine_observation
                        .revisions
                        .as_ref()
                        .is_none_or(|old| {
                            old.groups != revisions.groups
                                || old.providers != revisions.providers
                                || old.rules != revisions.rules
                        });
                    state.engine_observation.revisions = Some(revisions);
                    if changed {
                        self.publish_resource_change_locked(&mut state);
                    }
                }
                Err(error) => failure = Some(error),
            }
            // Archive/session maintenance cannot block the counter lane's
            // mutex or IPC publication while waiting for remote observations.
            state.logs.sync_session();
            if let Ok(mut runtime) = RUNTIME_LOGS.lock() {
                runtime.sync(state.logs.root());
            }
            state.logs.session_id().map(ToOwned::to_owned)
        };
        self.persist_vpn_telemetry()?;
        if let Some(nonce) = recording.filter(|_| self.engine_observation_lane.current(ticket)) {
            let outcome = self.consume_engine_runtime_logs_for(target).await;
            self.record_native_archive_outcome(target, &nonce, outcome.as_ref().err())?;
            self.persist_vpn_telemetry()?;
            if let Err(error) = outcome {
                failure = failure.or(Some(error));
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub fn trim_engine_memory(&self) -> Result<(), PawsError> {
        let session = self
            .active_engine_session()?
            .ok_or_else(|| engine_error(EngineError::NotRunning))?;
        // This is an optional maintenance request. Unsupported is visible in
        // capabilities; callers must not mistake it for a successful trim.
        session.trim_memory().map_err(engine_error)
    }
    pub fn engine_can_trim_memory(&self) -> Result<bool, PawsError> {
        Ok(self.active_engine_session()?.is_some_and(|session| {
            session
                .capabilities()
                .supports(EngineCapability::TrimMemory)
        }))
    }
    pub(super) fn active_engine_session(
        &self,
    ) -> Result<Option<Arc<dyn EngineSession>>, PawsError> {
        self.active_engine
            .lock()
            .map(|slot| slot.as_ref().map(|active| active.session.clone()))
            .map_err(|_| PawsError::Core("active engine lock poisoned".into()))
    }
    pub(super) async fn start_meow_session(
        &self,
        fd: i32,
        options: VpnOptions,
        tunnel: Tunnel,
        sniffer: SnifferConfig,
    ) -> Result<u64, PawsError> {
        self.stop_active_engine().await?;
        let services_plan = {
            let state = self.lock_state()?;
            paws_engine::EngineServicePlan {
                enabled: self
                    .platform_ipc()?
                    .is_some_and(|platform| !platform.is_ui()),
                network_protected: state.platform_network_protected,
                network: paws_engine::EngineNetworkServices {
                    ports: state.network_ports,
                    access: state.controller_access.clone(),
                },
                benchmark_port: engine_policy::benchmark_port(VpnEngine::Meow),
            }
        };
        let (proxy_providers, rule_providers, auth) = {
            let state = self.lock_state()?;
            let config = state.controller_start.as_ref().ok_or_else(|| {
                PawsError::Core("Meow runtime configuration is unavailable".into())
            })?;
            (
                config.proxy_providers.clone(),
                config.rule_providers.clone(),
                config.auth.clone(),
            )
        };
        let session: Arc<dyn EngineSession> = Arc::new(MeowEngineSession::new(
            self.vpn.clone(),
            paws_engine_meow::MeowRuntimeConfig {
                tunnel,
                sniffer,
                proxy_providers,
                rule_providers,
                auth,
            },
            services_plan,
        ));
        let attempt = self.lock_state()?.platform_start_attempt_id.clone();
        *self
            .active_engine
            .lock()
            .map_err(|_| PawsError::Core("active engine lock poisoned".into()))? =
            Some(ActiveEngineSession {
                attempt,
                session: session.clone(),
            });
        set_engine_log_sink(session.runtime_log_sink());
        // Only the actual Meow owner publishes the process resolver. Preview
        // configurations must not replace it, and stop/replacement clears it.
        let resolver = self
            .lock_state()?
            .controller_start
            .as_ref()
            .and_then(|config| config.host_resolver.clone());
        if let Some(resolver) = resolver {
            meow_common::set_host_resolver(resolver);
        } else {
            meow_common::clear_host_resolver();
        }
        let result = session
            .start(EngineStart {
                tun_fd: fd,
                underlying_network: 0,
                options,
                config: EngineConfig::Meow,
            })
            .await
            .map_err(engine_error);
        if result.is_err() {
            meow_common::clear_host_resolver();
        }
        self.lock_state()?.meow_runtime_ready = result.is_ok();
        result
    }
    pub(super) fn active_engine_services(&self) -> Result<paws_engine::EngineServices, PawsError> {
        Ok(self
            .active_engine_session()?
            .map(|session| session.services())
            .unwrap_or_default())
    }
    pub(super) fn stop_engine_services(&self) -> Result<(), PawsError> {
        if let Some(session) = self.active_engine_session()? {
            session.stop_services().map_err(engine_error)?;
        }
        Ok(())
    }
    pub(super) fn stop_engine_public_listener(&self) -> Result<(), PawsError> {
        if let Some(session) = self.active_engine_session()? {
            session.stop_public_listener().map_err(engine_error)?;
        }
        Ok(())
    }
    pub(super) async fn stop_active_engine(&self) -> Result<(), PawsError> {
        self.engine_observation_lane.cancel_and_join().await;
        let Some(session) = self.active_engine_session()? else {
            return Ok(());
        };
        session.stop().await.map_err(engine_error)?;
        let removed = {
            let mut active = self
                .active_engine
                .lock()
                .map_err(|_| PawsError::Core("active engine lock poisoned".into()))?;
            if active
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(&current.session, &session))
            {
                *active = None;
                set_engine_log_sink(None);
                true
            } else {
                false
            }
        };
        if removed && session.engine() == VpnEngine::Meow {
            meow_common::clear_host_resolver();
            self.lock_state()?.meow_runtime_ready = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared_core() -> (Arc<CoreHandle>, PreparedEngine, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "paws-engine-plan-{}-{}",
            std::process::id(),
            now_unix_nanos()
        ));
        let core = Arc::new(CoreHandle::new_with_profile_root(&root));
        let mut state = core.lock_state().unwrap();
        let profile_id = state
            .profiles
            .import_profile_content("direct", "test", paws_profile::default_runtime_yaml(), None)
            .unwrap();
        state.profiles.set_active(&profile_id).unwrap();
        state.platform_start_attempt_id = "attempt-a".into();
        state.platform_start_outcome = PlatformStartOutcome::Pending;
        state.platform_extension_attached = true;
        let options = VpnOptions {
            engine: VpnEngine::Mihomo,
            ..VpnOptions::default()
        };
        let mut plan = PreparedEngine::native(
            VpnEngine::Mihomo,
            "attempt-a",
            state.config_revision,
            options,
        );
        plan.init_params = "{}".into();
        plan.setup_params = "{}".into();
        plan.controller_endpoint = Some(paws_engine::EngineControllerEndpoint {
            address: "127.0.0.1:19090".parse().unwrap(),
            secret: Some("synthetic-plan-epoch".into()),
        });
        drop(state);
        (core, plan, root)
    }

    #[test]
    fn platform_tun_mtu_matches_the_resolved_engine_input() {
        let (core, _, root) = prepared_core();
        {
            let mut state = core.lock_state().unwrap();
            let id = state.profiles.active_profile().unwrap().to_owned();
            let mut raw: serde_yaml::Value =
                serde_yaml::from_str(&state.profiles.raw_yaml(&id).unwrap()).unwrap();
            raw["tun"] = serde_yaml::from_str("{mtu: 7777}").unwrap();
            state
                .profiles
                .replace_profile_content(&id, &serde_yaml::to_string(&raw).unwrap())
                .unwrap();
            // This public settings path stores a real profile overlay. Both
            // raw YAML and overlays are applied before the final patch_tun.
            state
                .profiles
                .set_profile_vpn_config_with_engine(
                    &id,
                    "mihomo".into(),
                    false,
                    true,
                    false,
                    VpnOptions::default().stack,
                )
                .unwrap();
            let merged: serde_yaml::Value =
                serde_yaml::from_str(&state.profiles.raw_yaml(&id).unwrap()).unwrap();
            assert_eq!(merged["tun"]["mtu"].as_u64(), Some(7777));
            assert!(merged["tun"]["stack"].as_str().is_some());
        }
        for mtu in [1280, 1500, 2816, 9000] {
            for engine in [VpnEngine::Meow, VpnEngine::Mihomo, VpnEngine::SingBox] {
                let options = VpnOptions {
                    engine,
                    mtu,
                    ..Default::default()
                };
                let options_json = to_json(&options).unwrap();
                assert_eq!(
                    core.resolve_vpn_engine(&options_json).unwrap(),
                    engine.as_str(),
                    "the fixture must exercise the requested backend"
                );
                let platform_mtu = core.engine_tun_mtu(&options_json).unwrap();
                assert_eq!(platform_mtu, mtu, "{engine:?}");
                if engine == VpnEngine::SingBox {
                    // Exercise Core's actual prepared config, including its
                    // production renderer and controller setup, not a second
                    // test-only mapping of the JSON's MTU.
                    let plan = core
                        .prepare_sing_box_vpn(&options_json, "attempt-a")
                        .unwrap();
                    let config: serde_json::Value = from_json(&plan.config_json).unwrap();
                    let tun = config["inbounds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|inbound| inbound["type"] == "tun")
                        .unwrap();
                    assert_eq!(tun["mtu"].as_u64(), Some(u64::from(platform_mtu)));
                } else if engine == VpnEngine::Mihomo {
                    let plan = core.prepare_mihomo_vpn(&options_json, "attempt-a").unwrap();
                    let init: serde_json::Value = from_json(&plan.init_params).unwrap();
                    let yaml = fs::read_to_string(
                        Path::new(init["home-dir"].as_str().unwrap()).join("config.yaml"),
                    )
                    .unwrap();
                    let config: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
                    assert_eq!(config["tun"]["mtu"].as_u64(), Some(u64::from(platform_mtu)));
                }
            }
        }

        // A profile omitting tun.mtu receives the production default before
        // rendering; neither the native bridge nor the OS guesses one later.
        let defaults =
            paws_profile::vpn_options_from_yaml("proxies: []\nrules: [MATCH,DIRECT]").unwrap();
        assert_eq!(defaults.mtu, 1500);
        let options = to_json(&defaults).unwrap();
        assert_eq!(core.resolve_vpn_engine(&options).unwrap(), "mihomo");
        assert_eq!(core.engine_tun_mtu(&options).unwrap(), 1500);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reserved_native_service_ports_are_rejected_before_profile_persistence() {
        let (core, _, root) = prepared_core();
        let (profile, revision, original) = {
            let state = core.lock_state().unwrap();
            let profile = state.profiles.active_profile().unwrap().to_owned();
            let original = state.profiles.raw_yaml(&profile).unwrap();
            (profile, state.config_revision, original)
        };
        for port in [18490, 18491, 18492, 19090] {
            for mixed in [false, true] {
                let mut ports = NetworkPortConfig::default();
                if mixed {
                    ports.mixed_enabled = true;
                    ports.mixed_port = port;
                } else {
                    ports.controller_enabled = true;
                    ports.controller_port = port;
                }
                assert!(core
                    .set_profile_network_config_checked(&profile, revision, ports, false)
                    .await
                    .is_err());
                let state = core.lock_state().unwrap();
                assert_eq!(state.config_revision, revision);
                assert_eq!(state.profiles.raw_yaml(&profile).unwrap(), original);
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn same_attempt_cannot_start_a_plan_from_an_older_configuration() {
        let (core, plan, root) = prepared_core();
        core.lock_state().unwrap().config_revision += 1;
        let options = to_json(&plan.options).unwrap();
        let plan = to_json(&plan).unwrap();
        let error = core
            .start_engine(9, 42, &options, "attempt-a", &plan)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("revision"), "{error}");
        assert!(core.active_engine_session().unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn same_engine_with_changed_vpn_options_rejects_the_prepared_plan() {
        let (core, plan, root) = prepared_core();
        let mut options = plan.options.clone();
        options.mtu += 1;
        let error = core
            .start_engine(
                9,
                42,
                &to_json(&options).unwrap(),
                "attempt-a",
                &to_json(&plan).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stale VPN options"), "{error}");
        assert!(core.active_engine_session().unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    async fn management_server(
        gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> (
        paws_engine::EngineControllerEndpoint,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{
            extract::Path,
            http::{HeaderMap, StatusCode},
            routing::get,
            Json, Router,
        };
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let probe = move |Path(name): Path<String>, headers: HeaderMap| {
            let gate = gate.clone();
            async move {
                assert_eq!(headers.get("authorization").unwrap(), "Bearer test-owner");
                if let Some((entered, release)) = gate {
                    entered.notify_one();
                    release.notified().await;
                }
                if name == "failed" {
                    (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({"message":"native probe failed"})),
                    )
                } else {
                    (StatusCode::OK, Json(serde_json::json!({"delay":37})))
                }
            }
        };
        let roster = Arc::new(AtomicBool::new(false));
        let groups_roster = roster.clone();
        let members_roster = roster.clone();
        let requested_group_roster = roster.clone();
        let app = Router::new()
            .route("/paws/observations/proxy-groups/{name}", get(move |Path(name):Path<String>,headers:HeaderMap| { let roster=requested_group_roster.clone(); async move {
                assert_eq!(name,"group"); assert_eq!(headers.get("authorization").unwrap(),"Bearer test-owner");
                let names=if roster.load(Ordering::Acquire) { vec!["new-node"] } else { vec!["node","failed"] };
                Json(serde_json::json!({"name":"group","groupType":"Selector","selected":names[0],"fixed":null,"proxies":names.iter().map(|name|serde_json::json!({"name":name,"proxyType":"Direct","delayMs":null,"selected":*name==names[0]})).collect::<Vec<_>>()}))
            }}))
            .route("/proxies", get(move || { let roster = groups_roster.clone(); async move {
                let all = if roster.load(Ordering::Acquire) { vec!["new-node"] } else { vec!["node", "failed"] };
                Json(serde_json::json!({"proxies":{"group":{"type":"Selector","now":all[0],"all":all},"new-node":{"type":"Direct","history":[{"delay":63}]},"node":{"type":"Direct","history":[{"delay":63}]}}}))
            }}))
            .route("/proxies/{name}/delay", get(probe))
            .route("/proxies/{name}", get(|| async { Json(serde_json::json!({"now":"failed","fixed":""})) }).put(|Json(value): Json<serde_json::Value>| async move {
                if value["name"] == "bad-node" { StatusCode::BAD_REQUEST } else { StatusCode::NO_CONTENT }
            }).delete(|| async { StatusCode::NO_CONTENT }))
            .route("/connections", axum::routing::delete(|| async { StatusCode::NO_CONTENT }))
            .route("/providers/proxies/{name}/{proxy}/healthcheck", get(|| async { Json(serde_json::json!({"delay":51})) }))
            .route("/providers/proxies/{name}/healthcheck", get(|| async { StatusCode::NO_CONTENT }))
            .route("/providers/proxies/{name}", get(move || { let roster = members_roster.clone(); async move {
                let name = if roster.load(Ordering::Acquire) { "new-node" } else { "node" };
                Json(serde_json::json!({"proxies":[{"name":name,"type":"Direct","alive":true,"history":[{"delay":63}]}]}))
            }}).put(move || { let roster = roster.clone(); async move { roster.store(true, Ordering::Release); StatusCode::NO_CONTENT } }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (
            paws_engine::EngineControllerEndpoint {
                address,
                secret: Some("test-owner".into()),
            },
            task,
        )
    }

    async fn managed_core(
        endpoint: paws_engine::EngineControllerEndpoint,
    ) -> (Arc<CoreHandle>, PathBuf) {
        let (core, _, root) = prepared_core();
        let config = load_meow_config("rules:\n  - MATCH,DIRECT\n")
            .await
            .unwrap();
        let mut state = core.lock_state().unwrap();
        state.engine_loaded = true;
        state.platform_vpn_running = true;
        state.platform_vpn_state_updated_at = now_unix_nanos();
        state.platform_start_outcome = PlatformStartOutcome::Connected;
        state.platform_vpn_engine = Some(VpnEngine::Mihomo);
        state.platform_controller_endpoint = Some(endpoint);
        // This is a configuration model, deliberately not the running engine.
        state.tunnel = Some(tunnel_from_config(config, RuntimeMode::Direct));
        state.proxy_groups = vec![ProxyGroup {
            name: "group".into(),
            group_type: "Selector".into(),
            selected: Some("node".into()),
            fixed: None,
            proxies: ["node", "failed"]
                .into_iter()
                .map(|name| ProxyItem {
                    name: name.into(),
                    proxy_type: "Direct".into(),
                    delay_ms: Some(999),
                    selected: name == "node",
                })
                .collect(),
        }];
        state.providers = vec![serde_json::from_value(serde_json::json!({
            "name":"provider", "providerType":"proxy", "path":null, "url":null, "vehicleType":null,
            "members":[{"name":"node","proxyType":"Direct","alive":false,"delayMs":null}]
        }))
        .unwrap()];
        drop(state);
        (core, root)
    }

    struct SampleSession {
        endpoint: paws_engine::EngineControllerEndpoint,
        traffic_fails: bool,
        stall_revision: bool,
        traffic_sequence: Option<Arc<AtomicU64>>,
    }
    #[async_trait::async_trait]
    impl EngineSession for SampleSession {
        fn engine(&self) -> VpnEngine {
            VpnEngine::Mihomo
        }
        fn capabilities(&self) -> EngineCapabilities {
            paws_engine_go::GoEngineSession::descriptor(self.engine()).unwrap()
        }
        fn controller_endpoint(&self) -> Option<paws_engine::EngineControllerEndpoint> {
            Some(self.endpoint.clone())
        }
        fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
            self.management()
                .map(|client| paws_engine::EngineManagementBinding {
                    client,
                    endpoint: self.endpoint.clone(),
                })
        }
        fn management(&self) -> Option<Arc<dyn paws_engine::EngineManagement>> {
            if self.stall_revision {
                return Some(Arc::new(StalledManagement));
            }
            Some(Arc::new(
                paws_engine_go::ControllerClient::new(self.engine(), self.endpoint.clone())
                    .unwrap(),
            ))
        }
        async fn start(&self, _: EngineStart) -> Result<u64, EngineError> {
            Ok(1)
        }
        async fn stop(&self) -> Result<(), EngineError> {
            Ok(())
        }
        fn is_running(&self) -> bool {
            true
        }
        fn traffic(&self) -> Result<paws_engine::EngineTraffic, EngineError> {
            if self.traffic_fails {
                return Err(EngineError::NotRunning);
            }
            let step = self
                .traffic_sequence
                .as_ref()
                .map(|sequence| sequence.fetch_add(1, Ordering::SeqCst) + 1);
            Ok(paws_engine::EngineTraffic {
                upload_speed: 3,
                download_speed: 5,
                upload_total: step.unwrap_or(7),
                download_total: step.map_or(11, |step| step * 2),
            })
        }
        fn version(&self) -> Result<String, EngineError> {
            Ok("test".into())
        }
    }

    struct StalledManagement;
    #[async_trait::async_trait]
    impl paws_engine::EngineManagement for StalledManagement {
        fn capabilities(&self) -> EngineCapabilities {
            paws_engine_go::GoEngineSession::descriptor(VpnEngine::Mihomo).unwrap()
        }
        async fn runtime_revisions(
            &self,
        ) -> Result<paws_engine::EngineRuntimeRevisions, EngineError> {
            std::future::pending().await
        }
        async fn connections(&self) -> Result<Vec<ConnectionSummary>, EngineError> {
            Err(EngineError::NotRunning)
        }
        async fn close_connections(&self, _: Option<&str>) -> Result<(), EngineError> {
            Err(EngineError::NotRunning)
        }
        async fn flush_dns(&self, _: bool) -> Result<(), EngineError> {
            Err(EngineError::NotRunning)
        }
        async fn proxy_delay(&self, _: &str, _: &str, _: Duration) -> Result<u16, EngineError> {
            Err(EngineError::NotRunning)
        }
        async fn refresh_provider(&self, _: &str, _: &str) -> Result<(), EngineError> {
            Err(EngineError::NotRunning)
        }
        async fn provider_health(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: Duration,
            _: Option<&str>,
        ) -> Result<Option<u16>, EngineError> {
            Err(EngineError::NotRunning)
        }
    }

    struct SwitchingBinding {
        sample: SampleSession,
        core: std::sync::Weak<CoreHandle>,
    }
    #[async_trait::async_trait]
    impl EngineSession for SwitchingBinding {
        fn engine(&self) -> VpnEngine {
            self.sample.engine()
        }
        fn capabilities(&self) -> EngineCapabilities {
            self.sample.capabilities()
        }
        fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
            let binding = self.sample.management_binding();
            self.core
                .upgrade()
                .unwrap()
                .lock_state()
                .unwrap()
                .platform_controller_endpoint
                .as_mut()
                .unwrap()
                .secret = Some("new-epoch".into());
            binding
        }
        fn is_running(&self) -> bool {
            true
        }
        async fn start(&self, _: EngineStart) -> Result<u64, EngineError> {
            Ok(1)
        }
        async fn stop(&self) -> Result<(), EngineError> {
            Ok(())
        }
        fn traffic(&self) -> Result<paws_engine::EngineTraffic, EngineError> {
            self.sample.traffic()
        }
        fn version(&self) -> Result<String, EngineError> {
            self.sample.version()
        }
    }
    struct BindingPublication {
        endpoint: Arc<Mutex<paws_engine::EngineControllerEndpoint>>,
    }
    #[async_trait::async_trait]
    impl EngineSession for BindingPublication {
        fn engine(&self) -> VpnEngine {
            VpnEngine::Mihomo
        }
        fn capabilities(&self) -> EngineCapabilities {
            paws_engine_go::GoEngineSession::descriptor(self.engine()).unwrap()
        }
        fn management_binding(&self) -> Option<paws_engine::EngineManagementBinding> {
            let endpoint = self.endpoint.lock().unwrap().clone();
            Some(paws_engine::EngineManagementBinding {
                client: Arc::new(
                    paws_engine_go::ControllerClient::new(self.engine(), endpoint.clone()).unwrap(),
                ),
                endpoint,
            })
        }
        fn is_running(&self) -> bool {
            true
        }
        async fn start(&self, _: EngineStart) -> Result<u64, EngineError> {
            Ok(1)
        }
        async fn stop(&self) -> Result<(), EngineError> {
            Ok(())
        }
        fn traffic(&self) -> Result<paws_engine::EngineTraffic, EngineError> {
            Ok(paws_engine::EngineTraffic::default())
        }
        fn version(&self) -> Result<String, EngineError> {
            Ok("test".into())
        }
    }
    #[tokio::test]
    async fn a_committed_backend_epoch_fences_receipts_before_core_endpoint_publication() {
        let endpoint = paws_engine::EngineControllerEndpoint {
            address: "127.0.0.1:9".parse().unwrap(),
            secret: Some("old".into()),
        };
        let (core, root) = managed_core(endpoint.clone()).await;
        let binding = Arc::new(Mutex::new(endpoint));
        *core.active_engine.lock().unwrap() = Some(ActiveEngineSession {
            attempt: "attempt-a".into(),
            session: Arc::new(BindingPublication {
                endpoint: binding.clone(),
            }),
        });
        let target = core.runtime_management().unwrap().unwrap();
        binding.lock().unwrap().secret = Some("new-native-committed-epoch".into());
        assert!(core
            .ensure_management_owner(&target.attempt, target.revision, &target.endpoint)
            .is_err());
        assert!(core
            .record_managed_action(
                &target.attempt,
                target.revision,
                &target.endpoint,
                "must-not-publish".into(),
                None
            )
            .is_err());
        assert!(!core
            .lock_state()
            .unwrap()
            .logs
            .iter()
            .any(|entry| entry.message == "must-not-publish"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn management_capture_rejects_a_same_attempt_same_revision_epoch_replacement() {
        let endpoint = paws_engine::EngineControllerEndpoint {
            address: "127.0.0.1:9".parse().unwrap(),
            secret: Some("old-epoch".into()),
        };
        let (core, root) = managed_core(endpoint.clone()).await;
        let revision = core.lock_state().unwrap().config_revision;
        *core.active_engine.lock().unwrap() = Some(ActiveEngineSession {
            attempt: "attempt-a".into(),
            session: Arc::new(SwitchingBinding {
                sample: SampleSession {
                    endpoint,
                    traffic_fails: false,
                    stall_revision: false,
                    traffic_sequence: None,
                },
                core: Arc::downgrade(&core),
            }),
        });
        assert!(core.runtime_management().is_err());
        assert_eq!(core.lock_state().unwrap().config_revision, revision);
        assert_eq!(
            core.lock_state().unwrap().platform_start_attempt_id,
            "attempt-a"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_revision_observer_cannot_discard_a_real_traffic_sample() {
        let root = std::env::temp_dir().join(format!(
            "paws-revision-timeout-{}-{}",
            std::process::id(),
            now_unix_nanos()
        ));
        let core = Arc::new(CoreHandle::new_with_profile_root(&root));
        let endpoint = paws_engine::EngineControllerEndpoint {
            address: "127.0.0.1:9".parse().unwrap(),
            secret: None,
        };
        {
            let mut state = core.lock_state().unwrap();
            state.platform_vpn_running = true;
            state.platform_vpn_engine = Some(VpnEngine::Mihomo);
            state.platform_start_attempt_id = "attempt-a".into();
            state.platform_controller_endpoint = Some(endpoint.clone());
        }
        *core.active_engine.lock().unwrap() = Some(ActiveEngineSession {
            attempt: "attempt-a".into(),
            session: Arc::new(SampleSession {
                endpoint,
                traffic_fails: false,
                stall_revision: true,
                traffic_sequence: Some(Arc::default()),
            }),
        });
        let observer_core = core.clone();
        let observer = tokio::spawn(async move { observer_core.poll_engine_observations().await });
        tokio::task::yield_now().await;
        assert!(
            !observer.is_finished(),
            "the real revision future is still pending"
        );
        for expected in [1, 2] {
            tokio::time::timeout(Duration::from_millis(1), core.poll_engine_traffic())
                .await
                .expect("traffic waited for the stalled observer")
                .unwrap();
            let samples = core.telemetry_publications.lock().unwrap();
            assert_eq!(samples.last().unwrap().traffic.upload_bytes, expected);
            let wire = serde_json::to_value(samples.last().unwrap()).unwrap();
            assert_eq!(wire["traffic"]["uploadBytes"], expected);
            assert!(wire.get("connections").is_none());
            assert!(wire.get("dns").is_none());
            assert!(wire.get("logs").is_none());
        }
        assert!(
            !observer.is_finished(),
            "both traffic publications must precede the observer timeout"
        );
        assert_eq!(core.telemetry_publications.lock().unwrap().len(), 2);
        core.engine_observation_lane.cancel_and_join().await;
        observer.await.unwrap().unwrap();
        assert!(core
            .lock_state()
            .unwrap()
            .engine_observation
            .revisions
            .is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn periodic_traffic_does_not_query_heavy_connections_or_fabricate_failed_counters() {
        for traffic_fails in [false, true] {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let endpoint = paws_engine::EngineControllerEndpoint {
                address: listener.local_addr().unwrap(),
                secret: None,
            };
            let router = axum::Router::new().route(
                "/connections",
                axum::routing::get(move || async move {
                    if traffic_fails {
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({"connections":[]})),
                        )
                    } else {
                        (
                            axum::http::StatusCode::BAD_GATEWAY,
                            axum::Json(serde_json::json!({})),
                        )
                    }
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let (core, root) = managed_core(endpoint.clone()).await;
            let before = {
                let mut state = core.lock_state().unwrap();
                state.traffic.upload_bytes = 123;
                state.dns.model = "actual-dns-config".into();
                state.connections = vec![serde_json::from_value(serde_json::json!({
                "id":"old", "host":"example.test", "domain":"example.test", "destinationIp":"192.0.2.1", "destinationPort":443,
                "network":"TCP", "rule":"MATCH", "proxy":"node", "uploadBytes":1, "downloadBytes":2
            })).unwrap()];
                state.telemetry_revision
            };
            *core.active_engine.lock().unwrap() = Some(ActiveEngineSession {
                attempt: "attempt-a".into(),
                session: Arc::new(SampleSession {
                    endpoint,
                    traffic_fails,
                    stall_revision: false,
                    traffic_sequence: None,
                }),
            });
            assert_eq!(
                core.poll_engine_traffic().await.is_err(),
                traffic_fails,
                "only the traffic source can fail its counter lane"
            );
            {
                let state = core.lock_state().unwrap();
                assert!(state.telemetry_revision > before);
                assert_eq!(
                    state.dns.model, "actual-dns-config",
                    "traffic cannot reset another domain to a synthesized DNS config"
                );
                if traffic_fails {
                    assert_eq!(
                        state.traffic.upload_bytes, 123,
                        "failed traffic cannot manufacture zero"
                    );
                    assert_eq!(
                        state.connections[0].id, "old",
                        "periodic traffic never replaces an unqueried connection domain"
                    );
                } else {
                    assert_eq!(
                        state.traffic.upload_bytes, 7,
                        "valid traffic survives a connection-list failure"
                    );
                    assert_eq!(
                        state.connections[0].id, "old",
                        "failed connection query cannot pretend all connections closed"
                    );
                }
            }
            let queried = core
                .refresh_and_query_runtime_domain(RuntimeDomainKey {
                    domain: RuntimeDomain::Connections,
                    group_key: None,
                })
                .await;
            if traffic_fails {
                queried.unwrap();
                assert!(
                    core.lock_state().unwrap().connections.is_empty(),
                    "a successful explicit empty list is authoritative"
                );
            } else {
                assert!(
                    queried.is_err(),
                    "failed explicit connection query remains visible independently of traffic"
                );
            }
            server.abort();
            let _ = fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn managed_probes_publish_authoritative_resources_and_survive_telemetry_sampling() {
        let (endpoint, server) = management_server(None).await;
        let (core, root) = managed_core(endpoint).await;
        let before = core.snapshot().unwrap();
        assert!(
            core.active_engine_session().unwrap().is_none(),
            "UI process has no local engine"
        );
        assert_eq!(core.test_proxy_delay("node", None, None).await.unwrap(), 37);
        let after = core.snapshot().unwrap();
        assert!(after.resource_revision > before.resource_revision);
        assert_eq!(after.proxy_groups[0].proxies[0].delay_ms, Some(37));
        assert_eq!(after.config_revision, before.config_revision);
        let delays = core
            .test_proxy_group_via_controller("group", None, None)
            .await
            .unwrap();
        assert_eq!(delays["node"], 37);
        assert_eq!(delays["failed"], 0);
        assert_eq!(
            core.healthcheck_provider_proxy_via_controller(
                "provider",
                "node",
                "http://example.test",
                None,
                None
            )
            .await
            .unwrap(),
            51
        );
        assert_eq!(
            core.snapshot().unwrap().providers[0].members[0].delay_ms,
            Some(51)
        );
        core.healthcheck_proxy_provider_via_controller("provider")
            .await
            .unwrap();
        let measured = core.snapshot().unwrap();
        assert_eq!(measured.providers[0].members[0].delay_ms, Some(63));
        assert_eq!(measured.providers[0].members[0].alive, Some(true));
        assert_eq!(measured.proxy_groups[0].proxies[1].delay_ms, None);
        core.refresh_telemetry().unwrap();
        let sampled = core.snapshot().unwrap();
        assert_eq!(sampled.proxy_groups, measured.proxy_groups);
        assert_eq!(sampled.providers, measured.providers);
        assert_eq!(sampled.resource_revision, measured.resource_revision);
        assert_eq!(sampled.config_revision, before.config_revision);
        server.abort();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rule_followup_closes_only_the_connection_owners_actual_session() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received = calls.clone();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let endpoint = paws_engine::EngineControllerEndpoint {
            address: listener.local_addr().unwrap(),
            secret: Some("followup-owner".into()),
        };
        let router = axum::Router::new().route(
            "/connections/{id}",
            axum::routing::delete(move |headers: axum::http::HeaderMap| {
                let received = received.clone();
                async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer followup-owner"
                    );
                    received.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (core, root) = managed_core(endpoint).await;
        assert!(core
            .close_connection_for_session("same-id", "obsolete-owner")
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        core.close_connection_for_session("same-id", "attempt-a")
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        core.lock_state().unwrap().platform_start_attempt_id = "replacement-owner".into();
        assert!(core
            .close_connection_for_session("same-id", "attempt-a")
            .await
            .is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "old rule followup cannot close the replacement's same-id connection"
        );
        server.abort();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn service_recovery_selects_the_actual_go_controller_and_fences_late_receipts() {
        use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
        #[derive(Clone, Default)]
        struct Controller {
            selections: Arc<Mutex<Vec<String>>>,
            reads: Arc<AtomicU64>,
            closes: Arc<AtomicU64>,
            gate: Option<(bool, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
        }
        impl Controller {
            async fn wait(&self, selecting: bool) {
                if let Some((phase, entered, release)) = &self.gate {
                    if *phase == selecting {
                        entered.notify_one();
                        release.notified().await;
                    }
                }
            }
        }
        for engine in [VpnEngine::Mihomo, VpnEngine::SingBox] {
            // None succeeds. The other rounds replace the session after PUT,
            // or commit a newer configuration while selection readback waits.
            for phase in [None, Some(true), Some(false)] {
                let controller = Controller {
                    gate: phase.map(|phase| (phase, Arc::default(), Arc::default())),
                    ..Default::default()
                };
                let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                    .await
                    .unwrap();
                let endpoint = paws_engine::EngineControllerEndpoint {
                    address: listener.local_addr().unwrap(),
                    secret: None,
                };
                let router = Router::new()
                    .route(
                        "/proxies/{name}",
                        get(|State(state): State<Controller>| async move {
                            state.reads.fetch_add(1, Ordering::SeqCst);
                            state.wait(false).await;
                            Json(serde_json::json!({"now":"failed","fixed":"failed"}))
                        })
                        .put(
                            |State(state): State<Controller>,
                             Json(value): Json<serde_json::Value>| async move {
                                state
                                    .selections
                                    .lock()
                                    .unwrap()
                                    .push(value["name"].as_str().unwrap().to_owned());
                                state.wait(true).await;
                                StatusCode::NO_CONTENT
                            },
                        ),
                    )
                    .route(
                        "/connections",
                        axum::routing::delete(|State(state): State<Controller>| async move {
                            state.closes.fetch_add(1, Ordering::SeqCst);
                            StatusCode::NO_CONTENT
                        }),
                    )
                    .with_state(controller.clone());
                let server =
                    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
                let (core, root) = managed_core(endpoint).await;
                core.lock_state().unwrap().platform_vpn_engine = Some(engine);
                let revision = core.snapshot().unwrap().config_revision;
                let mut obsolete = core.runtime_management().unwrap().unwrap();
                obsolete.attempt = "old-attempt".into();
                assert!(core
                    .select_managed_proxy("group", "failed", obsolete)
                    .await
                    .is_err());
                let mut obsolete = core.runtime_management().unwrap().unwrap();
                obsolete.revision = revision + 1;
                assert!(core
                    .select_managed_proxy("group", "failed", obsolete)
                    .await
                    .is_err());
                assert!(
                    controller.selections.lock().unwrap().is_empty(),
                    "stale recovery must not send a write"
                );
                let target = core.runtime_management().unwrap().unwrap();
                let client = core.clone();
                let action = tokio::spawn(async move {
                    client.select_managed_proxy("group", "failed", target).await
                });
                if let Some((selecting, entered, release)) = &controller.gate {
                    entered.notified().await;
                    let mut state = core.lock_state().unwrap();
                    if *selecting {
                        state.platform_start_attempt_id = "attempt-b".into();
                    } else {
                        state.config_revision += 1;
                    }
                    drop(state);
                    release.notify_one();
                }
                let result = action.await.unwrap();
                assert_eq!(*controller.selections.lock().unwrap(), vec!["failed"]);
                if phase.is_none() {
                    result.unwrap();
                    assert_eq!(controller.closes.load(Ordering::SeqCst), 1);
                    assert_eq!(
                        core.snapshot().unwrap().proxy_groups[0].selected.as_deref(),
                        Some("failed")
                    );
                } else {
                    assert!(result.is_err());
                    assert_eq!(controller.closes.load(Ordering::SeqCst), 0);
                    assert_eq!(
                        core.snapshot().unwrap().proxy_groups[0].selected.as_deref(),
                        Some("node")
                    );
                    if phase == Some(true) {
                        assert_eq!(
                            controller.reads.load(Ordering::SeqCst),
                            0,
                            "replaced owner cannot issue a follow-up read"
                        );
                    }
                }
                server.abort();
                let _ = fs::remove_dir_all(root);
            }
        }
    }

    #[tokio::test]
    async fn delayed_management_result_cannot_mutate_a_new_configuration_or_session() {
        for replace_owner in [false, true] {
            let gate = Arc::new(tokio::sync::Notify::new());
            let entered = Arc::new(tokio::sync::Notify::new());
            let (endpoint, server) = management_server(Some((entered.clone(), gate.clone()))).await;
            let (core, root) = managed_core(endpoint).await;
            let client = core.clone();
            let probe =
                tokio::spawn(async move { client.test_proxy_delay("node", None, None).await });
            // Await the HTTP request entering the handler without relying on a sleep.
            entered.notified().await;
            {
                let mut state = core.lock_state().unwrap();
                if replace_owner {
                    state.platform_start_attempt_id = "attempt-b".into();
                } else {
                    state.config_revision += 1;
                }
            }
            gate.notify_one();
            assert!(probe.await.unwrap().is_err());
            assert_eq!(
                core.snapshot().unwrap().proxy_groups[0].proxies[0].delay_ms,
                Some(999)
            );
            server.abort();
            let _ = fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn native_selection_and_provider_roster_survive_the_shadow_controller_watcher() {
        let (endpoint, server) = management_server(None).await;
        let (core, root) = managed_core(endpoint).await;
        core.select_proxy_via_controller("group", "failed")
            .await
            .unwrap();
        let selected = core.snapshot().unwrap();
        assert_eq!(selected.proxy_groups[0].selected.as_deref(), Some("failed"));
        assert!(core
            .select_proxy_via_controller("group", "bad-node")
            .await
            .is_err());
        assert_eq!(
            core.snapshot().unwrap().config_revision,
            selected.config_revision
        );
        assert!(!core.sync_external_controller_config().await.unwrap());
        core.refresh_telemetry().unwrap();
        assert_eq!(core.snapshot().unwrap().proxy_groups, selected.proxy_groups);
        core.unfix_proxy_via_controller("group").await.unwrap();
        assert_eq!(
            core.snapshot().unwrap().proxy_groups[0].fixed.as_deref(),
            Some("")
        );
        core.lock_state().unwrap().providers[0].members[0].name = "removed-node".into();
        core.refresh_provider_of_type("proxy", "provider")
            .await
            .unwrap();
        let refreshed = core.snapshot().unwrap();
        assert_eq!(refreshed.providers[0].members[0].name, "new-node");
        assert_eq!(refreshed.providers[0].members[0].delay_ms, Some(63));
        assert_eq!(
            refreshed.proxy_groups[0]
                .proxies
                .iter()
                .map(|proxy| proxy.name.as_str())
                .collect::<Vec<_>>(),
            vec!["new-node"]
        );
        assert!(refreshed.providers[0].last_refresh_error.is_none());
        core.refresh_telemetry().unwrap();
        assert_eq!(core.snapshot().unwrap().providers, refreshed.providers);
        core.lock_state().unwrap().platform_vpn_running = false;
        assert!(!core.sync_external_controller_config().await.unwrap());
        core.refresh_telemetry().unwrap();
        let stopped = core.snapshot().unwrap();
        assert_eq!(stopped.providers, refreshed.providers);
        assert_eq!(stopped.proxy_groups, refreshed.proxy_groups);
        assert_eq!(
            stopped.profiles[0]
                .selected_proxies
                .get("group")
                .map(String::as_str),
            Some("")
        );
        server.abort();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stopping_the_engine_preserves_the_native_selection_in_snapshot_and_profile() {
        let (endpoint, server) = management_server(None).await;
        let (core, root) = managed_core(endpoint).await;
        {
            let mut state = core.lock_state().unwrap();
            let profile = state.profiles.active_profile().unwrap().to_owned();
            state.profiles.update_profile_content(&profile, "proxies:\n  - name: node\n    type: http\n    server: 127.0.0.1\n    port: 1\n  - name: failed\n    type: http\n    server: 127.0.0.1\n    port: 2\nproxy-groups:\n  - name: group\n    type: select\n    proxies: [node, failed]\nrules:\n  - MATCH,group\n").unwrap();
        }
        let profile = core
            .lock_state()
            .unwrap()
            .profiles
            .active_profile()
            .unwrap()
            .to_owned();
        crate::tests::without_test_geodata(&core, &profile);
        core.select_proxy_via_controller("group", "failed")
            .await
            .unwrap();
        let selected = core.snapshot().unwrap();
        core.stop_engine("attempt-a").await.unwrap();
        assert!(!core.sync_external_controller_config().await.unwrap());
        core.refresh_telemetry().unwrap();
        let stopped = core.snapshot().unwrap();
        assert_eq!(stopped.proxy_groups, selected.proxy_groups);
        assert_eq!(
            stopped.profiles[0]
                .selected_proxies
                .get("group")
                .map(String::as_str),
            Some("failed")
        );
        assert!(
            !core.prepare_active_vpn().await.unwrap(),
            "ordinary bootstrap keeps the authoritative native projection"
        );
        {
            let mut state = core.lock_state().unwrap();
            assert!(state.engine_resources_authoritative);
            state.platform_start_attempt_id = "attempt-b".into();
            state.platform_start_outcome = PlatformStartOutcome::Pending;
            state.platform_stop_requested = false;
        }
        let options = VpnOptions {
            engine: VpnEngine::Meow,
            ..Default::default()
        };
        core.prepare_engine(&to_json(&options).unwrap(), "attempt-b")
            .await
            .unwrap();
        let state = core.lock_state().unwrap();
        assert!(!state.engine_resources_authoritative);
        assert_eq!(
            state
                .tunnel
                .as_ref()
                .unwrap()
                .proxy("group")
                .unwrap()
                .current()
                .as_deref(),
            Some("failed"),
            "the next Meow session must receive the persisted native selection"
        );
        drop(state);
        server.abort();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn serialized_extension_endpoint_enables_ui_management_without_a_local_session() {
        let (endpoint, server) = management_server(None).await;
        let (extension, root) = managed_core(endpoint.clone()).await;
        for engine in [VpnEngine::Meow, VpnEngine::Mihomo, VpnEngine::SingBox] {
            let ui_root = root.join(format!("ui-{engine:?}"));
            let ui = CoreHandle::new_with_profile_root(&ui_root);
            {
                let mut extension = extension.lock_state().unwrap();
                extension.platform_vpn_engine = Some(engine);
                extension.platform_vpn_state_updated_at += 1;
                let frame = serde_json::to_vec(&platform_vpn_state(&extension)).unwrap();
                let remote: PlatformVpnState = serde_json::from_slice(&frame).unwrap();
                assert_eq!(remote.controller_endpoint, Some(endpoint.clone()));
                let mut state = ui.lock_state().unwrap();
                state.platform_start_attempt_id = "attempt-a".into();
                state.proxy_groups = extension.proxy_groups.clone();
                ui.apply_platform_envelope_locked(&mut state, true, None, Some(remote));
            }
            assert!(ui.active_engine_session().unwrap().is_none());
            assert_eq!(ui.test_proxy_delay("node", None, None).await.unwrap(), 37);
            ui.refresh_telemetry().unwrap();
            assert_eq!(
                ui.snapshot().unwrap().proxy_groups[0].proxies[0].delay_ms,
                Some(37)
            );
        }
        server.abort();
        let _ = fs::remove_dir_all(root);
    }
}
