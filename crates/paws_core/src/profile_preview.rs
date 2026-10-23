//! Offline profile declarations are not a running Meow engine. Backend
//! capability checks and runtime observations have separate owners.
use super::*;

pub(super) struct ProfilePreview {
    pub profile_id: String,
    pub runtime_yaml: String,
    pub metadata: paws_profile::ProfileSourceMetadata,
    pub vpn_options: VpnOptions,
    pub controller_access: ControllerAccessConfig,
    pub network_ports: NetworkPortConfig,
    pub revision: u64,
    pub owner: PreviewOwner,
    pub traffic: Option<TunStats>,
}

pub(super) struct PreviewOwner {
    attempt: String,
    endpoint: Option<paws_engine::EngineControllerEndpoint>,
    running: bool,
    starting: bool,
}
impl PreviewOwner {
    pub(super) fn capture(state: &CoreState) -> Self {
        Self {
            attempt: state.platform_start_attempt_id.clone(),
            endpoint: state.platform_controller_endpoint.clone(),
            running: state.platform_vpn_running,
            starting: state.platform_vpn_starting,
        }
    }
    fn check(&self, state: &CoreState) -> Result<(), PawsError> {
        if self.attempt != state.platform_start_attempt_id
            || self.endpoint != state.platform_controller_endpoint
            || self.running != state.platform_vpn_running
            || self.starting != state.platform_vpn_starting
        {
            return Err(PawsError::Core(
                "profile preview owner changed before commit".into(),
            ));
        }
        Ok(())
    }
}

impl CoreHandle {
    pub(super) async fn commit_profile_preview(
        &self,
        preview: ProfilePreview,
    ) -> Result<(), PawsError> {
        let previous = {
            let mut state = self.lock_state()?;
            Self::ensure_config_revision_locked(&state, preview.revision)?;
            preview.owner.check(&state)?;
            if self.vpn.is_running() {
                return Err(PawsError::Core(
                    "cannot replace a live Meow owner with profile declarations".into(),
                ));
            }
            state.api_controller.take()
        };
        if let Some(mut previous) = previous {
            previous.shutdown().await;
        }
        let mut state = self.lock_state()?;
        Self::ensure_config_revision_locked(&state, preview.revision)?;
        preview.owner.check(&state)?;
        if self.vpn.is_running() {
            return Err(PawsError::Core(
                "Meow started before profile preview commit".into(),
            ));
        }
        state
            .profiles
            .write_runtime_yaml(&preview.profile_id, &preview.runtime_yaml)?;
        if state.profiles.active_profile() != Some(preview.profile_id.as_str()) {
            settle_traffic_before_profile_switch(&mut state, preview.traffic.as_ref())?;
        }
        state.profiles.set_active(&preview.profile_id)?;
        let editable = state.profiles.rules_for_profile(&preview.profile_id);
        state.runtime_rules =
            runtime_rule_summaries(&preview.profile_id, &preview.metadata.rule_lines, &editable);
        state.proxy_groups = preview.metadata.proxy_groups;
        state.providers = preview.metadata.providers;
        state.engine_observation = Default::default();
        state.engine_observation.providers = runtime_observation::definition_provider_availability(
            &state.providers,
            &preview.metadata.unverified_providers,
        );
        let refresh = state.provider_refresh.clone();
        apply_provider_refresh_states(&mut state.providers, &refresh);
        state.tunnel = None;
        state.controller_start = None;
        state.meow_runtime_ready = false;
        state.controller_diagnostics = Default::default();
        state.vpn_options = preview.vpn_options;
        state.dns = dns_snapshot(&state.vpn_options, None);
        state.controller_access = preview.controller_access;
        state.network_ports = preview.network_ports;
        state.engine_loaded = true;
        state.engine_resources_authoritative = false;
        state.geodata = state.profiles.geodata_files();
        state.last_meow_traffic_sample = None;
        invalidate_exit_location(&mut state);
        persist_runtime_ui_cache_best_effort(&mut state);
        state.logs.push(info_log(
            "profile definitions loaded; runtime health has not been measured",
        ));
        Ok(())
    }

    /// Returns true only when this operation is an offline persisted preselect.
    /// A live/starting owner must use the session-fenced management path.
    pub(super) fn select_offline_definition(
        &self,
        group_name: &str,
        selected: Option<&str>,
    ) -> Result<bool, PawsError> {
        let _transaction = self.try_config_transaction()?;
        let mut state = self.lock_state()?;
        if state.platform_vpn_running
            || state.platform_vpn_starting
            || self.vpn.is_running()
            || state.meow_runtime_ready
        {
            return Ok(false);
        }
        if !state.engine_loaded {
            return Ok(false);
        }
        let group = state
            .proxy_groups
            .iter()
            .find(|group| group.name == group_name)
            .ok_or_else(|| {
                PawsError::Core(
                    "preselection group is not present in the profile definitions".into(),
                )
            })?;
        if let Some(name) = selected {
            if !group.proxies.iter().any(|proxy| proxy.name == name) {
                return Err(PawsError::Core(
                    "preselection member is not present in the profile definitions/cache".into(),
                ));
            }
        } else if group.fixed.is_none() {
            return Err(PawsError::Core(
                "only automatic groups can clear a fixed preselection".into(),
            ));
        }
        let profile = state
            .profiles
            .active_profile()
            .ok_or_else(|| PawsError::Core("activate a profile before selecting a node".into()))?
            .to_owned();
        state.profiles.set_selected_proxy(
            &profile,
            group_name.to_owned(),
            selected.unwrap_or_default().to_owned(),
        )?;
        let group = state
            .proxy_groups
            .iter_mut()
            .find(|group| group.name == group_name)
            .unwrap();
        group.selected = selected.map(ToOwned::to_owned);
        if group.fixed.is_some() {
            group.fixed = Some(selected.unwrap_or_default().to_owned());
        }
        for proxy in &mut group.proxies {
            proxy.selected = selected == Some(proxy.name.as_str());
        }
        persist_runtime_ui_cache_best_effort(&mut state);
        invalidate_exit_location(&mut state);
        self.publish_config_and_resource_change_locked(&mut state);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "paws-definition-preview-{}-{}-{}",
            std::process::id(),
            now_unix_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
    fn plan(core: &CoreHandle, id: &str, traffic: Option<TunStats>) -> ProfilePreview {
        let state = core.lock_state().unwrap();
        let options = state.profiles.vpn_options_for_profile(id).unwrap();
        let yaml = state
            .profiles
            .render_runtime_yaml(id, state.mode, &options)
            .unwrap();
        ProfilePreview {
            profile_id: id.into(),
            metadata: state
                .profiles
                .source_metadata(&yaml, &state.profiles.selected_proxies(id).unwrap())
                .unwrap(),
            runtime_yaml: yaml,
            vpn_options: options,
            controller_access: state.controller_access.clone(),
            network_ports: state.network_ports,
            revision: state.config_revision,
            owner: PreviewOwner::capture(&state),
            traffic,
        }
    }
    #[tokio::test]
    async fn actual_import_activation_and_preselection_keep_go_only_nodes_without_a_meow_tunnel() {
        let root = root();
        let core = CoreHandle::new_with_profile_root(&root);
        let source="proxies:\n  - {name: grpc, type: trojan, server: example.test, port: 443, password: test, network: grpc, grpc-opts: {grpc-service-name: svc}}\n  - {name: other, type: socks5, server: example.test, port: 1080}\nproxy-providers:\n  remote: {type: http, url: 'http://127.0.0.1:1/no-network'}\nproxy-groups:\n  - {name: Main, type: select, proxies: [grpc,other], use: [remote]}\nrules: ['MATCH,Main']\n";
        let id = core
            .import_profile_from_content("Go fixture", "test", source, None)
            .await
            .unwrap();
        // This is the production UI/remote-engine context after a stopped Go
        // session; no fake native controller or runtime has been installed.
        core.lock_state().unwrap().platform_vpn_engine = Some(VpnEngine::SingBox);
        core.activate_profile(&id).await.unwrap();
        {
            let state = core.lock_state().unwrap();
            assert!(state.engine_loaded);
            assert!(!state.meow_runtime_ready);
            assert!(state.tunnel.is_none());
            assert_eq!(state.proxy_groups[0].proxies[0].name, "grpc");
            assert!(state
                .proxy_groups
                .iter()
                .flat_map(|group| &group.proxies)
                .all(|proxy| proxy.delay_ms.is_none()));
        }
        core.select_proxy_via_controller("Main", "other")
            .await
            .unwrap();
        core.reload_config_for_purpose(&id, MeowLoadPurpose::Preview)
            .await
            .unwrap();
        assert_eq!(
            core.lock_state().unwrap().proxy_groups[0]
                .selected
                .as_deref(),
            Some("other")
        );
        assert_eq!(
            core.lock_state()
                .unwrap()
                .profiles
                .selected_proxies(&id)
                .unwrap()
                .get("Main")
                .map(String::as_str),
            Some("other")
        );
        for engine in [VpnEngine::SingBox, VpnEngine::Mihomo] {
            let options = VpnOptions {
                engine,
                ..Default::default()
            };
            assert_eq!(
                core.resolve_vpn_engine(&to_json(&options).unwrap())
                    .unwrap(),
                engine.as_str()
            );
        }
        let error = core
            .reload_config_for_purpose(&id, MeowLoadPurpose::Runtime)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Trojan network"));
        assert!(core.lock_state().unwrap().tunnel.is_none());
        drop(core);
        let _ = fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn preview_rejects_a_new_attempt_even_when_config_revision_is_unchanged() {
        let root = root();
        let core = CoreHandle::new_with_profile_root(&root);
        let id = core
            .import_profile_from_content(
                "next",
                "test",
                &paws_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let active = core
            .lock_state()
            .unwrap()
            .profiles
            .active_profile()
            .map(ToOwned::to_owned);
        let pending = plan(&core, &id, None);
        core.lock_state().unwrap().platform_start_attempt_id = "new-attempt".into();
        assert!(core.commit_profile_preview(pending).await.is_err());
        assert_eq!(
            core.lock_state().unwrap().profiles.active_profile(),
            active.as_deref()
        );
        drop(core);
        let _ = fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn preview_settles_previous_profile_traffic_once_before_activation() {
        let root = root();
        let core = CoreHandle::new_with_profile_root(&root);
        let first = core
            .import_profile_from_content(
                "first",
                "test",
                &paws_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        let second = core
            .import_profile_from_content(
                "second",
                "test",
                &paws_profile::default_runtime_yaml(),
                None,
            )
            .await
            .unwrap();
        {
            let mut state = core.lock_state().unwrap();
            state.profiles.set_active(&first).unwrap();
            apply_traffic_sample(
                &mut state,
                &TunStats {
                    tx_bytes: 100,
                    rx_bytes: 200,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        core.commit_profile_preview(plan(
            &core,
            &second,
            Some(TunStats {
                tx_bytes: 150,
                rx_bytes: 260,
                ..Default::default()
            }),
        ))
        .await
        .unwrap();
        core.commit_profile_preview(plan(
            &core,
            &second,
            Some(TunStats {
                tx_bytes: 150,
                rx_bytes: 260,
                ..Default::default()
            }),
        ))
        .await
        .unwrap();
        {
            let state = core.lock_state().unwrap();
            let first = state.profiles.profile(&first).unwrap();
            let second = state.profiles.profile(&second).unwrap();
            assert_eq!((first.upload_bytes, first.download_bytes), (260, 150));
            assert_eq!((second.upload_bytes, second.download_bytes), (0, 0));
        }
        drop(core);
        let _ = fs::remove_dir_all(root);
    }
}
