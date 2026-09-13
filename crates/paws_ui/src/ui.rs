use crate::activity_filter::{
    matches_connection_query, matches_request_filter, request_connection_query, RequestStatusFilter,
};
use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;
use crate::log_filter::{matches_log_filter_normalized, normalize_log_query, LogLevelFilter};
use crate::mode_feedback::mode_changed_message;
use crate::notification::NotificationCenter;
use crate::profile_filter::matches_profile_query;
use crate::profile_refresh_feedback::{
    profile_activation_message, profile_backup_restore_message, profile_batch_refresh_message,
    profile_delete_message,
};
use crate::provider_refresh_feedback::provider_batch_refresh_message;
use crate::proxy_grid::{
    effective_group_leaf, grouped_proxy_rows, primary_selected_group_leaf, proxy_group_summary,
    ProxyGroupHeaderRow, ProxyGroupMemberRow, ProxyGroupRow,
};
use crate::resource_filter::{matches_geodata_query, matches_provider_query, matches_rule_query};
use crate::route_status::latest_active_rule_node;
use crate::rule_feedback::rule_import_message;
use crate::settings_feedback::settings_saved_message;
use crate::subscription_scan::{parse_scanned_subscription, ScannedSubscriptionError};
use crate::time_format;
use crate::traffic_history::summarize_traffic_history;
use crate::ui_preferences::{LanguagePreference, ThemePreference, UiPreferences};
use crate::vpn_feedback::vpn_command_message;
use crate::vpn_operation::{
    VpnCommandAction, VpnOperationFailureDisposition, VpnOperationPhase, VpnOperationState,
};
use crate::yaml_summary::summarize_yaml_edit;
use arkit::prelude::ReadableExt;
use paws_model::{
    ManualRuleMatchKind, ManualRuleMutationKind, ManualRuleSpec, RuntimeMode, TrafficHistoryPoint,
    VpnLifecycle,
};
use std::collections::BTreeMap;
use std::future::Future;

#[path = "ui_store.rs"]
mod store;
pub(crate) use store::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleLookupState {
    id: u64,
    query: String,
    submitting: bool,
    result: Option<paws_core::RuleLookupResult>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualRuleEditorState {
    connection_id: Option<String>,
    domain: String,
    destination_ip: String,
    match_kind: ManualRuleMatchKind,
    value: String,
    target: String,
    disconnect_after_save: bool,
    submitting: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManualRuleSaveResult {
    applied: paws_core::ManualRuleApplyResult,
    connection_close_requested: bool,
    connection_close_error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct UiServices {
    runtime: arkit::RuntimeHandle,
    stores: UiStores,
    operations: UiOperationStores,
    notifications: NotificationCenter,
    next_request_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    mode_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    profile_import_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    profile_import_cancel:
        std::sync::Arc<std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
}

impl UiServices {
    fn new(
        runtime: arkit::RuntimeHandle,
        stores: UiStores,
        operations: UiOperationStores,
        notifications: NotificationCenter,
    ) -> Self {
        Self {
            runtime,
            stores,
            operations,
            notifications,
            next_request_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            mode_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            profile_import_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            profile_import_cancel: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub(crate) fn set_language(&self, language: LanguagePreference) {
        let current = self.stores.preferences.peek().clone();
        let preferences = UiPreferences {
            language,
            theme: current.theme,
        };
        let system = crate::system_preferences::current();
        let locale = language.resolve(&system.locale);
        let save = preferences.save();
        let preferences_error = save.as_ref().err().cloned();
        self.stores.set_preferences(PreferencesProjection {
            locale,
            language,
            dark: current.theme.resolve_dark(system.color_mode),
            color_mode_error: None,
            preferences_error,
            ..current
        });
        let message = match save {
            Ok(()) if locale == UiLocale::ZhCn => {
                translate_ui(locale, tr::hard_zh_051()).to_owned()
            }
            Ok(()) => "Language preference updated".to_owned(),
            Err(error) if locale == UiLocale::ZhCn => translate_ui(locale, tr::hard_zh_047(error)),
            Err(error) => format!("Failed to save language preference: {error}"),
        };
        self.notifications.publish(message);
    }

    pub(crate) fn set_theme(&self, theme: ThemePreference) {
        let current = self.stores.preferences.peek().clone();
        let preferences = UiPreferences {
            language: current.language,
            theme,
        };
        let system = crate::system_preferences::current();
        let save = preferences.save();
        let preferences_error = save.as_ref().err().cloned();
        self.stores.set_preferences(PreferencesProjection {
            locale: current.language.resolve(&system.locale),
            theme,
            dark: theme.resolve_dark(system.color_mode),
            color_mode_error: None,
            preferences_error,
            ..current
        });
        let message = match save {
            Ok(()) if current.locale == UiLocale::ZhCn => {
                translate_ui(current.locale, tr::hard_zh_052()).to_owned()
            }
            Ok(()) => "Theme preference updated".to_owned(),
            Err(error) if current.locale == UiLocale::ZhCn => {
                translate_ui(current.locale, tr::hard_zh_048(error))
            }
            Err(error) => format!("Failed to save theme preference: {error}"),
        };
        self.notifications.publish(message);
    }

    pub(crate) fn retry_color_mode(&self) {
        let current = self.stores.preferences.peek().clone();
        self.stores.set_preferences(PreferencesProjection {
            color_mode_error: None,
            ..current
        });
    }

    pub(crate) fn open_external_url(&self, url: String) {
        let locale = self.stores.preferences.peek().locale;
        let notifications = self.notifications;
        let runtime = self.runtime.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = crate::bridge::open_external_url(url).await;
            if let Err(error) = result {
                runtime.queue_ui(move || {
                    notifications.publish(format!(
                        "{}{}",
                        translate_ui(locale, tr::feedback_open_link_failed_prefix()),
                        error
                    ));
                });
            }
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyDelayBatchResult {
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct VpnCommandResult {
    action: VpnCommandAction,
    profile_name: Option<String>,
    request_error: Option<String>,
    request_unconfirmed: Option<crate::bridge::VpnUnconfirmedOperation>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileImportResult {
    profile_name: String,
    restart_requested: bool,
    restart_error: Option<String>,
    restart_unconfirmed: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProfileDeleteVpnAction {
    Stop,
    Restart,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileDeleteResult {
    vpn_action: Option<ProfileDeleteVpnAction>,
    vpn_followup: Result<crate::bridge::VpnOperationOutcome<bool>, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LogRecordingChangeResult {
    status: paws_core::LogRecordingStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct LogArchiveDeleteResult {
    file_name: String,
    status: paws_core::LogRecordingStatus,
}

#[path = "ui/tasks.rs"]
mod tasks;
use tasks::*;

#[path = "ui/operations.rs"]
mod operations;

#[path = "view.rs"]
mod view;

pub(crate) use view::App;
