use super::*;

pub(super) async fn load_snapshot() -> RuntimeSnapshot {
    let core = hmeta_core::shared_core();
    let _ = core.sync_external_controller_config().await;
    core.snapshot().unwrap_or_default()
}

pub(super) async fn delayed_snapshot() -> RuntimeSnapshot {
    // A pending platform start transaction owns the ashmem notification
    // waiter. Regular UI refreshes use a bounded timer and synchronize the
    // latest frame while loading the snapshot, avoiding competing reads from
    // the single notification socket.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    load_snapshot().await
}

pub(super) async fn delayed_vpn_snapshot() -> RuntimeSnapshot {
    tokio::time::sleep(Duration::from_millis(200)).await;
    load_snapshot().await
}

pub(super) async fn bootstrap_active_profile() -> RuntimeSnapshot {
    let core = hmeta_core::shared_core();
    // State::new restores a revision-checked proxy-group cache synchronously,
    // so the dashboard can render immediately. Parse the complete meow config
    // only after the first frame, then replace the cache-backed snapshot.
    let _ = core.prepare_active_vpn().await;
    let refresh_core = core.clone();
    tokio::spawn(async move {
        let _ = refresh_core.refresh_due_profiles().await;
    });
    load_snapshot().await
}

pub(super) async fn lookup_rule(query: String) -> Result<hmeta_core::RuleLookupResult, String> {
    hmeta_core::shared_core()
        .lookup_rule(&query)
        .await
        .map_err(|error| error.to_string())
}

pub(super) fn reconcile_vpn_command(state: &mut State) {
    state.vpn_command_pending = state.vpn_command_pending.filter(|action| {
        vpn_command_is_pending(
            *action,
            state.snapshot.vpn_lifecycle,
            state.snapshot.vpn_running,
        )
    });
}

pub(super) fn count_failed_refreshed_profiles(
    snapshot: &RuntimeSnapshot,
    attempted_profile_ids: &[String],
) -> usize {
    snapshot
        .profiles
        .iter()
        .filter(|profile| {
            attempted_profile_ids.iter().any(|id| id == &profile.id)
                && profile.last_refresh_error.is_some()
        })
        .count()
}

pub(super) fn count_failed_refreshed_providers(
    snapshot: &RuntimeSnapshot,
    attempted_providers: &[(String, String)],
) -> usize {
    snapshot
        .providers
        .iter()
        .filter(|provider| {
            attempted_providers.iter().any(|(provider_type, name)| {
                provider_type == &provider.provider_type && name == &provider.name
            }) && provider.last_refresh_error.is_some()
        })
        .count()
}

pub(super) async fn start_vpn_command_and_snapshot(
    profile_id: String,
    profile_name: String,
    ui_strings: &'static UiStrings,
) -> Result<VpnCommandResult, String> {
    let core = hmeta_core::shared_core();
    let active_profile = core
        .snapshot()
        .map_err(|error| error.to_string())?
        .active_profile;
    if active_profile.as_deref() != Some(profile_id.as_str()) {
        core.activate_profile(&profile_id).await.map_err(|error| {
            format!(
                "{}{}{}{}",
                ui_strings.feedback_vpn_start_profile_load_failed_prefix,
                profile_name,
                ui_strings.feedback_vpn_start_profile_load_failed_mid,
                error
            )
        })?;
    }
    let options_json = core.active_vpn_options_json().map_err(|error| {
        format!(
            "{}{}",
            ui_strings.feedback_vpn_start_options_failed_prefix, error
        )
    })?;
    let request_error = crate::platform_callbacks::request_start_vpn(options_json)
        .await
        .err()
        .map(|error| error.to_string());
    Ok(VpnCommandResult {
        snapshot: load_snapshot().await,
        action: VpnCommandAction::Start,
        profile_name: Some(profile_name),
        request_error,
    })
}

pub(super) async fn stop_vpn_command_and_snapshot(
    ui_strings: &'static UiStrings,
) -> Result<VpnCommandResult, String> {
    let request_error = match crate::platform_callbacks::request_stop_vpn().await {
        Ok(()) => None,
        Err(error) => {
            hmeta_core::shared_core().stop_vpn().map_err(|fallback| {
                format!(
                    "{}{}{}{}",
                    ui_strings.feedback_vpn_stop_callback_failed_prefix,
                    error,
                    ui_strings.feedback_vpn_stop_fallback_failed_mid,
                    fallback
                )
            })?;
            Some(error.to_string())
        }
    };
    Ok(VpnCommandResult {
        snapshot: load_snapshot().await,
        action: VpnCommandAction::Stop,
        profile_name: None,
        request_error,
    })
}

pub(super) async fn request_vpn_restart_if_running(
    was_vpn_running: bool,
    ui_strings: &UiStrings,
) -> Option<String> {
    if !was_vpn_running {
        return None;
    }

    let mut errors = Vec::new();
    if let Err(error) = crate::platform_callbacks::request_stop_vpn().await {
        match hmeta_core::shared_core().stop_vpn() {
            Ok(()) => {
                errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_stop_fallback_applied_prefix, error
                ));
                return Some(errors.join("；"));
            }
            Err(fallback) => {
                errors.push(format!(
                    "{}{}{}{}",
                    ui_strings.feedback_vpn_stop_failed_prefix,
                    error,
                    ui_strings.feedback_vpn_stop_fallback_failed_suffix,
                    fallback
                ));
                return Some(errors.join("；"));
            }
        }
    }

    match hmeta_core::shared_core().active_vpn_options_json() {
        Ok(options_json) => {
            if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json).await {
                errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_start_callback_failed_prefix, error
                ));
            }
        }
        Err(error) => errors.push(format!(
            "{}{}",
            ui_strings.feedback_vpn_options_failed_prefix, error
        )),
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors.join("；"))
    }
}

pub(super) fn dns_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (String, String, String) {
    (
        snapshot.vpn_options.dns_servers.join(", "),
        snapshot.vpn_options.dns_fallbacks.join(", "),
        dns_policy_text(&snapshot.vpn_options.dns_nameserver_policy),
    )
}

pub(super) fn vpn_draft_from_snapshot(snapshot: &RuntimeSnapshot) -> (bool, bool, bool, String) {
    let stack = hmeta_model::VpnStack::try_from(snapshot.vpn_options.stack.as_str())
        .unwrap_or_default()
        .as_str()
        .to_owned();
    (
        snapshot.vpn_options.system_proxy,
        snapshot.vpn_options.dns_hijacking,
        snapshot.vpn_options.allow_bypass,
        stack,
    )
}

pub(super) fn dns_policy_text(policy: &BTreeMap<String, Vec<String>>) -> String {
    policy
        .iter()
        .map(|(matcher, servers)| format!("{matcher} = {}", servers.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
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
    ui_strings: &UiStrings,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut policy = BTreeMap::new();
    for line in value.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((matcher, servers)) = line.split_once('=') else {
            return Err(ui_strings.feedback_dns_policy_format_error.to_owned());
        };
        let matcher = matcher.trim();
        if matcher.is_empty() {
            return Err(ui_strings.feedback_dns_policy_matcher_required.to_owned());
        }
        let servers = parse_dns_servers_text(servers);
        if servers.is_empty() {
            return Err(format!(
                "{}{}{}",
                ui_strings.feedback_dns_policy_upstream_missing_prefix,
                matcher,
                ui_strings.feedback_dns_policy_upstream_missing_suffix
            ));
        }
        policy.insert(matcher.to_owned(), servers);
    }
    Ok(policy)
}

pub(super) async fn import_profile_url_and_snapshot(
    url: String,
    name: Option<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let id = hmeta_core::shared_core()
        .import_profile_from_url(&url, name)
        .await
        .map_err(|error| error.to_string())?;
    hmeta_core::shared_core()
        .activate_profile(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(profile_import_result(id, was_vpn_running, ui_strings).await)
}

pub(super) async fn scan_profile_subscription_and_snapshot(
    name: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let payload = crate::platform_callbacks::scan_subscription_code()
        .await
        .map_err(|error| format!("{}{}", ui_strings.profiles_scan_failed_prefix, error))?;
    let scanned = match parse_scanned_subscription(&payload) {
        Ok(scanned) => scanned,
        Err(ScannedSubscriptionError::Empty) => {
            return Err("profile scan cancelled".to_owned());
        }
        Err(ScannedSubscriptionError::Unsupported) => {
            return Err(ui_strings.profiles_scan_invalid.to_owned());
        }
    };
    let name = match name.trim() {
        "" => scanned.name,
        value => Some(value.to_owned()),
    };
    let core = hmeta_core::shared_core();
    let existing = core.snapshot().ok().and_then(|snapshot| {
        snapshot
            .profiles
            .into_iter()
            .find(|profile| profile.subscription_url.as_deref() == Some(scanned.url.as_str()))
    });
    if let Some(profile) = existing {
        if let Some(name) = name {
            core.update_profile_subscription(&profile.id, &name, &scanned.url)
                .map_err(|error| error.to_string())?;
        }
        core.refresh_profile(&profile.id)
            .await
            .map_err(|error| error.to_string())?;
        core.activate_profile(&profile.id)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(profile_import_result(profile.id, was_vpn_running, ui_strings).await);
    }
    import_profile_url_and_snapshot(scanned.url, name, was_vpn_running, ui_strings).await
}

pub(super) async fn import_profile_file_and_snapshot(
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileImportResult, String> {
    let (name, raw_yaml) = crate::platform_callbacks::pick_profile_text().await?;
    let id = hmeta_core::shared_core()
        .import_profile_from_content(&name, "local-file", &raw_yaml, None)
        .await
        .map_err(|error| error.to_string())?;
    hmeta_core::shared_core()
        .activate_profile(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(profile_import_result(id, was_vpn_running, ui_strings).await)
}

pub(super) async fn profile_import_result(
    profile_id: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> ProfileImportResult {
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
    let snapshot = load_snapshot().await;
    let profile_name = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.name.clone())
        .unwrap_or(profile_id);
    ProfileImportResult {
        snapshot,
        profile_name,
        restart_requested: was_vpn_running,
        restart_error,
    }
}

pub(super) async fn import_rules_and_snapshot(
    active_profile: Option<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleImportResult, String> {
    let profile_id =
        active_profile.ok_or_else(|| ui_strings.feedback_active_profile_required.to_owned())?;
    let (name, rules_text) = crate::platform_callbacks::pick_profile_text().await?;
    let source = format!("rules:{name}");
    let imported_rule_ids = hmeta_core::shared_core()
        .import_rules_from_content(Some(&profile_id), &source, &rules_text)
        .map_err(|error| error.to_string())?;
    let reload_error = hmeta_core::shared_core()
        .activate_profile(&profile_id)
        .await
        .err()
        .map(|error| error.to_string());
    let restart_error = if reload_error.is_none() {
        request_vpn_restart_if_running(was_vpn_running, ui_strings).await
    } else {
        None
    };
    Ok(RuleImportResult {
        snapshot: load_snapshot().await,
        imported_count: imported_rule_ids.len(),
        reload_error,
        restart_requested: was_vpn_running,
        restart_error,
    })
}

pub(super) fn picker_was_cancelled(error: &str) -> bool {
    error.to_ascii_lowercase().contains("cancel")
        || error.contains("取消")
        || error.contains("已取消")
}

pub(super) async fn delete_profile_and_snapshot(
    profile_id: String,
    profile_name: String,
    was_active: bool,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<ProfileDeleteResult, String> {
    let mut vpn_errors = Vec::new();
    if was_active && was_vpn_running {
        if let Err(error) = crate::platform_callbacks::request_stop_vpn().await {
            match hmeta_core::shared_core().stop_vpn() {
                Ok(()) => vpn_errors.push(format!(
                    "{}{}",
                    ui_strings.feedback_vpn_stop_fallback_applied_prefix, error
                )),
                Err(fallback) => {
                    return Err(format!(
                        "{}{}{}{}",
                        ui_strings.feedback_vpn_stop_failed_prefix,
                        error,
                        ui_strings.feedback_vpn_stop_fallback_failed_suffix,
                        fallback
                    ));
                }
            }
        }
    }
    hmeta_core::shared_core()
        .delete_profile(&profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = load_snapshot().await;
    let mut vpn_action = None;
    if was_active && was_vpn_running && snapshot.active_profile.is_some() {
        vpn_action = Some(ProfileDeleteVpnAction::Restart);
        let options_json = hmeta_core::shared_core()
            .active_vpn_options_json()
            .map_err(|error| error.to_string())?;
        if let Err(error) = crate::platform_callbacks::request_start_vpn(options_json).await {
            vpn_errors.push(format!(
                "{}{}",
                ui_strings.feedback_vpn_start_callback_failed_prefix, error
            ));
        }
    } else if was_active && was_vpn_running {
        vpn_action = Some(ProfileDeleteVpnAction::Stop);
    }
    Ok(ProfileDeleteResult {
        snapshot: load_snapshot().await,
        profile_name,
        vpn_action,
        vpn_error: (!vpn_errors.is_empty()).then(|| vpn_errors.join("；")),
    })
}

pub(super) async fn select_proxy_and_snapshot(
    group: String,
    proxy: String,
) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .select_proxy_via_controller(&group, &proxy)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

pub(super) async fn unfix_proxy_and_snapshot(group: String) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .unfix_proxy_via_controller(&group)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

pub(super) async fn test_proxy_delays_and_snapshot(
    groups: Vec<(String, usize)>,
) -> Result<ProxyDelayBatchResult, String> {
    let mut succeeded = 0;
    let mut failed = 0;
    for (group, member_count) in groups {
        match hmeta_core::shared_core()
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
    Ok(ProxyDelayBatchResult {
        snapshot: load_snapshot().await,
        succeeded,
        failed,
    })
}

pub(super) fn proxy_groups_for_delay_test(snapshot: &RuntimeSnapshot) -> Vec<(String, usize)> {
    snapshot
        .proxy_groups
        .iter()
        .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL") && !group.proxies.is_empty())
        .map(|group| (group.name.clone(), group.proxies.len()))
        .collect()
}

pub(super) async fn close_connection_and_snapshot(
    connection_id: String,
) -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .close_connection_via_controller(&connection_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

pub(super) async fn apply_manual_rule_and_snapshot(
    profile_id: String,
    spec: ManualRuleSpec,
    connection_id: Option<String>,
) -> Result<ManualRuleSaveResult, String> {
    let applied = hmeta_core::shared_core()
        .apply_manual_rule(&profile_id, &spec)
        .await
        .map_err(|error| error.to_string())?;
    let connection_close_requested = connection_id.is_some();
    let connection_close_error = if let Some(connection_id) = connection_id {
        hmeta_core::shared_core()
            .close_connection_via_controller(&connection_id)
            .await
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    Ok(ManualRuleSaveResult {
        snapshot: load_snapshot().await,
        applied,
        connection_close_requested,
        connection_close_error,
    })
}

pub(super) async fn close_all_connections_and_snapshot() -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .close_all_connections_via_controller()
        .await
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

pub(super) async fn clear_request_history_and_snapshot() -> Result<RuntimeSnapshot, String> {
    hmeta_core::shared_core()
        .clear_request_history()
        .map_err(|error| error.to_string())?;
    Ok(load_snapshot().await)
}

pub(super) fn refresh_log_recording_status(state: &mut State) {
    if let Ok(status) = hmeta_core::shared_core().log_recording_status() {
        state.log_recording = status;
    }
}

pub(super) fn locale_text(locale: UiLocale, zh: &'static str, en: &'static str) -> &'static str {
    if locale == UiLocale::ZhCn {
        zh
    } else {
        en
    }
}

pub(super) async fn set_log_recording_and_snapshot(
    enabled: bool,
) -> Result<LogRecordingChangeResult, String> {
    let status = hmeta_core::shared_core()
        .set_log_recording_enabled(enabled)
        .map_err(|error| error.to_string())?;
    Ok(LogRecordingChangeResult {
        snapshot: load_snapshot().await,
        status,
    })
}

pub(super) async fn export_log_archive(file_name: String) -> Result<String, String> {
    let content = hmeta_core::shared_core()
        .read_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    crate::platform_callbacks::export_log(file_name.clone(), content).await?;
    Ok(file_name)
}

pub(super) async fn delete_log_archive(
    file_name: String,
) -> Result<LogArchiveDeleteResult, String> {
    let status = hmeta_core::shared_core()
        .delete_log_archive(&file_name)
        .map_err(|error| error.to_string())?;
    Ok(LogArchiveDeleteResult { file_name, status })
}

pub(super) async fn set_rule_enabled_and_snapshot(
    profile_id: String,
    rule_id: String,
    enabled: bool,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .set_rule_enabled(&profile_id, &rule_id, enabled)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

pub(super) async fn delete_rule_and_snapshot(
    profile_id: String,
    rule_id: String,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .delete_rule(&rule_id)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

pub(super) async fn reorder_rules_and_snapshot(
    profile_id: String,
    ordered_rule_ids: Vec<String>,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .reorder_rules(&profile_id, &ordered_rule_ids)
        .map_err(|error| error.to_string())?;
    reload_profile_after_rule_change(&profile_id, was_vpn_running, ui_strings).await
}

pub(super) async fn reload_profile_after_rule_change(
    profile_id: &str,
    was_vpn_running: bool,
    ui_strings: &'static UiStrings,
) -> Result<RuleChangeResult, String> {
    hmeta_core::shared_core()
        .activate_profile(profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let restart_error = request_vpn_restart_if_running(was_vpn_running, ui_strings).await;
    Ok(RuleChangeResult {
        snapshot: load_snapshot().await,
        restart_requested: was_vpn_running,
        restart_error,
    })
}

pub(super) fn localized_profile_import_message(
    profile_name: &str,
    restart_requested: bool,
    restart_error: Option<&str>,
    strings: &UiStrings,
) -> String {
    let base = format!(
        "{}{}{}",
        strings.profiles_import_toast_prefix,
        profile_name,
        strings.profiles_import_toast_success_suffix
    );
    if let Some(error) = restart_error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{base}{}{}",
            strings.profiles_import_toast_restart_failed_suffix, error
        )
    } else if restart_requested {
        format!("{base}{}", strings.profiles_import_toast_restart_suffix)
    } else {
        base
    }
}

pub(super) fn localized_profile_backup_restore_message(
    profile_name: &str,
    error: Option<&str>,
    strings: &UiStrings,
) -> String {
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
        format!(
            "{}{}{}{}",
            strings.feedback_profile_prefix,
            profile_name,
            strings.profiles_backup_restore_failed_suffix,
            error
        )
    } else {
        format!(
            "{}{}{}",
            strings.feedback_profile_prefix,
            profile_name,
            strings.profiles_backup_restore_success_suffix
        )
    }
}

pub(super) fn profile_delete_vpn_action_label(
    action: ProfileDeleteVpnAction,
    strings: &UiStrings,
) -> &'static str {
    match action {
        ProfileDeleteVpnAction::Stop => strings.feedback_vpn_action_stop,
        ProfileDeleteVpnAction::Restart => strings.feedback_vpn_action_restart,
    }
}

pub(super) fn manual_rule_saved_message(result: &ManualRuleSaveResult, locale: UiLocale) -> String {
    let mut parts = vec![
        match (locale, result.applied.mutation.kind) {
            (UiLocale::ZhCn, ManualRuleMutationKind::Added) => "手动规则已添加".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Updated) => "冲突规则已更新".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Reenabled) => "手动规则已重新启用".to_owned(),
            (UiLocale::ZhCn, ManualRuleMutationKind::Unchanged) => "相同规则已存在".to_owned(),
            (_, ManualRuleMutationKind::Added) => "Manual rule added".to_owned(),
            (_, ManualRuleMutationKind::Updated) => "Conflicting rule updated".to_owned(),
            (_, ManualRuleMutationKind::Reenabled) => "Manual rule re-enabled".to_owned(),
            (_, ManualRuleMutationKind::Unchanged) => "The same rule already exists".to_owned(),
        },
        result.applied.mutation.line.clone(),
    ];
    if result.applied.live_updated {
        parts.push(if locale == UiLocale::ZhCn {
            "运行时规则已热更新".to_owned()
        } else {
            "Runtime rules updated live".to_owned()
        });
    } else {
        parts.push(if locale == UiLocale::ZhCn {
            "将在 VPN 下次启动时生效".to_owned()
        } else {
            "Takes effect on the next VPN start".to_owned()
        });
    }
    if !result.applied.rule_mode_active {
        parts.push(if locale == UiLocale::ZhCn {
            "当前不是规则模式，切换到规则模式后才会命中".to_owned()
        } else {
            "Rule mode is not active; switch to Rule mode for matching".to_owned()
        });
    }
    if let Some(error) = &result.connection_close_error {
        parts.push(if locale == UiLocale::ZhCn {
            format!("规则已保存，但断开当前连接失败：{error}")
        } else {
            format!("Rule saved, but the current connection could not be closed: {error}")
        });
    } else if result.connection_close_requested {
        parts.push(if locale == UiLocale::ZhCn {
            "当前连接已断开，新连接将使用新规则".to_owned()
        } else {
            "Current connection closed; the new rule applies when it reconnects".to_owned()
        });
    }
    parts.join(" · ")
}
