use super::*;
use std::future::Future;

impl UiServices {
    fn next_request_id(&self) -> u64 {
        let id = self
            .next_request_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1);
        if id == 0 {
            self.next_request_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(1)
        } else {
            id
        }
    }

    fn locale(&self) -> UiLocale {
        self.stores.preferences.peek().locale
    }

    pub(crate) fn notify(&self, message: impl Into<String>) {
        self.notifications.publish(message.into());
    }

    /// Run a mutation which has already crossed the commit boundary. The core
    /// work is intentionally not cancelled on root replacement; queue_ui's
    /// root-generation guard discards the obsolete completion callback.
    pub(crate) fn run_durable_mutation<F, T, C>(&self, future: F, complete: C)
    where
        F: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
        C: FnOnce(UiServices, Result<T, String>) + 'static,
    {
        let task = self.runtime.tokio().spawn(future);
        let runtime = self.runtime.clone();
        let services = self.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = task
                .await
                .map_err(|error| format!("Background task failed: {error}"))
                .and_then(|result| result);
            runtime.queue_ui(move || complete(services, result));
        });
    }

    pub(crate) fn refresh_config(&self) {
        match paws_core::shared_core().config_projection() {
            Ok(projection) => {
                self.stores.apply_config_projection(projection);
            }
            Err(error) => self.runtime_error(error.to_string()),
        }
    }

    fn refresh_telemetry(&self) {
        match paws_core::shared_core().telemetry_projection() {
            Ok(projection) => {
                self.stores.apply_telemetry_projection(projection);
            }
            Err(error) => self.runtime_error(error.to_string()),
        }
    }

    pub(crate) fn refresh_status(&self) {
        match paws_core::shared_core().runtime_status_projection() {
            Ok(projection) => {
                self.stores.apply_status_projection(projection);
            }
            Err(error) => self.runtime_error(error.to_string()),
        }
    }

    fn refresh_resources(&self) {
        match paws_core::shared_core().resource_projection() {
            Ok(projection) => {
                self.stores.apply_resource_projection(projection);
            }
            Err(error) => self.runtime_error(error.to_string()),
        }
    }

    fn runtime_error(&self, error: String) {
        let previous = self.stores.session.peek().runtime_error.clone();
        self.stores.set_runtime_error(error.clone());
        if previous.as_deref() != Some(error.as_str()) {
            self.notify(error);
        }
    }

    pub(crate) fn connected_session_owner(&self) -> Option<String> {
        self.stores
            .session
            .peek()
            .vpn_running
            .then(|| self.stores.session.peek().vpn_session_id.clone())
            .flatten()
    }

    fn begin_profile_import(&self) -> (u64, tokio::sync::watch::Receiver<bool>) {
        self.cancel_profile_import();
        let request_id = self
            .profile_import_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        *self
            .profile_import_cancel
            .lock()
            .expect("profile import lock") = Some(cancel_tx);
        self.operations.update_profile_import(|state| {
            state.loading = true;
            state.error = None;
            state.succeeded = false;
        });
        (request_id, cancel_rx)
    }

    fn finish_profile_import(&self, request_id: u64) -> bool {
        if self
            .profile_import_generation
            .load(std::sync::atomic::Ordering::Acquire)
            != request_id
        {
            return false;
        }
        *self
            .profile_import_cancel
            .lock()
            .expect("profile import lock") = None;
        self.operations
            .update_profile_import(|state| state.loading = false);
        true
    }

    pub(crate) fn cancel_profile_import(&self) {
        self.profile_import_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let Some(cancel) = self
            .profile_import_cancel
            .lock()
            .expect("profile import lock")
            .take()
        {
            cancel.send_replace(true);
        }
        self.operations.update_profile_import(|state| {
            state.loading = false;
            state.error = None;
            state.succeeded = false;
        });
    }

    pub(crate) fn reset_profile_import_feedback(&self) {
        self.operations.update_profile_import(|state| {
            state.error = None;
            state.succeeded = false;
        });
    }

    fn profile_import_is_current(&self, request_id: u64) -> bool {
        self.profile_import_generation
            .load(std::sync::atomic::Ordering::Acquire)
            == request_id
    }

    fn start_profile_import<F>(&self, future: F)
    where
        F: Future<Output = Result<ProfileImportPreparation, String>> + Send + 'static,
    {
        if self.operations.profile_import.peek().loading {
            return;
        }
        let locale = self.locale();
        let owner = self.connected_session_owner();
        let (request_id, cancel_rx) = self.begin_profile_import();
        let task = self
            .runtime
            .tokio()
            .spawn(run_profile_import_task(future, cancel_rx, locale));
        let runtime = self.runtime.clone();
        let services = self.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = task
                .await
                .map_err(|error| format!("Background task failed: {error}"))
                .and_then(|result| result);
            runtime.queue_ui(move || {
                if !services.profile_import_is_current(request_id) {
                    return;
                }
                match result {
                    Ok(ProfileImportPreparation::Ready(prepared)) => {
                        services.commit_profile_import(request_id, owner, prepared, locale);
                    }
                    Ok(ProfileImportPreparation::Cancelled) => {
                        services.finish_profile_import(request_id);
                    }
                    Err(error) => {
                        services.finish_profile_import_with_error(request_id, error, locale)
                    }
                }
            });
        });
    }

    fn commit_profile_import(
        &self,
        request_id: u64,
        owner: Option<String>,
        prepared: PreparedProfileMutation,
        locale: UiLocale,
    ) {
        let (owner, vpn_operation_id) =
            self.begin_owned_vpn_operation(owner, VpnCommandAction::Restart);
        self.run_durable_mutation(
            async move {
                let commit = commit_prepared_profile_import(prepared).await?;
                let restart = Self::restart_config_projection(owner, &commit.config).await;
                Ok((commit, restart))
            },
            move |services, result| {
                let import_is_current = services.finish_profile_import(request_id);
                match result {
                    Ok((mut commit, restart)) => {
                        let (requested, error, unconfirmed) =
                            services.finish_vpn_followup(vpn_operation_id, restart);
                        if !import_is_current {
                            return;
                        }
                        commit.result.restart_requested = requested;
                        commit.result.restart_error = error;
                        commit.result.restart_unconfirmed = unconfirmed;
                        services.stores.apply_config_projection(commit.config);
                        services.refresh_resources();
                        services.refresh_status();
                        services.operations.update_profile_import(|state| {
                            state.error = None;
                            state.succeeded = true;
                        });
                        if let Some(message) = commit.result.restart_unconfirmed.clone() {
                            services.notify(message);
                        }
                        services.notify(localized_profile_import_message(
                            &commit.result.profile_name,
                            commit.result.restart_requested,
                            commit.result.restart_error.as_deref(),
                            locale,
                        ));
                    }
                    Err(error) => {
                        if let Some(operation_id) = vpn_operation_id {
                            services.finish_vpn_failure(operation_id);
                        }
                        if import_is_current {
                            services.publish_profile_import_error(error, locale);
                        }
                    }
                }
            },
        );
    }

    fn finish_profile_import_with_error(&self, request_id: u64, error: String, locale: UiLocale) {
        if self.finish_profile_import(request_id) {
            self.publish_profile_import_error(error, locale);
        }
    }

    fn publish_profile_import_error(&self, error: String, locale: UiLocale) {
        let message = format!(
            "{}{}",
            translate_ui(locale, tr::profiles_import_failed_prefix()),
            error
        );
        self.operations.update_profile_import(|state| {
            state.error = Some(message.clone());
            state.succeeded = false;
        });
        self.notify(message);
    }

    pub(crate) fn import_local_profile(&self) {
        self.start_profile_import(prepare_local_profile_import());
    }

    pub(crate) fn scan_profile_subscription(&self, name: String) {
        let locale = self.locale();
        self.start_profile_import(prepare_scanned_profile_import(name, locale));
    }

    pub(crate) fn import_profile_from_url(&self, url: String, name: String) {
        let url = url.trim().to_owned();
        let locale = self.locale();
        if url.is_empty() {
            self.operations.update_profile_import(|state| {
                state.succeeded = false;
                state.error =
                    Some(translate_ui(locale, tr::profiles_import_url_required()).to_owned());
            });
            return;
        }
        let name = match name.trim() {
            "" => None,
            value => Some(value.to_owned()),
        };
        self.start_profile_import(prepare_profile_url_import(url, name));
    }

    pub(crate) fn activate_profile(&self, profile_id: String) {
        let profile_name = self
            .stores
            .profiles
            .peek()
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| profile_id.clone());
        let (owner, vpn_operation_id) = self
            .begin_owned_vpn_operation(self.connected_session_owner(), VpnCommandAction::Restart);
        let locale = self.locale();
        let expected_config_revision = self.stores.settings.peek().config_revision;
        self.run_durable_mutation(
            async move {
                let config = paws_core::shared_core()
                    .activate_profile_checked(&profile_id, expected_config_revision)
                    .await
                    .map_err(|error| error.to_string())?;
                let restart = Self::restart_config_projection(owner, &config).await;
                Ok((config, restart))
            },
            move |services, result| match result {
                Ok((config, restart)) => {
                    services.stores.apply_config_projection(config);
                    services.refresh_status();
                    let (requested, error, unconfirmed) =
                        services.finish_vpn_followup(vpn_operation_id, restart);
                    if let Some(message) = unconfirmed {
                        services.notify(message);
                    }
                    services.notify(profile_activation_message(
                        &profile_name,
                        requested,
                        error.as_deref(),
                        locale,
                    ));
                }
                Err(error) => {
                    if let Some(operation_id) = vpn_operation_id {
                        services.finish_vpn_failure(operation_id);
                    }
                    services.notify(format!(
                        "{}{}",
                        translate_ui(locale, tr::feedback_profile_activate_failed_prefix()),
                        error
                    ));
                }
            },
        );
    }

    pub(crate) fn delete_profile(&self, profile_id: String) {
        let (was_active, has_replacement, profile_name) = {
            let profiles = self.stores.profiles.peek();
            (
                profiles.active_profile.as_deref() == Some(profile_id.as_str()),
                profiles
                    .profiles
                    .iter()
                    .any(|profile| profile.id != profile_id),
                profiles
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| profile_id.clone()),
            )
        };
        let owner = was_active.then(|| self.connected_session_owner()).flatten();
        let action = if has_replacement {
            VpnCommandAction::Restart
        } else {
            VpnCommandAction::OwnedStop
        };
        let (owner, vpn_operation_id) = self.begin_owned_vpn_operation(owner, action);
        let locale = self.locale();
        self.run_durable_mutation(
            async move {
                let core = paws_core::shared_core();
                core.delete_profile(&profile_id)
                    .await
                    .map_err(|error| error.to_string())?;
                let config = core
                    .config_projection()
                    .map_err(|error| error.to_string())?;
                let (vpn_action, vpn_followup) = match owner {
                    Some(expected_owner) if config.active_profile.is_some() => (
                        Some(ProfileDeleteVpnAction::Restart),
                        Self::restart_config_projection(Some(expected_owner), &config).await,
                    ),
                    Some(expected_owner) => (
                        Some(ProfileDeleteVpnAction::Stop),
                        crate::bridge::request_stop_vpn_if_current(
                            expected_owner,
                            config.revisions.config_revision,
                        )
                        .await,
                    ),
                    None => (
                        None,
                        Ok(crate::bridge::VpnOperationOutcome::Completed(false)),
                    ),
                };
                Ok(ProfileDeleteResult {
                    vpn_action,
                    vpn_followup,
                })
            },
            move |services, result| match result {
                Ok(result) => {
                    services.refresh_config();
                    services.refresh_status();
                    let (requested, error, unconfirmed) =
                        services.finish_vpn_followup(vpn_operation_id, result.vpn_followup);
                    if let Some(message) = unconfirmed {
                        services.notify(message);
                    }
                    services.notify(profile_delete_message(
                        &profile_name,
                        requested
                            .then_some(result.vpn_action)
                            .flatten()
                            .map(|action| profile_delete_vpn_action_label(action, locale))
                            .as_deref(),
                        error.as_deref(),
                        locale,
                    ));
                }
                Err(error) => {
                    if let Some(operation_id) = vpn_operation_id {
                        services.finish_vpn_failure(operation_id);
                    }
                    services.notify(format!(
                        "{}{}",
                        translate_ui(locale, tr::feedback_profile_delete_failed_prefix()),
                        error
                    ));
                }
            },
        );
    }

    pub(crate) fn refresh_profile(&self, profile_id: String) {
        let profile_name = self
            .stores
            .profiles
            .peek()
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| profile_id.clone());
        let locale = self.locale();
        self.run_durable_mutation(
            async move {
                paws_core::shared_core()
                    .refresh_profile(&profile_id)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |services, result| {
                services.refresh_config();
                match result {
                    Ok(()) => services.notify(format!(
                        "{}{}{}",
                        translate_ui(locale, tr::feedback_subscription_prefix()),
                        profile_name,
                        translate_ui(locale, tr::feedback_subscription_refreshed_suffix())
                    )),
                    Err(error) => services.notify(format!(
                        "{}{}{}{}",
                        translate_ui(locale, tr::feedback_subscription_prefix()),
                        profile_name,
                        translate_ui(locale, tr::feedback_subscription_refresh_failed_suffix()),
                        error
                    )),
                }
            },
        );
    }

    pub(crate) fn refresh_all_profiles(&self) {
        let attempted = self
            .stores
            .profiles
            .peek()
            .profiles
            .iter()
            .filter(|profile| profile.subscription_url.is_some())
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>();
        let locale = self.locale();
        self.run_durable_mutation(
            async move {
                paws_core::shared_core()
                    .refresh_all_profiles()
                    .await
                    .map_err(|error| error.to_string())
            },
            move |services, result| match result {
                Ok(()) => {
                    services.refresh_config();
                    let failed = services
                        .stores
                        .profiles
                        .peek()
                        .profiles
                        .iter()
                        .filter(|profile| {
                            attempted.iter().any(|id| id == &profile.id)
                                && profile.last_refresh_error.is_some()
                        })
                        .count();
                    services.notify(profile_batch_refresh_message(
                        &translate_ui(locale, tr::feedback_profile_refresh_all_label()),
                        attempted.len(),
                        failed,
                        locale,
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_subscription_refresh_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn restore_profile_backup(&self, profile_id: String) {
        let (profile_name, active) = {
            let profiles = self.stores.profiles.peek();
            (
                profiles
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| profile_id.clone()),
                profiles.active_profile.as_deref() == Some(profile_id.as_str()),
            )
        };
        let owner = active.then(|| self.connected_session_owner()).flatten();
        let (owner, vpn_operation_id) =
            self.begin_owned_vpn_operation(owner, VpnCommandAction::Restart);
        let locale = self.locale();
        self.run_durable_mutation(
            async move {
                paws_core::shared_core()
                    .restore_profile_backup(&profile_id)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(Self::restart_owned_session(owner).await)
            },
            move |services, result| match result {
                Ok(restart) => {
                    services.refresh_config();
                    services.refresh_status();
                    let (requested, error, unconfirmed) =
                        services.finish_vpn_followup(vpn_operation_id, restart);
                    if let Some(message) = unconfirmed {
                        services.notify(message);
                    }
                    services.notify(profile_backup_restore_message(
                        &profile_name,
                        None,
                        requested,
                        error.as_deref(),
                        locale,
                    ));
                }
                Err(error) => {
                    if let Some(operation_id) = vpn_operation_id {
                        services.finish_vpn_failure(operation_id);
                    }
                    services.notify(profile_backup_restore_message(
                        &profile_name,
                        Some(&error),
                        false,
                        None,
                        locale,
                    ));
                }
            },
        );
    }

    pub(crate) fn update_profile_subscription(
        &self,
        profile_id: String,
        name: String,
        subscription_url: String,
    ) {
        let locale = self.locale();
        let profile_name = name.trim().to_owned();
        let revision = self.stores.settings.peek().config_revision;
        self.run_durable_mutation(
            async move {
                paws_core::shared_core()
                    .update_profile_subscription_checked(
                        &profile_id,
                        revision,
                        &name,
                        &subscription_url,
                    )
                    .map_err(|error| error.to_string())
            },
            move |services, result| match result {
                Ok(config) => {
                    services.stores.apply_config_projection(config);
                    services.notify(format!(
                        "{}{}",
                        profile_name,
                        translate_ui(locale, tr::hard_zh_065())
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::hard_zh_066()),
                    error
                )),
            },
        );
    }

    pub(crate) fn export_profile(&self, profile_id: String) {
        let profile_name = self
            .stores
            .profiles
            .peek()
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "profile".to_owned());
        let locale = self.locale();
        let raw_yaml = match paws_core::shared_core().profile_raw_yaml(&profile_id) {
            Ok(raw_yaml) => raw_yaml,
            Err(error) => {
                self.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::profiles_yaml_read_failed_prefix()),
                    error
                ));
                return;
            }
        };
        self.run_durable_mutation(
            async move {
                crate::bridge::export_profile(profile_name.clone(), raw_yaml).await?;
                Ok(profile_name)
            },
            move |services, result| match result {
                Ok(profile_name) => services.notify(format!(
                    "{}{}",
                    profile_name,
                    translate_ui(locale, tr::hard_zh_067())
                )),
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::hard_zh_068()),
                    error
                )),
            },
        );
    }

    async fn restart_owned_session(
        owner: Option<String>,
    ) -> Result<crate::bridge::VpnOperationOutcome<bool>, String> {
        let Some(owner) = owner else {
            return Ok(crate::bridge::VpnOperationOutcome::Completed(false));
        };
        let core = paws_core::shared_core();
        let config = core
            .config_projection()
            .map_err(|error| error.to_string())?;
        let options =
            serde_json::to_string(&config.vpn_options).map_err(|error| error.to_string())?;
        crate::bridge::request_restart_vpn(owner, config.revisions.config_revision, options).await
    }

    pub(crate) async fn restart_config_projection(
        owner: Option<String>,
        config: &paws_core::ConfigProjection,
    ) -> Result<crate::bridge::VpnOperationOutcome<bool>, String> {
        let Some(owner) = owner else {
            return Ok(crate::bridge::VpnOperationOutcome::Completed(false));
        };
        let options =
            serde_json::to_string(&config.vpn_options).map_err(|error| error.to_string())?;
        crate::bridge::request_restart_vpn(owner, config.revisions.config_revision, options).await
    }

    pub(crate) fn begin_owned_vpn_operation(
        &self,
        owner: Option<String>,
        action: VpnCommandAction,
    ) -> (Option<String>, Option<u64>) {
        let Some(owner) = owner else {
            return (None, None);
        };
        let operation_id = self.next_request_id();
        let mut began = false;
        self.operations.update_vpn(|operation| {
            began = operation.begin_owned(operation_id, action, Some(owner.clone()));
        });
        if began {
            (Some(owner), Some(operation_id))
        } else {
            // The durable configuration mutation may continue, but it must
            // not enqueue a second platform mutation behind an unknown one.
            (None, None)
        }
    }

    pub(crate) fn finish_vpn_followup(
        &self,
        operation_id: Option<u64>,
        result: Result<crate::bridge::VpnOperationOutcome<bool>, String>,
    ) -> (bool, Option<String>, Option<String>) {
        match result {
            Ok(crate::bridge::VpnOperationOutcome::Completed(requested)) => {
                if operation_id.is_some_and(|id| !self.finish_vpn_success(id)) {
                    return (false, None, None);
                }
                (requested, None, None)
            }
            Ok(crate::bridge::VpnOperationOutcome::Unconfirmed(operation)) => {
                let message = operation.message.clone();
                let Some(operation_id) = operation_id else {
                    return (false, None, Some(message));
                };
                let mut current = false;
                self.operations.update_vpn(|state| {
                    current = state.mark_unconfirmed(operation_id, operation);
                });
                if current {
                    (false, None, Some(message))
                } else {
                    (false, None, None)
                }
            }
            Err(error) => {
                if operation_id.is_some_and(|id| !self.finish_vpn_failure(id)) {
                    return (false, None, None);
                }
                (false, Some(error), None)
            }
        }
    }

    pub(crate) fn toggle_vpn(&self) {
        let (lifecycle, vpn_running) = {
            let session = self.stores.session.peek();
            (session.lifecycle, session.vpn_running)
        };
        if self.operations.vpn.peek().active.is_some()
            || matches!(lifecycle, VpnLifecycle::Starting)
        {
            self.notify(translate_ui(self.locale(), tr::hard_zh_053()));
            return;
        }
        let locale = self.locale();
        if vpn_running {
            let operation_id = self.next_request_id();
            let mut began = false;
            self.operations.update_vpn(|operation| {
                began = operation.begin(operation_id, VpnCommandAction::Stop);
            });
            if !began {
                return;
            }
            self.run_durable_mutation(stop_vpn_command(locale), move |services, result| {
                services.finish_vpn_command(operation_id, result)
            });
            return;
        }
        let profile_id = {
            let profiles = self.stores.profiles.peek();
            profiles
                .active_profile
                .clone()
                .or_else(|| profiles.profiles.first().map(|profile| profile.id.clone()))
        };
        let Some(profile_id) = profile_id else {
            self.notify(translate_ui(locale, tr::feedback_profile_required()));
            return;
        };
        let profile_name = self
            .stores
            .profiles
            .peek()
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| profile_id.clone());
        let operation_id = self.next_request_id();
        let mut began = false;
        self.operations.update_vpn(|operation| {
            began = operation.begin(operation_id, VpnCommandAction::Start);
        });
        if !began {
            return;
        }
        self.run_durable_mutation(
            start_vpn_command(profile_id, profile_name, locale),
            move |services, result| services.finish_vpn_command(operation_id, result),
        );
    }

    pub(crate) fn confirm_vpn_operation(&self) {
        let Some(active) = self.operations.vpn.peek().active.clone() else {
            return;
        };
        let mut operation = None;
        self.operations.update_vpn(|state| {
            operation = state.begin_confirmation(active.id);
        });
        let Some(operation) = operation else {
            return;
        };
        let action = active.action;
        self.run_durable_mutation(
            crate::bridge::confirm_vpn_operation(operation.receipt),
            move |services, result| services.finish_vpn_confirmation(active.id, action, result),
        );
    }

    pub(crate) fn recover_vpn_operation(&self) {
        let Some(active) = self.operations.vpn.peek().active.clone() else {
            return;
        };
        let recovery_id = self.next_request_id();
        let mut began = false;
        self.operations.update_vpn(|operation| {
            began = operation.begin_recovery(active.id, recovery_id);
        });
        if !began {
            return;
        }
        let locale = self.locale();
        self.run_durable_mutation(stop_vpn_command(locale), move |services, result| {
            services.finish_vpn_command(recovery_id, result)
        });
    }

    fn finish_vpn_confirmation(
        &self,
        operation_id: u64,
        action: VpnCommandAction,
        result: Result<crate::bridge::VpnOperationOutcome<bool>, String>,
    ) {
        match result {
            Ok(crate::bridge::VpnOperationOutcome::Completed(applied)) => {
                if !self.finish_vpn_success(operation_id) {
                    return;
                }
                if applied {
                    self.notify(vpn_command_message(action, None, None, self.locale()));
                } else {
                    self.notify(translate_ui(
                        self.locale(),
                        tr::vpn_operation_confirmed_not_applied(),
                    ));
                }
            }
            Ok(crate::bridge::VpnOperationOutcome::Unconfirmed(operation)) => {
                let message = operation.message.clone();
                let mut current = false;
                self.operations.update_vpn(|state| {
                    current = state.mark_unconfirmed(operation_id, operation);
                });
                if current {
                    self.refresh_config();
                    self.refresh_status();
                    self.notify(message);
                }
            }
            Err(error) => {
                if !self.finish_vpn_failure(operation_id) {
                    return;
                }
                self.notify(vpn_command_message(
                    action,
                    None,
                    Some(&error),
                    self.locale(),
                ));
            }
        }
    }

    fn finish_vpn_success(&self, operation_id: u64) -> bool {
        let mut current = false;
        self.operations.update_vpn(|state| {
            current = state.finish_success(operation_id);
        });
        if current {
            self.refresh_config();
            self.refresh_status();
        }
        current
    }

    pub(crate) fn finish_vpn_failure(&self, operation_id: u64) -> bool {
        let mut disposition = VpnOperationFailureDisposition::Stale;
        self.operations.update_vpn(|state| {
            disposition = state.finish_failure(operation_id);
        });
        if disposition == VpnOperationFailureDisposition::Stale {
            return false;
        }
        self.refresh_config();
        self.refresh_status();
        true
    }

    fn finish_vpn_command(&self, operation_id: u64, result: Result<VpnCommandResult, String>) {
        match result {
            Ok(mut result) => {
                if let Some(operation) = result.request_unconfirmed.take() {
                    let message = operation.message.clone();
                    let mut current = false;
                    self.operations.update_vpn(|state| {
                        current = state.mark_unconfirmed(operation_id, operation);
                    });
                    if !current {
                        return;
                    }
                    self.refresh_config();
                    self.refresh_status();
                    self.notify(message);
                    return;
                }
                let current = if result.request_error.is_some() {
                    self.finish_vpn_failure(operation_id)
                } else {
                    self.finish_vpn_success(operation_id)
                };
                if !current {
                    return;
                }
                self.notify(vpn_command_message(
                    result.action,
                    result.profile_name.as_deref(),
                    result.request_error.as_deref(),
                    self.locale(),
                ));
            }
            Err(error) => {
                if self.finish_vpn_failure(operation_id) {
                    self.notify(error);
                }
            }
        }
    }

    pub(crate) fn set_mode(&self, mode: RuntimeMode) {
        let locale = self.locale();
        let generation = self
            .mode_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            .wrapping_add(1);
        let current_generation = self.mode_generation.clone();
        self.run_durable_mutation(
            async move {
                let core = paws_core::shared_core();
                if mode == RuntimeMode::Global {
                    let prepared = core.prepare_active_vpn().await;
                    if current_generation.load(std::sync::atomic::Ordering::Acquire) != generation {
                        return Ok(None);
                    }
                    prepared.map_err(|error| error.to_string())?;
                }
                if current_generation.load(std::sync::atomic::Ordering::Acquire) != generation {
                    return Ok(None);
                }
                core.set_mode(mode).map_err(|error| error.to_string())?;
                Ok(Some(mode))
            },
            move |services, result| match result {
                Ok(Some(mode)) => {
                    services.refresh_config();
                    services.refresh_status();
                    services.notify(mode_changed_message(mode, locale));
                }
                Ok(None) => {}
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_mode_change_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn select_proxy(&self, group: String, proxy: Option<String>) {
        if self.operations.proxy.peek().selection_pending.is_some() {
            return;
        }
        let pending_proxy = proxy.clone().unwrap_or_default();
        self.operations.update_proxy(|current| {
            current.selection_pending = Some((group.clone(), pending_proxy));
        });
        let locale = self.locale();
        let future = async move {
            match proxy {
                Some(proxy) => select_proxy(group, proxy).await,
                None => unfix_proxy(group).await,
            }
        };
        self.run_durable_mutation(future, move |services, result| {
            services
                .operations
                .update_proxy(|current| current.selection_pending = None);
            match result {
                Ok(_) => services.refresh_status(),
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_proxy_switch_failed_prefix()),
                    error
                )),
            }
        });
    }

    pub(crate) fn test_all_proxy_delays(&self) {
        let groups = {
            let proxies = self.stores.proxies.peek();
            if self.operations.proxy.peek().delay_loading {
                return;
            }
            proxies
                .groups
                .iter()
                .filter(|group| {
                    !group.name.eq_ignore_ascii_case("GLOBAL") && !group.proxies.is_empty()
                })
                .map(|group| (group.name.clone(), group.proxies.len()))
                .collect::<Vec<_>>()
        };
        let locale = self.locale();
        if groups.is_empty() {
            self.notify(translate_ui(locale, tr::feedback_proxy_delay_empty()));
            return;
        }
        self.operations
            .update_proxy(|current| current.delay_loading = true);
        self.run_durable_mutation(test_proxy_delays(groups), move |services, result| {
            services
                .operations
                .update_proxy(|current| current.delay_loading = false);
            match result {
                Ok(result) => {
                    services.refresh_status();
                    services.refresh_resources();
                    services.notify(format!(
                        "{}{}{}{}{}",
                        translate_ui(locale, tr::feedback_proxy_delay_batch_prefix()),
                        result.succeeded,
                        translate_ui(locale, tr::feedback_provider_batch_success_mid()),
                        result.failed,
                        translate_ui(locale, tr::feedback_provider_batch_failed_suffix())
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_proxy_delay_batch_failed_prefix()),
                    error
                )),
            }
        });
    }

    pub(crate) fn flush_dns_cache(&self) {
        let locale = self.locale();
        self.run_diagnostic("dns".to_owned(), async move {
            paws_core::shared_core()
                .flush_dns_cache_via_controller()
                .await
                .map_err(|error| error.to_string())?;
            Ok(translate_ui(locale, tr::hard_zh_054()).to_owned())
        });
    }

    pub(crate) fn flush_fake_ip_cache(&self) {
        let locale = self.locale();
        self.run_diagnostic("fakeip".to_owned(), async move {
            paws_core::shared_core()
                .flush_fake_ip_cache_via_controller()
                .await
                .map_err(|error| error.to_string())?;
            Ok(translate_ui(locale, tr::hard_zh_055()).to_owned())
        });
    }

    fn run_diagnostic<F>(&self, key: String, future: F)
    where
        F: Future<Output = Result<String, String>> + Send + 'static,
    {
        if self.operations.diagnostics.peek().pending.is_some() {
            return;
        }
        let locale = self.locale();
        self.operations
            .update_diagnostics(|current| current.pending = Some(key));
        self.run_durable_mutation(future, move |services, result| {
            services
                .operations
                .update_diagnostics(|current| current.pending = None);
            match result {
                Ok(message) => {
                    services.refresh_telemetry();
                    services.notify(message);
                }
                Err(error) => services.notify(translate_ui(locale, tr::hard_zh_050(error))),
            }
        });
    }

    pub(crate) fn clear_request_history(&self) {
        let locale = self.locale();
        self.run_durable_mutation(
            clear_request_history(),
            move |services, result| match result {
                Ok(_) => {
                    services.refresh_telemetry();
                    services.notify(translate_ui(locale, tr::feedback_request_history_cleared()));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_request_history_clear_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn close_connection(&self, connection_id: String) {
        let locale = self.locale();
        self.run_durable_mutation(close_connection(connection_id), move |services, result| {
            match result {
                Ok(_) => {
                    services.refresh_telemetry();
                    services.notify(translate_ui(locale, tr::feedback_connection_closed()));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_connection_close_failed_prefix()),
                    error
                )),
            }
        });
    }

    pub(crate) fn close_all_connections(&self) {
        let locale = self.locale();
        self.run_durable_mutation(
            close_all_connections(),
            move |services, result| match result {
                Ok(_) => {
                    services.refresh_telemetry();
                    services.notify(translate_ui(locale, tr::feedback_connections_closed()));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_connections_close_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn open_manual_rule_editor(
        &self,
        local: LocalRuleEditors,
        connection_id: Option<String>,
        domain: String,
        destination_ip: String,
    ) {
        let domain = domain.trim().to_owned();
        let destination_ip = destination_ip.trim().to_owned();
        let (match_kind, value) = if domain.is_empty() {
            if destination_ip.is_empty() {
                (ManualRuleMatchKind::Domain, String::new())
            } else {
                (ManualRuleMatchKind::IpCidr, destination_ip.clone())
            }
        } else {
            (ManualRuleMatchKind::Domain, domain.clone())
        };
        local.update(|editors| {
            editors.manual = Some(ManualRuleEditorState {
                connection_id,
                domain,
                destination_ip,
                match_kind,
                value,
                target: "DIRECT".to_owned(),
                disconnect_after_save: false,
                submitting: false,
                error: None,
            });
        });
    }

    pub(crate) fn close_manual_rule_editor(&self, local: LocalRuleEditors) {
        local.update(|editors| {
            if !editors
                .manual
                .as_ref()
                .is_some_and(|editor| editor.submitting)
            {
                editors.manual = None;
            }
        });
    }

    pub(crate) fn set_manual_rule_match_kind(
        &self,
        local: LocalRuleEditors,
        match_kind: ManualRuleMatchKind,
    ) {
        local.update(|editors| {
            let Some(editor) = editors.manual.as_mut().filter(|editor| !editor.submitting) else {
                return;
            };
            editor.match_kind = match_kind;
            editor.value = match match_kind {
                ManualRuleMatchKind::Domain | ManualRuleMatchKind::DomainSuffix => {
                    editor.domain.clone()
                }
                ManualRuleMatchKind::IpCidr => editor.destination_ip.clone(),
            };
            editor.error = None;
        });
    }

    pub(crate) fn set_manual_rule_value(&self, local: LocalRuleEditors, value: String) {
        local.update(|editors| {
            if let Some(editor) = editors.manual.as_mut().filter(|editor| !editor.submitting) {
                editor.value = value;
                editor.error = None;
            }
        });
    }

    pub(crate) fn set_manual_rule_target(&self, local: LocalRuleEditors, target: String) {
        local.update(|editors| {
            if let Some(editor) = editors.manual.as_mut().filter(|editor| !editor.submitting) {
                editor.target = target;
                editor.error = None;
            }
        });
    }

    pub(crate) fn set_manual_rule_disconnect(&self, local: LocalRuleEditors, disconnect: bool) {
        local.update(|editors| {
            if let Some(editor) = editors.manual.as_mut().filter(|editor| !editor.submitting) {
                editor.disconnect_after_save = disconnect;
            }
        });
    }

    pub(crate) fn save_manual_rule(&self, local: LocalRuleEditors) {
        let editors = local.signal.peek().clone();
        let Some(editor) = editors.manual.filter(|editor| !editor.submitting) else {
            return;
        };
        let Some(profile_id) = self.stores.profiles.peek().active_profile.clone() else {
            let locale = self.locale();
            local.update(|editors| {
                if let Some(editor) = editors.manual.as_mut() {
                    editor.error = Some(if locale == UiLocale::ZhCn {
                        translate_ui(locale, tr::hard_zh_056()).to_owned()
                    } else {
                        "Activate a profile before adding a rule".to_owned()
                    });
                }
            });
            return;
        };
        local.update(|editors| {
            if let Some(editor) = editors.manual.as_mut() {
                editor.submitting = true;
                editor.error = None;
            }
        });
        let spec = ManualRuleSpec {
            match_kind: editor.match_kind,
            value: editor.value,
            target: editor.target,
        };
        let connection_id = editor
            .disconnect_after_save
            .then_some(editor.connection_id)
            .flatten();
        let locale = self.locale();
        self.run_durable_mutation(
            apply_manual_rule(profile_id, spec, connection_id),
            move |services, result| {
                if !local.is_alive() {
                    return;
                }
                match result {
                    Ok(result) => {
                        let message = manual_rule_saved_message(&result, locale);
                        services.refresh_config();
                        services.refresh_telemetry();
                        local.update(|editors| editors.manual = None);
                        services.notify(message);
                    }
                    Err(error) => local.update(|editors| {
                        if let Some(editor) = editors.manual.as_mut() {
                            editor.submitting = false;
                            editor.error = Some(error);
                        }
                    }),
                }
            },
        );
    }

    pub(crate) fn open_rule_lookup(&self, local: LocalRuleEditors) {
        local.cancel_query();
        let id = self.next_request_id();
        local.update(|editors| {
            editors.lookup = Some(RuleLookupState {
                id,
                query: String::new(),
                submitting: false,
                result: None,
                error: None,
            });
        });
    }

    pub(crate) fn close_rule_lookup(&self, local: LocalRuleEditors) {
        local.cancel_query();
        local.update(|editors| {
            editors.lookup = None;
        });
    }

    pub(crate) fn set_rule_lookup_query(&self, local: LocalRuleEditors, query: String) {
        local.update(|editors| {
            let Some(lookup) = editors.lookup.as_mut().filter(|lookup| !lookup.submitting) else {
                return;
            };
            lookup.query = query;
            lookup.result = None;
            lookup.error = None;
        });
    }

    pub(crate) fn lookup_rule(&self, local: LocalRuleEditors) {
        let editors = local.signal.peek().clone();
        let Some(lookup) = editors.lookup.filter(|lookup| !lookup.submitting) else {
            return;
        };
        let locale = self.locale();
        if self.stores.profiles.peek().active_profile.is_none() {
            local.update(|editors| {
                if let Some(lookup) = editors.lookup.as_mut() {
                    lookup.error = Some(if locale == UiLocale::ZhCn {
                        translate_ui(locale, tr::hard_zh_056()).to_owned()
                    } else {
                        "Activate a profile before querying rules".to_owned()
                    });
                }
            });
            return;
        }
        if lookup.query.trim().is_empty() {
            local.update(|editors| {
                if let Some(lookup) = editors.lookup.as_mut() {
                    lookup.error = Some(if locale == UiLocale::ZhCn {
                        translate_ui(locale, tr::hard_zh_057()).to_owned()
                    } else {
                        "Enter a domain name or IP address".to_owned()
                    });
                }
            });
            return;
        }
        let lookup_id = lookup.id;
        local.update(|editors| {
            if let Some(lookup) = editors.lookup.as_mut() {
                lookup.submitting = true;
                lookup.result = None;
                lookup.error = None;
            }
        });
        let task = self.runtime.tokio().spawn(lookup_rule(lookup.query));
        local.replace_query(task.abort_handle());
        let runtime = self.runtime.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = match task.await {
                Ok(result) => result,
                Err(error) if error.is_cancelled() => return,
                Err(error) => Err(format!("Background task failed: {error}")),
            };
            runtime.queue_ui(move || {
                if !local.is_alive() {
                    return;
                }
                local.update(|editors| {
                    let Some(lookup) = editors
                        .lookup
                        .as_mut()
                        .filter(|lookup| lookup.id == lookup_id)
                    else {
                        return;
                    };
                    local.clear_query();
                    lookup.submitting = false;
                    match result {
                        Ok(result) => {
                            lookup.result = Some(result);
                            lookup.error = None;
                        }
                        Err(error) => {
                            lookup.result = None;
                            lookup.error = Some(error);
                        }
                    }
                });
            });
        });
    }

    pub(crate) fn add_rule_from_lookup(&self, local: LocalRuleEditors) {
        let result = local
            .signal
            .peek()
            .lookup
            .as_ref()
            .and_then(|lookup| lookup.result.clone());
        let Some(result) = result else {
            return;
        };
        self.close_rule_lookup(local.clone());
        let (domain, destination_ip) = match result.input_kind {
            paws_core::RuleLookupInputKind::Domain => (result.query, String::new()),
            paws_core::RuleLookupInputKind::Ip => (String::new(), result.query),
        };
        self.open_manual_rule_editor(local, None, domain, destination_ip);
    }

    pub(crate) fn healthcheck_proxy_provider(&self, provider_name: String) {
        let key = format!("provider:{provider_name}");
        let locale = self.locale();
        self.run_diagnostic(key, async move {
            paws_core::shared_core()
                .healthcheck_proxy_provider_via_controller(&provider_name)
                .await
                .map_err(|error| error.to_string())?;
            Ok(translate_ui(locale, tr::hard_zh_049(provider_name)))
        });
    }

    pub(crate) fn healthcheck_provider_proxy(
        &self,
        provider_name: String,
        proxy_name: String,
        url: String,
        expected_status: Option<String>,
    ) {
        let key = format!("provider:{provider_name}:{proxy_name}");
        self.run_diagnostic(key, async move {
            let delay = paws_core::shared_core()
                .healthcheck_provider_proxy_via_controller(
                    &provider_name,
                    &proxy_name,
                    &url,
                    Some(5000),
                    expected_status.as_deref(),
                )
                .await
                .map_err(|error| error.to_string())?;
            Ok(format!("{proxy_name}: {delay} ms"))
        });
    }

    pub(crate) fn refresh_provider(&self, provider_type: String, provider_name: String) {
        let locale = self.locale();
        let expected_revision = self.stores.resource_revision();
        self.run_durable_mutation(
            async move {
                let projection = paws_core::shared_core()
                    .refresh_provider_checked(&provider_type, &provider_name, expected_revision)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok((provider_name, projection))
            },
            move |services, result| match result {
                Ok((provider_name, projection)) => {
                    services.stores.apply_resource_projection(projection);
                    services.refresh_status();
                    services.notify(format!(
                        "{}{}{}",
                        translate_ui(locale, tr::feedback_resource_prefix()),
                        provider_name,
                        translate_ui(locale, tr::feedback_resource_refreshed_suffix())
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_resource_refresh_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn refresh_all_providers(&self) {
        let attempted = self
            .stores
            .resources
            .peek()
            .providers
            .iter()
            .map(|provider| (provider.provider_type.clone(), provider.name.clone()))
            .collect::<Vec<_>>();
        let locale = self.locale();
        self.run_durable_mutation(
            async move {
                let error = paws_core::shared_core()
                    .refresh_all_providers()
                    .await
                    .err()
                    .map(|error| error.to_string());
                Ok(error)
            },
            move |services, result| match result {
                Ok(error) => {
                    services.refresh_resources();
                    services.refresh_status();
                    let resources = services.stores.resources.peek();
                    let failed = resources
                        .providers
                        .iter()
                        .filter(|provider| {
                            attempted.iter().any(|(kind, name)| {
                                kind == &provider.provider_type && name == &provider.name
                            }) && provider.last_refresh_error.is_some()
                        })
                        .count();
                    services.notify(provider_batch_refresh_message(
                        attempted.len(),
                        failed,
                        error.as_deref(),
                        locale,
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_resource_refresh_failed_prefix()),
                    error
                )),
            },
        );
    }

    pub(crate) fn cancel_rule_import(&self) {
        self.operations
            .update_resources(|resources| resources.rule_import_loading = false);
    }

    pub(crate) fn import_rules(&self, page_tasks: super::view::PageTasks) {
        if self.operations.resources.peek().rule_import_loading {
            return;
        }
        let Some(profile_id) = self.stores.profiles.peek().active_profile.clone() else {
            self.notify(translate_ui(
                self.locale(),
                tr::feedback_active_profile_required(),
            ));
            return;
        };
        let owner = self.connected_session_owner();
        let locale = self.locale();
        let expected_config_revision = self.stores.settings.peek().config_revision;
        self.operations
            .update_resources(|resources| resources.rule_import_loading = true);
        let services = self.clone();
        page_tasks.query(
            async move {
                let Some((name, rules_text)) = crate::bridge::pick_profile_text().await? else {
                    return Ok(None);
                };
                let source = format!("rules:{name}");
                let prepared = paws_core::CoreHandle::prepare_rule_import(&source, &rules_text)
                    .map_err(|error| error.to_string())?;
                Ok(Some(prepared))
            },
            move |result| match result {
                Ok(Some(prepared)) => {
                    let current_profile = services.stores.profiles.peek().active_profile.clone();
                    let current_revision = services.stores.settings.peek().config_revision;
                    if current_profile.as_deref() != Some(profile_id.as_str())
                        || current_revision != expected_config_revision
                    {
                        services.cancel_rule_import();
                        services.notify(format!(
                            "{}{}",
                            translate_ui(locale, tr::feedback_rule_import_failed_prefix()),
                            translate_ui(locale, tr::feedback_rule_import_state_changed())
                        ));
                        return;
                    }
                    let commit_services = services.clone();
                    let (owner, vpn_operation_id) =
                        commit_services.begin_owned_vpn_operation(owner, VpnCommandAction::Restart);
                    commit_services.run_durable_mutation(
                        async move {
                            let receipt = paws_core::shared_core()
                                .commit_prepared_rule_import_checked(
                                    &profile_id,
                                    expected_config_revision,
                                    prepared,
                                )
                                .await
                                .map_err(|error| error.to_string())?;
                            let restart =
                                Self::restart_config_projection(owner, &receipt.config).await;
                            Ok((receipt, restart))
                        },
                        move |commit_services, result| {
                            commit_services.cancel_rule_import();
                            match result {
                                Ok((receipt, restart)) => {
                                    let count = receipt.imported_rule_ids.len();
                                    commit_services
                                        .stores
                                        .apply_config_projection(receipt.config);
                                    commit_services.refresh_resources();
                                    commit_services.refresh_status();
                                    let (requested, error, unconfirmed) = commit_services
                                        .finish_vpn_followup(vpn_operation_id, restart);
                                    if let Some(message) = unconfirmed {
                                        commit_services.notify(message);
                                    }
                                    commit_services.notify(rule_import_message(
                                        count,
                                        None,
                                        requested,
                                        error.as_deref(),
                                        locale,
                                    ));
                                }
                                Err(error) => {
                                    if let Some(operation_id) = vpn_operation_id {
                                        commit_services.finish_vpn_failure(operation_id);
                                    }
                                    commit_services.notify(format!(
                                        "{}{}",
                                        translate_ui(
                                            locale,
                                            tr::feedback_rule_import_failed_prefix(),
                                        ),
                                        error
                                    ));
                                }
                            }
                        },
                    );
                }
                Ok(None) => services.cancel_rule_import(),
                Err(error) => {
                    services.cancel_rule_import();
                    services.notify(format!(
                        "{}{}",
                        translate_ui(locale, tr::feedback_rule_import_failed_prefix()),
                        error
                    ));
                }
            },
        );
    }

    pub(crate) fn set_rule_enabled(&self, profile_id: String, rule_id: String, enabled: bool) {
        let revision = self.stores.settings.peek().config_revision;
        let (owner, vpn_operation_id) = self
            .begin_owned_vpn_operation(self.connected_session_owner(), VpnCommandAction::Restart);
        self.run_rule_change(vpn_operation_id, async move {
            let config = paws_core::shared_core()
                .set_rule_enabled_checked(&profile_id, revision, &rule_id, enabled)
                .await
                .map_err(|error| error.to_string())?;
            let restart = Self::restart_config_projection(owner, &config).await;
            Ok((config, restart))
        });
    }

    pub(crate) fn reorder_rules(&self, profile_id: String, ordered_rule_ids: Vec<String>) {
        let revision = self.stores.settings.peek().config_revision;
        let (owner, vpn_operation_id) = self
            .begin_owned_vpn_operation(self.connected_session_owner(), VpnCommandAction::Restart);
        self.run_rule_change(vpn_operation_id, async move {
            let config = paws_core::shared_core()
                .reorder_rules_checked(&profile_id, revision, ordered_rule_ids)
                .await
                .map_err(|error| error.to_string())?;
            let restart = Self::restart_config_projection(owner, &config).await;
            Ok((config, restart))
        });
    }

    pub(crate) fn delete_rule(&self, profile_id: String, rule_id: String) {
        let revision = self.stores.settings.peek().config_revision;
        let (owner, vpn_operation_id) = self
            .begin_owned_vpn_operation(self.connected_session_owner(), VpnCommandAction::Restart);
        self.run_rule_change(vpn_operation_id, async move {
            let config = paws_core::shared_core()
                .delete_rule_checked(&profile_id, revision, &rule_id)
                .await
                .map_err(|error| error.to_string())?;
            let restart = Self::restart_config_projection(owner, &config).await;
            Ok((config, restart))
        });
    }

    fn run_rule_change<F>(&self, vpn_operation_id: Option<u64>, future: F)
    where
        F: Future<
                Output = Result<
                    (
                        paws_core::ConfigProjection,
                        Result<crate::bridge::VpnOperationOutcome<bool>, String>,
                    ),
                    String,
                >,
            > + Send
            + 'static,
    {
        let locale = self.locale();
        self.run_durable_mutation(future, move |services, result| match result {
            Ok((config, restart)) => {
                services.stores.apply_config_projection(config);
                services.refresh_resources();
                services.refresh_status();
                let (requested, error, unconfirmed) =
                    services.finish_vpn_followup(vpn_operation_id, restart);
                if let Some(message) = unconfirmed {
                    services.notify(message);
                }
                services.notify(settings_saved_message(
                    &translate_ui(locale, tr::feedback_label_rules()),
                    requested,
                    error.as_deref(),
                    locale,
                ));
            }
            Err(error) => {
                if let Some(operation_id) = vpn_operation_id {
                    services.finish_vpn_failure(operation_id);
                }
                services.notify(format!(
                    "{}{}",
                    translate_ui(locale, tr::feedback_rule_update_failed_prefix()),
                    error
                ));
            }
        });
    }

    pub(crate) fn toggle_log_recording(&self) {
        let next_enabled = {
            if self.operations.logs.peek().recording_pending {
                return;
            }
            !self.stores.logs.peek().recording.enabled
        };
        self.operations
            .update_logs(|logs| logs.recording_pending = true);
        let locale = self.locale();
        self.run_durable_mutation(set_log_recording(next_enabled), move |services, result| {
            services
                .operations
                .update_logs(|logs| logs.recording_pending = false);
            match result {
                Ok(result) => {
                    let enabled = result.status.enabled;
                    services.stores.update_logs(|logs| {
                        logs.recording_error = result.status.last_error.clone();
                        logs.recording = result.status;
                    });
                    services.refresh_telemetry();
                    services.notify(
                        if enabled {
                            translate_ui(locale, tr::hard_zh_058())
                        } else {
                            translate_ui(locale, tr::hard_zh_059())
                        }
                        .to_owned(),
                    );
                }
                Err(error) => services.notify(format!(
                    "{}{error}",
                    translate_ui(locale, tr::hard_zh_060())
                )),
            }
        });
    }

    pub(crate) fn export_log_archive(&self, file_name: String) {
        let operation_pending = {
            let logs = self.operations.logs.peek();
            logs.archive_export_pending.is_some() || logs.archive_delete_pending.is_some()
        };
        if operation_pending {
            return;
        }
        self.operations
            .update_logs(|logs| logs.archive_export_pending = Some(file_name.clone()));
        let locale = self.locale();
        self.run_durable_mutation(export_log_archive(file_name), move |services, result| {
            services
                .operations
                .update_logs(|logs| logs.archive_export_pending = None);
            match result {
                Ok(file_name) => services.notify(format!(
                    "{}{file_name}",
                    translate_ui(locale, tr::hard_zh_061())
                )),
                Err(error) => services.notify(format!(
                    "{}{error}",
                    translate_ui(locale, tr::hard_zh_062())
                )),
            }
        });
    }

    pub(crate) fn delete_log_archive(&self, file_name: String) {
        let operation_pending = {
            let logs = self.operations.logs.peek();
            logs.archive_export_pending.is_some() || logs.archive_delete_pending.is_some()
        };
        if operation_pending {
            return;
        }
        self.operations
            .update_logs(|logs| logs.archive_delete_pending = Some(file_name.clone()));
        let locale = self.locale();
        self.run_durable_mutation(delete_log_archive(file_name), move |services, result| {
            services
                .operations
                .update_logs(|logs| logs.archive_delete_pending = None);
            match result {
                Ok(result) => {
                    services.stores.update_logs(|logs| {
                        logs.recording_error = result.status.last_error.clone();
                        logs.recording = result.status;
                    });
                    services.refresh_telemetry();
                    services.notify(format!(
                        "{}{}",
                        translate_ui(locale, tr::hard_zh_063()),
                        result.file_name
                    ));
                }
                Err(error) => services.notify(format!(
                    "{}{error}",
                    translate_ui(locale, tr::hard_zh_064())
                )),
            }
        });
    }
}
