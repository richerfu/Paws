use super::*;
use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;
use std::time::Duration;

const PROFILE_IMPORT_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) async fn run_profile_import_task<F>(
    task: F,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    locale: UiLocale,
) -> Result<ProfileImportPreparation, String>
where
    F: Future<Output = Result<ProfileImportPreparation, String>> + Send,
{
    let cancelled = async move {
        if *cancel_rx.borrow() {
            return;
        }
        while cancel_rx.changed().await.is_ok() {
            if *cancel_rx.borrow() {
                return;
            }
        }
    };

    tokio::select! {
        biased;
        _ = cancelled => Ok(ProfileImportPreparation::Cancelled),
        result = task => result,
        _ = tokio::time::sleep(PROFILE_IMPORT_TIMEOUT) => {
            Err(translate_ui(locale, tr::profiles_import_timeout()).to_owned())
        }
    }
}

pub(super) enum ProfileImportPreparation {
    Cancelled,
    Ready(PreparedProfileMutation),
}

pub(super) enum PreparedProfileMutation {
    Import {
        prepared: paws_core::PreparedProfileImport,
        expected_config_revision: u64,
    },
    Refresh {
        profile_id: String,
        name: Option<String>,
        subscription_url: String,
        prepared: paws_core::PreparedProfileRefresh,
        expected_config_revision: u64,
    },
}

pub(super) struct ProfileImportCommitResult {
    pub result: ProfileImportResult,
    pub config: paws_core::ConfigProjection,
}

pub(super) async fn bootstrap_active_profile() -> Result<(), String> {
    let core = paws_core::shared_core();
    // The typed stores restore a revision-checked proxy-group cache synchronously,
    // so the dashboard can render immediately. Parse the complete meow config
    // only after the first frame, then replace the cache-backed snapshot.
    let active_profile = core
        .config_projection()
        .map_err(|error| error.to_string())?
        .active_profile;
    if active_profile.is_some() {
        core.prepare_active_vpn()
            .await
            .map_err(|error| error.to_string())?;
    }
    // Keep the refresh inside the root-owned Dioxus future. A nested Tokio
    // task would detach when the root is disposed and could outlive the UI
    // generation which initiated it.
    core.refresh_due_profiles()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn lookup_rule(query: String) -> Result<paws_core::RuleLookupResult, String> {
    paws_core::shared_core()
        .lookup_rule(&query)
        .await
        .map_err(|error| error.to_string())
}

pub(super) async fn start_vpn_command(
    profile_id: String,
    profile_name: String,
    locale: UiLocale,
) -> Result<VpnCommandResult, String> {
    let core = paws_core::shared_core();
    let mut config = core
        .config_projection()
        .map_err(|error| error.to_string())?;
    if config.active_profile.as_deref() != Some(profile_id.as_str()) {
        config = core
            .activate_profile_checked(&profile_id, config.revisions.config_revision)
            .await
            .map_err(|error| {
                format!(
                    "{}{}{}{}",
                    translate_ui(locale, tr::feedback_vpn_start_profile_load_failed_prefix()),
                    profile_name,
                    translate_ui(locale, tr::feedback_vpn_start_profile_load_failed_mid()),
                    error
                )
            })?;
    }
    let options_json = serde_json::to_string(&config.vpn_options).map_err(|error| {
        format!(
            "{}{}",
            translate_ui(locale, tr::feedback_vpn_start_options_failed_prefix()),
            error
        )
    })?;
    let (request_error, request_unconfirmed) = match crate::bridge::request_start_vpn(options_json)
        .await
    {
        Ok(crate::bridge::VpnOperationOutcome::Completed(())) => (None, None),
        Ok(crate::bridge::VpnOperationOutcome::Unconfirmed(operation)) => (None, Some(operation)),
        Err(error) => (Some(error), None),
    };
    Ok(VpnCommandResult {
        action: VpnCommandAction::Start,
        profile_name: Some(profile_name),
        request_error,
        request_unconfirmed,
    })
}

pub(super) async fn stop_vpn_command(locale: UiLocale) -> Result<VpnCommandResult, String> {
    let (request_error, request_unconfirmed) = match crate::bridge::request_stop_vpn().await {
        Ok(crate::bridge::VpnOperationOutcome::Completed(())) => (None, None),
        Ok(crate::bridge::VpnOperationOutcome::Unconfirmed(operation)) => (None, Some(operation)),
        Err(error) => (
            Some(format!(
                "{}{}",
                translate_ui(locale, tr::feedback_vpn_stop_callback_failed_prefix()),
                error
            )),
            None,
        ),
    };
    Ok(VpnCommandResult {
        action: VpnCommandAction::Stop,
        profile_name: None,
        request_error,
        request_unconfirmed,
    })
}

pub(super) fn parse_dns_servers_text(value: &str) -> Vec<String> {
    let mut servers = Vec::new();
    for item in value.split(|character: char| {
        character == ',' || character == ';' || character.is_ascii_whitespace()
    }) {
        let item = item.trim();
        if item.is_empty() || servers.iter().any(|server| server == item) {
            continue;
        }
        servers.push(item.to_owned());
    }
    servers
}

pub(super) fn parse_dns_policy_text(
    value: &str,
    locale: UiLocale,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut policy = BTreeMap::new();
    for line in value.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((matcher, servers)) = line.split_once('=') else {
            return Err(translate_ui(locale, tr::feedback_dns_policy_format_error()));
        };
        let matcher = matcher.trim();
        if matcher.is_empty() {
            return Err(translate_ui(
                locale,
                tr::feedback_dns_policy_matcher_required(),
            ));
        }
        let servers = parse_dns_servers_text(servers);
        if servers.is_empty() {
            return Err(format!(
                "{}{}{}",
                translate_ui(locale, tr::feedback_dns_policy_upstream_missing_prefix()),
                matcher,
                translate_ui(locale, tr::feedback_dns_policy_upstream_missing_suffix())
            ));
        }
        policy.insert(matcher.to_owned(), servers);
    }
    Ok(policy)
}

pub(super) async fn prepare_profile_url_import(
    url: String,
    name: Option<String>,
) -> Result<ProfileImportPreparation, String> {
    let core = paws_core::shared_core();
    let expected_config_revision = core
        .config_projection()
        .map_err(|error| error.to_string())?
        .revisions
        .config_revision;
    let prepared = core
        .prepare_profile_import_from_url(&url, name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProfileImportPreparation::Ready(
        PreparedProfileMutation::Import {
            prepared,
            expected_config_revision,
        },
    ))
}

pub(super) async fn prepare_scanned_profile_import(
    name: String,
    locale: UiLocale,
) -> Result<ProfileImportPreparation, String> {
    let core = paws_core::shared_core();
    let initial_config = core
        .config_projection()
        .map_err(|error| error.to_string())?;
    let expected_config_revision = initial_config.revisions.config_revision;
    let payload = crate::bridge::scan_subscription_code()
        .await
        .map_err(|error| {
            format!(
                "{}{}",
                translate_ui(locale, tr::profiles_scan_failed_prefix()),
                error
            )
        })?;
    let scanned = match parse_scanned_subscription(&payload) {
        Ok(scanned) => scanned,
        Err(ScannedSubscriptionError::Empty) => {
            return Ok(ProfileImportPreparation::Cancelled);
        }
        Err(ScannedSubscriptionError::Unsupported) => {
            return Err(translate_ui(locale, tr::profiles_scan_invalid()));
        }
    };
    let name = match name.trim() {
        "" => scanned.name,
        value => Some(value.to_owned()),
    };
    let existing = initial_config
        .profiles
        .into_iter()
        .find(|profile| profile.subscription_url.as_deref() == Some(scanned.url.as_str()));
    if let Some(profile) = existing {
        let prepared = core
            .prepare_profile_refresh_from_url(&scanned.url)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(ProfileImportPreparation::Ready(
            PreparedProfileMutation::Refresh {
                profile_id: profile.id,
                name,
                subscription_url: scanned.url,
                prepared,
                expected_config_revision,
            },
        ));
    }
    let prepared = core
        .prepare_profile_import_from_url(&scanned.url, name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProfileImportPreparation::Ready(
        PreparedProfileMutation::Import {
            prepared,
            expected_config_revision,
        },
    ))
}

pub(super) async fn prepare_local_profile_import() -> Result<ProfileImportPreparation, String> {
    let core = paws_core::shared_core();
    let expected_config_revision = core
        .config_projection()
        .map_err(|error| error.to_string())?
        .revisions
        .config_revision;
    let Some((name, raw_yaml)) = crate::bridge::pick_profile_text().await? else {
        return Ok(ProfileImportPreparation::Cancelled);
    };
    let prepared = core
        .prepare_profile_import_from_content(&name, "local-file", &raw_yaml, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProfileImportPreparation::Ready(
        PreparedProfileMutation::Import {
            prepared,
            expected_config_revision,
        },
    ))
}

pub(super) async fn commit_prepared_profile_import(
    prepared: PreparedProfileMutation,
) -> Result<ProfileImportCommitResult, String> {
    let core = paws_core::shared_core();
    let (profile_id, config) = match prepared {
        PreparedProfileMutation::Import {
            prepared,
            expected_config_revision,
        } => {
            let receipt = core
                .commit_prepared_profile_import_and_activate_checked(
                    prepared,
                    expected_config_revision,
                )
                .await
                .map_err(|error| error.to_string())?;
            (receipt.profile_id, receipt.config)
        }
        PreparedProfileMutation::Refresh {
            profile_id,
            name,
            subscription_url,
            prepared,
            expected_config_revision,
        } => {
            let config = core
                .commit_prepared_profile_refresh_and_activate_checked(
                    &profile_id,
                    name,
                    Some(subscription_url),
                    prepared,
                    expected_config_revision,
                )
                .await
                .map_err(|error| error.to_string())?;
            (profile_id, config)
        }
    };
    let profile_name = config
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.name.clone())
        .unwrap_or(profile_id);
    Ok(ProfileImportCommitResult {
        result: ProfileImportResult {
            profile_name,
            restart_requested: false,
            restart_error: None,
            restart_unconfirmed: None,
        },
        config,
    })
}

pub(super) async fn select_proxy(group: String, proxy: String) -> Result<(), String> {
    paws_core::shared_core()
        .select_proxy_via_controller(&group, &proxy)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn unfix_proxy(group: String) -> Result<(), String> {
    paws_core::shared_core()
        .unfix_proxy_via_controller(&group)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn test_proxy_delays(
    groups: Vec<(String, usize)>,
) -> Result<ProxyDelayBatchResult, String> {
    let mut succeeded = 0;
    let mut failed = 0;
    for (group, member_count) in groups {
        match paws_core::shared_core()
            .test_proxy_group_via_controller(&group, None, Some(5000))
            .await
        {
            Ok(delays) => {
                succeeded += delays.values().filter(|delay| **delay > 0).count();
                failed += delays.values().filter(|delay| **delay == 0).count();
            }
            Err(_) => failed += member_count,
        }
    }
    Ok(ProxyDelayBatchResult { succeeded, failed })
}

pub(super) async fn close_connection(connection_id: String) -> Result<(), String> {
    paws_core::shared_core()
        .close_connection_via_controller(&connection_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn apply_manual_rule(
    profile_id: String,
    spec: ManualRuleSpec,
    connection_id: Option<String>,
) -> Result<ManualRuleSaveResult, String> {
    let applied = paws_core::shared_core()
        .apply_manual_rule(&profile_id, &spec)
        .await
        .map_err(|error| error.to_string())?;
    let connection_close_requested = connection_id.is_some();
    let connection_close_error = if let Some(connection_id) = connection_id {
        paws_core::shared_core()
            .close_connection_via_controller(&connection_id)
            .await
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    Ok(ManualRuleSaveResult {
        applied,
        connection_close_requested,
        connection_close_error,
    })
}

pub(super) async fn close_all_connections() -> Result<(), String> {
    paws_core::shared_core()
        .close_all_connections_via_controller()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn clear_request_history() -> Result<(), String> {
    paws_core::shared_core()
        .clear_request_history()
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) async fn set_log_recording(enabled: bool) -> Result<LogRecordingChangeResult, String> {
    let status = paws_core::shared_core()
        .set_log_recording_enabled(enabled)
        .map_err(|error| error.to_string())?;
    Ok(LogRecordingChangeResult { status })
}

pub(super) async fn export_log_archive(file_name: String) -> Result<String, String> {
    let content = paws_core::shared_core()
        .read_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    crate::bridge::export_log(file_name.clone(), content).await?;
    Ok(file_name)
}

pub(super) async fn delete_log_archive(
    file_name: String,
) -> Result<LogArchiveDeleteResult, String> {
    let status = paws_core::shared_core()
        .delete_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    Ok(LogArchiveDeleteResult { file_name, status })
}

pub(super) fn localized_profile_import_message(
    profile_name: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    locale: UiLocale,
) -> String {
    let base = format!(
        "{}{}{}",
        translate_ui(locale, tr::profiles_import_toast_prefix()),
        profile_name,
        translate_ui(locale, tr::profiles_import_toast_success_suffix())
    );
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}",
            translate_ui(locale, tr::profiles_import_toast_restart_failed_suffix()),
            error
        )
    } else if restart_requested {
        format!(
            "{base}{}",
            translate_ui(locale, tr::profiles_import_toast_restart_suffix())
        )
    } else {
        base
    }
}

pub(super) fn profile_delete_vpn_action_label(
    action: ProfileDeleteVpnAction,
    locale: UiLocale,
) -> String {
    match action {
        ProfileDeleteVpnAction::Stop => translate_ui(locale, tr::feedback_vpn_action_stop()),
        ProfileDeleteVpnAction::Restart => translate_ui(locale, tr::feedback_vpn_action_restart()),
    }
}

pub(super) fn manual_rule_saved_message(result: &ManualRuleSaveResult, locale: UiLocale) -> String {
    let mut parts = vec![
        match result.applied.mutation.kind {
            ManualRuleMutationKind::Added => translate_ui(locale, tr::manual_rule_added()),
            ManualRuleMutationKind::Updated => translate_ui(locale, tr::manual_rule_updated()),
            ManualRuleMutationKind::Reenabled => translate_ui(locale, tr::manual_rule_reenabled()),
            ManualRuleMutationKind::Unchanged => translate_ui(locale, tr::manual_rule_unchanged()),
        },
        result.applied.mutation.line.clone(),
    ];
    if result.applied.live_updated {
        parts.push(translate_ui(locale, tr::manual_rule_live_updated()));
    } else {
        parts.push(translate_ui(locale, tr::manual_rule_next_start()));
    }
    if !result.applied.rule_mode_active {
        parts.push(translate_ui(locale, tr::manual_rule_mode_inactive()));
    }
    if let Some(error) = &result.connection_close_error {
        parts.push(translate_ui(
            locale,
            tr::manual_rule_close_failed(error.clone()),
        ));
    } else if result.connection_close_requested {
        parts.push(translate_ui(locale, tr::manual_rule_connection_closed()));
    }
    parts.join(" · ")
}
