//! UI-independent application command boundary. Native hosts own permissions,
//! file pickers and VPN abilities; this layer owns validation and core operations.
mod converter;
mod imports;
mod observation;
mod preferences;
mod request;
mod service_dns;
mod subscription_scan;

pub use preferences::{PreferencesSnapshot, PreferencesStore};
pub use request::{AppRequest, AppResponse, PlatformEffect};

use paws_core::{CommandContext, CoreHandle};
use paws_model::PawsError;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Core(#[from] PawsError),
    #[error("invalid application request: {0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("preferences IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application state lock poisoned")]
    Poisoned,
    #[error("{0}")]
    Conversion(String),
    #[error("Profile import was cancelled")]
    ImportCancelled,
    #[error("Profile import timed out after 120 seconds")]
    ImportTimedOut,
    #[error("Observation owner was closed")]
    ObservationClosed,
    #[error("Observation subscription was superseded or the domain is not subscribed")]
    ObservationSuperseded,
    #[error("A query for this observation domain is already in flight")]
    ObservationQueryBusy,
}

impl From<converter::ConverterError> for AppError {
    fn from(error: converter::ConverterError) -> Self {
        Self::Conversion(error.to_string())
    }
}

/// No UI toolkit, N-API, or concrete engine is part of this public contract.
pub struct Application {
    core: Arc<CoreHandle>,
    imports: imports::ImportSession,
    observation: observation::StateObservation,
}

impl Application {
    fn converter(&self) -> Result<converter::Converter, AppError> {
        let home = std::env::var_os("PAWS_HOME")
            .ok_or_else(|| AppError::InvalidRequest("application home is not configured".into()))?;
        Ok(converter::Converter::new(
            self.core.clone(),
            std::path::PathBuf::from(home),
        ))
    }
    pub fn shared() -> Self {
        let core = paws_core::shared_core();
        let observation = observation::StateObservation::new(core.subscribe_runtime_revisions());
        Self {
            core,
            imports: Default::default(),
            observation,
        }
    }

    pub fn close(&self) {
        self.imports.close();
        self.observation.close();
    }

    pub fn subscribe_state_json(&self, request: &str) -> Result<String, AppError> {
        Ok(serde_json::to_string(
            &self.observation.subscribe(serde_json::from_str(request)?)?,
        )?)
    }

    pub fn observe_state_json(&self, critical_only: bool) -> Result<String, AppError> {
        Ok(serde_json::to_string(
            &self.observation.observe(critical_only)?,
        )?)
    }

    /// This explicit domain query is serialized on a blocking worker by hosts.
    /// Revalidation fences a query which finished after its page was removed.
    pub async fn query_state_json(&self, request: &str) -> Result<String, AppError> {
        let query: observation::Query = serde_json::from_str(request)?;
        let key = query.key();
        let _lease = self
            .observation
            .begin_query(query.generation, key.clone())?;
        #[derive(Serialize)]
        struct Reply {
            generation: u64,
            #[serde(flatten)]
            projection: paws_core::RuntimeDomainProjection,
        }
        let projection = self
            .core
            .refresh_and_query_runtime_domain_with_guard(key.clone(), || {
                self.observation.validate(query.generation, &key).is_ok()
            })
            .await?;
        let json = serde_json::to_string(&Reply {
            generation: query.generation,
            projection,
        })?;
        self.observation.validate(query.generation, &key)?;
        Ok(json)
    }

    /// Subscribe before checking the current revision so a concurrent publish
    /// cannot be lost. The bounded wait lets disposed hosts release their task.
    pub async fn wait_for_revision(&self, after: u64) -> Result<u64, AppError> {
        let mut receiver = self.core.subscribe_runtime_revisions();
        let wait = async {
            loop {
                let revision = receiver.borrow_and_update().revision;
                if revision > after {
                    return revision;
                }
                if receiver.changed().await.is_err() {
                    return revision;
                }
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(20), wait).await {
            Ok(revision) => Ok(revision),
            Err(_) => Ok(self.core.subscribe_runtime_revisions().borrow().revision),
        }
    }

    pub async fn dispatch_json(&self, text: &str) -> Result<String, AppError> {
        let request: AppRequest = serde_json::from_str(text)?;
        Ok(serde_json::to_string(&self.dispatch(request).await)?)
    }

    pub async fn dispatch(&self, request: AppRequest) -> AppResponse {
        let id = request.id;
        match self.execute(request).await {
            Ok((data, effect)) => AppResponse {
                id,
                ok: true,
                data: Some(data),
                effect,
                error: None,
            },
            Err(error) => AppResponse {
                id,
                ok: false,
                data: None,
                effect: None,
                error: Some(error.to_string()),
            },
        }
    }

    async fn execute(
        &self,
        request: AppRequest,
    ) -> Result<(Value, Option<PlatformEffect>), AppError> {
        use request::*;
        request.validate()?;
        let before = self.core.command_context()?;
        let reconfigure = request::requires_reconfigure(
            &request.command,
            &request.payload,
            before.active_profile.as_deref(),
        );
        let p = request.payload;
        let mut effect = None;
        let data = match request.command.as_str() {
            "engines.catalog" => value(self.core.engine_catalog()?)?,
            "engines.decision" => {
                let p: EngineDecision = payload(p)?;
                self.core
                    .engine_decision_report(&serde_json::to_string(&p.options)?)
                    .await?
            }
            "bootstrap" => {
                if before.active_profile.is_some() {
                    self.core.prepare_active_vpn().await?;
                }
                self.core.refresh_due_profiles().await?;
                Value::Null
            }
            "vpn.start" | "vpn.restart" => {
                if before.active_profile.is_none() {
                    return Err(AppError::InvalidRequest("select a profile first".into()));
                }
                self.core.prepare_active_vpn().await?;
                let prepared = self.core.command_context()?;
                if prepared.active_profile.is_none() {
                    return Err(AppError::InvalidRequest("select a profile first".into()));
                }
                effect = Some(PlatformEffect {
                    kind: if request.command == "vpn.start" {
                        "start"
                    } else {
                        "restart"
                    }
                    .into(),
                    owner: before.vpn_session_id.clone().unwrap_or_default(),
                    config_revision: prepared.config_revision.to_string(),
                    options_json: serde_json::to_string(&prepared.vpn_options)?,
                });
                Value::Null
            }
            "vpn.stop" => {
                effect = Some(PlatformEffect::user_stop());
                Value::Null
            }
            "mode.set" => {
                let p: Mode = payload(p)?;
                self.core.set_mode(p.mode)?;
                Value::Null
            }
            "profiles.importUrl" => {
                let p: ImportUrl = payload(p)?;
                let mut intent = self.imports.begin(p.intent_id)?;
                let prepared = intent
                    .prepare(self.core.prepare_profile_import_from_url(&p.url, p.name))
                    .await?;
                let receipt = intent
                    .commit(
                        self.core
                            .commit_prepared_profile_import_and_activate_checked(
                                prepared,
                                before.config_revision,
                            ),
                    )
                    .await?;
                value(receipt.profile_id)?
            }
            "profiles.parseScan" => {
                let p: Scan = payload(p)?;
                let parsed = subscription_scan::parse_scanned_subscription(&p.payload)
                    .map_err(|error| AppError::InvalidRequest(error.to_string()))?;
                value(parsed)?
            }
            "profiles.subscription" => {
                let p: ProfileSubscription = payload(p)?;
                self.core.update_profile_subscription_checked(
                    &p.profile_id,
                    p.expected_revision.get()?,
                    &p.name,
                    &p.url,
                )?;
                Value::Null
            }
            "profiles.importContent" => {
                let p: ImportContent = payload(p)?;
                let mut intent = self.imports.begin(p.intent_id)?;
                let prepared = intent
                    .prepare(
                        self.core
                            .prepare_profile_import_from_content(&p.name, "nsh", &p.content, None),
                    )
                    .await?;
                let receipt = intent
                    .commit(
                        self.core
                            .commit_prepared_profile_import_and_activate_checked(
                                prepared,
                                before.config_revision,
                            ),
                    )
                    .await?;
                value(receipt.profile_id)?
            }
            "profiles.cancelImport" => {
                let p: CancelImport = payload(p)?;
                value(self.imports.cancel(p.intent_id)?)?
            }
            "profiles.activate" => {
                let p: Profile = payload(p)?;
                self.core
                    .activate_profile_checked(&p.profile_id, before.config_revision)
                    .await?;
                Value::Null
            }
            "profiles.delete" => {
                let p: Profile = payload(p)?;
                self.core.delete_profile(&p.profile_id).await?;
                Value::Null
            }
            "profiles.refresh" => {
                let p: Profile = payload(p)?;
                self.core.refresh_profile(&p.profile_id).await?;
                Value::Null
            }
            "profiles.read" => {
                let p: Profile = payload(p)?;
                value(self.core.profile_raw_yaml(&p.profile_id)?)?
            }
            "profiles.validate" => {
                let p: Content = payload(p)?;
                self.core.validate_profile_content(&p.content).await?;
                Value::Null
            }
            "profiles.restore" => {
                let p: Profile = payload(p)?;
                self.core.restore_profile_backup(&p.profile_id).await?;
                Value::Null
            }
            "profiles.refreshAll" => {
                self.core.refresh_all_profiles().await?;
                Value::Null
            }
            "profiles.save" => {
                let p: SaveProfile = payload(p)?;
                self.core
                    .update_profile_content_checked(
                        &p.profile_id,
                        p.expected_revision.get()?,
                        &p.content,
                    )
                    .await?;
                Value::Null
            }
            "proxies.select" => {
                let p: SelectProxy = payload(p)?;
                self.core
                    .select_proxy_via_controller(&p.group, &p.name)
                    .await?;
                Value::Null
            }
            "proxies.unfix" => {
                let p: Group = payload(p)?;
                self.core.unfix_proxy_via_controller(&p.group).await?;
                Value::Null
            }
            "proxies.delay" => {
                let p: Name = payload(p)?;
                value(
                    self.core
                        .test_proxy_delay_via_controller(&p.name, None, None)
                        .await?,
                )?
            }
            "proxies.testGroup" => {
                let p: Group = payload(p)?;
                value(
                    self.core
                        .test_proxy_group_via_controller(&p.group, None, None)
                        .await?,
                )?
            }
            "connections.close" => {
                let p: Identifier = payload(p)?;
                self.core.close_connection_via_controller(&p.id).await?;
                Value::Null
            }
            "connections.closeAll" => {
                self.core.close_all_connections_via_controller().await?;
                Value::Null
            }
            "requests.clear" => {
                self.core.clear_runtime_history(false).await?;
                Value::Null
            }
            "logs.clear" => {
                self.core.clear_runtime_history(true).await?;
                Value::Null
            }
            "logs.recordingStatus" => value(self.core.log_recording_status()?)?,
            "logs.setRecording" => {
                let p: Enabled = payload(p)?;
                value(
                    self.core
                        .set_engine_log_recording_enabled(p.enabled)
                        .await?,
                )?
            }
            "logs.readArchive" => {
                let p: Archive = payload(p)?;
                value(self.core.read_log_archive(&p.file_name)?)?
            }
            "logs.deleteArchive" => {
                let p: Archive = payload(p)?;
                value(self.core.delete_log_archive(&p.file_name)?)?
            }
            "rules.lookup" => {
                let p: Query = payload(p)?;
                let r = self.core.lookup_rule(&p.query).await?;
                json!({"query":r.query,"inputKind":match r.input_kind { paws_core::RuleLookupInputKind::Domain=>"domain", paws_core::RuleLookupInputKind::Ip=>"ip" },
                    "resolvedIp":r.resolved_ip,"resolutionAttempted":r.resolution_attempted,"matched":r.matched,
                    "ruleType":r.rule_type,"rulePayload":r.rule_payload,"target":r.target,"ruleLine":r.rule_line})
            }
            "rules.import" => {
                let p: RuleImport = payload(p)?;
                let r = self
                    .core
                    .import_rules_from_content_checked(
                        active(&before)?,
                        p.expected_revision.get()?,
                        "nsh",
                        &p.content,
                    )
                    .await?;
                value(r.imported_rule_ids)?
            }
            "rules.add" => {
                let p: AddRule = payload(p)?;
                if p.expected_revision.get()? != before.config_revision {
                    return Err(AppError::InvalidRequest(
                        "configuration changed; reload before editing".into(),
                    ));
                }
                let r = self
                    .core
                    .apply_manual_rule_checked(
                        active(&before)?,
                        p.expected_revision.get()?,
                        &p.spec,
                    )
                    .await?;
                let close_requested = p.disconnect_after_save;
                let close_error = if close_requested {
                    match (p.connection_id.as_deref(), before.vpn_session_id.as_deref()) {
                        (Some(id), Some(session)) => self
                            .core
                            .close_connection_for_session(id, session)
                            .await
                            .err()
                            .map(|error| error.to_string()),
                        (None, _) => Some("Rule saved, but no connection was specified".into()),
                        (_, None) => Some(
                            "Rule saved, but the original VPN session is no longer running".into(),
                        ),
                    }
                } else {
                    None
                };
                json!({"applied":{"mutation":r.mutation,"liveUpdated":r.live_updated,"ruleModeActive":r.rule_mode_active},
                    "connectionCloseRequested":close_requested,"connectionCloseError":close_error})
            }
            "rules.enabled" => {
                let p: RuleEnabled = payload(p)?;
                self.core
                    .set_rule_enabled_checked(
                        active(&before)?,
                        p.expected_revision.get()?,
                        &p.id,
                        p.enabled,
                    )
                    .await?;
                Value::Null
            }
            "rules.delete" => {
                let p: RuleDelete = payload(p)?;
                self.core
                    .delete_rule_checked(active(&before)?, p.expected_revision.get()?, &p.id)
                    .await?;
                Value::Null
            }
            "rules.reorder" => {
                let p: ReorderRules = payload(p)?;
                self.core
                    .reorder_rules_checked(active(&before)?, p.expected_revision.get()?, p.ids)
                    .await?;
                Value::Null
            }
            "providers.refresh" => {
                let p: Provider = payload(p)?;
                self.core
                    .refresh_provider_of_type(&p.provider_type, &p.name)
                    .await?;
                Value::Null
            }
            "providers.refreshAll" => {
                self.core.refresh_all_providers().await?;
                Value::Null
            }
            "providers.testProxy" => {
                let p: ProviderProxy = payload(p)?;
                value(
                    self.core
                        .healthcheck_provider_proxy_via_controller(
                            &p.provider,
                            &p.name,
                            "https://www.gstatic.com/generate_204",
                            Some(5000),
                            None,
                        )
                        .await?,
                )?
            }
            "providers.healthcheck" => {
                let p: Name = payload(p)?;
                self.core
                    .healthcheck_proxy_provider_via_controller(&p.name)
                    .await?;
                Value::Null
            }
            "dns.flush" => {
                self.core.flush_dns_cache_via_controller().await?;
                Value::Null
            }
            "dns.fakeIpFlush" => {
                self.core.flush_fake_ip_cache_via_controller().await?;
                Value::Null
            }
            "settings.vpn" => {
                let p: VpnSettings = payload(p)?;
                let o = p.options;
                let current = &before.vpn_options;
                if o.ipv6 != current.ipv6
                    || o.addresses != current.addresses
                    || o.routes != current.routes
                    || o.dns_addresses != current.dns_addresses
                {
                    return Err(AppError::InvalidRequest(
                        "edit IPv6, addresses and routes in the profile YAML".into(),
                    ));
                }
                self.core
                    .set_profile_vpn_config_checked(&p.profile_id, p.expected_revision.get()?, o)
                    .await?;
                Value::Null
            }
            "settings.dns" => {
                let p: DnsSettings = payload(p)?;
                self.core
                    .set_profile_dns_config_checked(
                        &p.profile_id,
                        p.expected_revision.get()?,
                        p.servers,
                        p.fallbacks,
                        p.policy,
                    )
                    .await?;
                Value::Null
            }
            "settings.network" => {
                let p: NetworkSettings = payload(p)?;
                if p.controller.secret != before.controller_access.secret {
                    return Err(AppError::InvalidRequest(
                        "edit the controller secret in the profile YAML".into(),
                    ));
                }
                self.core
                    .set_profile_network_config_checked(
                        &p.profile_id,
                        p.expected_revision.get()?,
                        p.ports,
                        p.controller.allow_lan,
                    )
                    .await?;
                Value::Null
            }
            "diagnostics.probe" => {
                let p: Probe = payload(p)?;
                let session = before
                    .vpn_session_id
                    .as_ref()
                    .ok_or(PawsError::VpnNotRunning)?;
                if !before.vpn_running {
                    return Err(PawsError::VpnNotRunning.into());
                }
                let delay = self
                    .core
                    .test_proxy_delay_via_controller(&p.group, Some(&p.url), Some(8000))
                    .await?;
                if !self.core.is_platform_vpn_session_current(session)? {
                    return Err(AppError::InvalidRequest(
                        "VPN session changed during probe".into(),
                    ));
                }
                json!({"delay":delay,"sessionId":session})
            }
            "diagnostics.watchGet" => value(self.core.service_watch_policy()?)?,
            "diagnostics.watchSet" => {
                let policy: paws_core::ServiceWatchPolicy = payload(p)?;
                self.core.save_service_watch_policy(policy)?;
                value(self.core.service_watch_policy()?)?
            }
            "diagnostics.serviceDns" => {
                let p: ServiceDns = payload(p)?;
                let profile = active(&before)?.to_owned();
                self.core
                    .validate_proxy_group(&p.group, p.expected_revision.get()?)?;
                let domains = service_dns::normalize_domains(&p.domains)?;
                let options = before.vpn_options.clone();
                let mut policy = options.dns_nameserver_policy;
                for domain in domains {
                    policy.insert(
                        format!("+.{domain}"),
                        vec![format!("https://1.1.1.1/dns-query#{}", p.group)],
                    );
                }
                self.core
                    .set_profile_dns_config_checked(
                        &profile,
                        p.expected_revision.get()?,
                        options.dns_servers,
                        options.dns_fallbacks,
                        policy,
                    )
                    .await?;
                Value::Null
            }
            "diagnostics.healthGet" => value(self.core.startup_health()?)?,
            "diagnostics.importQualification" => {
                let p: Content = payload(p)?;
                self.core.record_engine_qualification(&p.content)?;
                Value::Null
            }
            "diagnostics.report" => value(self.core.diagnostic_report()?)?,
            "diagnostics.preflight" => self.core.configuration_preflight()?,
            "backups.export" => value(self.core.export_portable_backup()?)?,
            "backups.restore" => {
                let p: Content = payload(p)?;
                value(self.core.restore_portable_backup(&p.content).await?)?
            }
            "backups.webdavUpload" => {
                let p: Webdav = payload(p)?;
                self.core
                    .upload_webdav_backup(&p.url, &p.username, &p.password)
                    .await?;
                Value::Null
            }
            "backups.webdavRestore" => {
                let p: Webdav = payload(p)?;
                value(
                    self.core
                        .restore_webdav_backup(&p.url, &p.username, &p.password)
                        .await?,
                )?
            }
            "preferences.get" => value(PreferencesStore::configured()?.snapshot()?)?,
            "preferences.set" => value(PreferencesStore::configured()?.update(payload(p)?)?)?,
            "converter.loadDraft" => value(self.converter()?.load_draft()?)?,
            "converter.saveDraft" => {
                let p: ConverterDraft = payload(p)?;
                self.converter()?.save_draft(&p.draft)?;
                Value::Null
            }
            "converter.buildUrl" => {
                let p: ConverterDraft = payload(p)?;
                value(self.converter()?.build_url(&p.draft)?)?
            }
            "converter.parseUrl" => {
                let p: Convert = payload(p)?;
                value(self.converter()?.parse_url(&p.input).await?)?
            }
            "converter.shortUrl" => {
                let p: ShortUrl = payload(p)?;
                value(self.converter()?.short_url(&p.api, &p.long_url).await?)?
            }
            "converter.uploadConfig" => {
                let p: UploadConfig = payload(p)?;
                value(self.converter()?.upload_config(&p.api, &p.content).await?)?
            }
            "converter.backendVersion" => {
                let p: Backend = payload(p)?;
                value(self.converter()?.backend_version(&p.backend).await?)?
            }
            "converter.convert" => {
                let p: Convert = payload(p)?;
                value(paws_profile::normalize_profile_content(&p.input)?)?
            }
            _ => {
                return Err(AppError::InvalidRequest(format!(
                    "unknown command: {}",
                    request.command
                )));
            }
        };
        if effect.is_none() && reconfigure {
            let after = self.core.command_context()?;
            effect = PlatformEffect::for_config_change(&before, &after)?;
        }
        Ok((data, effect))
    }
}

fn payload<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, AppError> {
    Ok(serde_json::from_value(value)?)
}
fn value<T: Serialize>(value: T) -> Result<Value, AppError> {
    Ok(serde_json::to_value(value)?)
}
fn active(snapshot: &CommandContext) -> Result<&str, AppError> {
    snapshot
        .active_profile
        .as_deref()
        .ok_or_else(|| AppError::InvalidRequest("select a profile first".into()))
}
