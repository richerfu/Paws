use crate::AppError;
use paws_core::CommandContext;
use paws_model::{
    ControllerAccessConfig, ManualRuleSpec, NetworkPortConfig, RuntimeMode, VpnOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRequest {
    pub id: u64,
    pub command: String,
    pub payload: Value,
}

impl AppRequest {
    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if self.id > 9_007_199_254_740_991 || !self.payload.is_object() {
            return Err(AppError::InvalidRequest(
                "request requires a safe integer id and an object payload".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct AppResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<PlatformEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformEffect {
    pub kind: String,
    pub owner: String,
    pub config_revision: String,
    pub options_json: String,
}

impl PlatformEffect {
    pub(crate) fn user_stop() -> Self {
        // Explicit Stop claims the current platform owner at dispatch time.
        // A concurrent configuration commit must not cancel the user's Stop;
        // the host's intent generation still protects a later explicit Start.
        Self {
            kind: "stop".into(),
            owner: String::new(),
            config_revision: String::new(),
            options_json: String::new(),
        }
    }

    pub(crate) fn for_config_change(
        before: &CommandContext,
        after: &CommandContext,
    ) -> Result<Option<Self>, AppError> {
        if before.config_revision == after.config_revision {
            return Ok(None);
        }
        // A cold-start refresh can commit after the first VPN starts. Bind that
        // new owner to the committed revision so it also receives the update.
        // If an owner already existed when the command began, retain it: a late
        // result must never acquire authority over a replacement session.
        let Some(owner) = before
            .vpn_session_id
            .as_ref()
            .filter(|owner| !owner.is_empty())
            .or_else(|| {
                after
                    .vpn_session_id
                    .as_ref()
                    .filter(|owner| !owner.is_empty())
            })
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            kind: if after.active_profile.is_some() {
                "restart"
            } else {
                "stop"
            }
            .into(),
            owner: owner.clone(),
            config_revision: after.config_revision.to_string(),
            options_json: serde_json::to_string(&after.vpn_options)?,
        }))
    }
}

pub(crate) fn requires_reconfigure(
    command: &str,
    payload: &Value,
    active_profile: Option<&str>,
) -> bool {
    match command {
        "profiles.delete" | "profiles.refresh" | "profiles.restore" | "profiles.save"
        | "settings.vpn" | "settings.dns" | "settings.network" => {
            active_profile.is_some()
                && payload.get("profileId").and_then(Value::as_str) == active_profile
        }
        "bootstrap"
        | "mode.set"
        | "diagnostics.serviceDns"
        | "profiles.importUrl"
        | "profiles.importContent"
        | "profiles.activate"
        | "profiles.refreshAll"
        | "rules.import"
        | "rules.add"
        | "rules.enabled"
        | "rules.delete"
        | "rules.reorder" => true,
        // Portable restores add inactive copies; they do not replace the
        // selected profile or change the running engine's configuration.
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Revision {
    Text(String),
    Number(u64),
}
impl Revision {
    pub(crate) fn get(&self) -> Result<u64, AppError> {
        match self {
            Self::Text(s) => s
                .parse()
                .map_err(|_| AppError::InvalidRequest("invalid revision".into())),
            Self::Number(n) if *n <= 9_007_199_254_740_991 => Ok(*n),
            Self::Number(_) => Err(AppError::InvalidRequest(
                "large revisions must be decimal strings".into(),
            )),
        }
    }
}

macro_rules! fields {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name { $(pub $field: $ty),* }
    };
}
fields!(Mode { mode: RuntimeMode });
fields!(EngineDecision {
    options: VpnOptions
});
fields!(ServiceDns {
    domains: String,
    group: String,
    expected_revision: Revision
});
fields!(ImportUrl { name: Option<String>, url: String, intent_id: u64 });
fields!(ImportContent {
    name: String,
    content: String,
    intent_id: u64
});
fields!(CancelImport { intent_id: u64 });
fields!(Profile { profile_id: String });
fields!(SaveProfile {
    profile_id: String,
    content: String,
    expected_revision: Revision
});
fields!(SelectProxy {
    group: String,
    name: String
});
fields!(Group { group: String });
fields!(Name { name: String });
fields!(Identifier { id: String });
fields!(Enabled { enabled: bool });
fields!(Archive { file_name: String });
fields!(Query { query: String });
fields!(RuleImport {
    content: String,
    expected_revision: Revision
});
fields!(RuleEnabled {
    id: String,
    enabled: bool,
    expected_revision: Revision
});
fields!(RuleDelete {
    id: String,
    expected_revision: Revision
});
fields!(ReorderRules { ids: Vec<String>, expected_revision: Revision });
fields!(Provider {
    name: String,
    provider_type: String
});
fields!(VpnSettings {
    profile_id: String,
    options: VpnOptions,
    expected_revision: Revision
});
fields!(DnsSettings { profile_id: String, servers: Vec<String>, fallbacks: Vec<String>, policy: BTreeMap<String,Vec<String>>, expected_revision: Revision });
fields!(NetworkSettings {
    profile_id: String,
    ports: NetworkPortConfig,
    controller: ControllerAccessConfig,
    expected_revision: Revision
});
fields!(Content { content: String });
fields!(Webdav {
    url: String,
    username: String,
    password: String
});
fields!(Convert { input: String });
fields!(Scan { payload: String });
fields!(ProfileSubscription {
    profile_id: String,
    name: String,
    url: String,
    expected_revision: Revision
});
fields!(ProviderProxy {
    provider: String,
    name: String
});

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddRule {
    #[serde(flatten)]
    pub spec: ManualRuleSpec,
    pub expected_revision: Revision,
    pub connection_id: Option<String>,
    #[serde(default)]
    pub disconnect_after_save: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_rejects_lossy_numbers_but_accepts_full_decimal_strings() {
        assert!(Revision::Number(9_007_199_254_740_992).get().is_err());
        assert_eq!(
            Revision::Text(u64::MAX.to_string()).get().unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn success_has_explicit_null_data_and_failure_has_no_effect() {
        let success = AppResponse {
            id: 3,
            ok: true,
            data: Some(Value::Null),
            effect: None,
            error: None,
        };
        let json = serde_json::to_value(success).unwrap();
        assert!(json.get("data").unwrap().is_null());
        assert!(json.get("effect").is_none());
    }

    #[test]
    fn rejects_malformed_command_payloads_before_mutation() {
        assert!(serde_json::from_value::<Mode>(serde_json::json!({"mode":"bogus"})).is_err());
        assert!(serde_json::from_value::<SaveProfile>(
            serde_json::json!({"profileId":"a","content":"x"})
        )
        .is_err());
        let request = AppRequest {
            id: 1,
            command: "profiles.read".into(),
            payload: Value::Null,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn configuration_followup_is_bound_to_previous_owner_and_committed_revision() {
        let before = CommandContext {
            vpn_session_id: Some("old-session".into()),
            config_revision: 4,
            ..CommandContext::default()
        };
        let mut after = before.clone();
        after.config_revision = 5;
        after.vpn_session_id = Some("replacement-session".into());
        after.active_profile = Some("new-profile".into());
        let effect = PlatformEffect::for_config_change(&before, &after)
            .unwrap()
            .unwrap();
        assert_eq!(effect.owner, "old-session");
        assert_eq!(effect.config_revision, "5");
        assert_eq!(effect.kind, "restart");
        after.active_profile = None;
        assert_eq!(
            PlatformEffect::for_config_change(&before, &after)
                .unwrap()
                .unwrap()
                .kind,
            "stop"
        );
        assert!(PlatformEffect::for_config_change(&before, &before)
            .unwrap()
            .is_none());
        after.vpn_session_id = None;
        assert!(
            PlatformEffect::for_config_change(&CommandContext::default(), &after)
                .unwrap()
                .is_none()
        );

        // The explicit user action remains valid when the configuration has
        // advanced; only automatic configuration follow-ups carry that fence.
        let stop = PlatformEffect::user_stop();
        assert_eq!(stop.kind, "stop");
        assert!(stop.owner.is_empty());
        assert!(stop.config_revision.is_empty());
    }

    #[test]
    fn cold_start_refresh_reconfigures_a_session_started_while_it_was_pending() {
        let before = CommandContext {
            config_revision: 7,
            active_profile: Some("subscription".into()),
            ..Default::default()
        };
        let mut after = before.clone();
        after.config_revision = 8;
        after.vpn_session_id = Some("first-session".into());
        let effect = PlatformEffect::for_config_change(&before, &after)
            .unwrap()
            .unwrap();
        assert_eq!(effect.kind, "restart");
        assert_eq!(effect.owner, "first-session");
        assert_eq!(effect.config_revision, "8");
        assert!(requires_reconfigure(
            "bootstrap",
            &serde_json::json!({}),
            Some("subscription")
        ));

        after.config_revision = before.config_revision;
        assert!(PlatformEffect::for_config_change(&before, &after)
            .unwrap()
            .is_none());
    }

    #[test]
    fn mode_changes_reconfigure_each_active_engine_with_the_committed_revision() {
        for engine in [
            paws_model::VpnEngine::Meow,
            paws_model::VpnEngine::Mihomo,
            paws_model::VpnEngine::SingBox,
        ] {
            let before = CommandContext {
                active_profile: Some("selected-profile".into()),
                active_engine: Some(engine),
                vpn_session_id: Some("running-session".into()),
                config_revision: 9,
                mode: RuntimeMode::Rule,
                ..Default::default()
            };
            let mut after = before.clone();
            after.mode = RuntimeMode::Direct;
            after.config_revision = 10;
            assert!(requires_reconfigure(
                "mode.set",
                &serde_json::json!({"mode": "direct"}),
                before.active_profile.as_deref()
            ));
            let effect = PlatformEffect::for_config_change(&before, &after)
                .unwrap()
                .unwrap();
            assert_eq!(effect.kind, "restart");
            assert_eq!(effect.owner, "running-session");
            assert_eq!(effect.config_revision, "10");
        }
    }

    #[test]
    fn editing_inactive_profiles_and_restoring_copies_do_not_restart_the_vpn() {
        let payload = serde_json::json!({ "profileId": "library-profile" });
        assert!(!requires_reconfigure(
            "profiles.save",
            &payload,
            Some("running-profile")
        ));
        assert!(!requires_reconfigure(
            "profiles.delete",
            &payload,
            Some("running-profile")
        ));
        assert!(!requires_reconfigure(
            "settings.dns",
            &payload,
            Some("running-profile")
        ));
        assert!(requires_reconfigure(
            "profiles.save",
            &payload,
            Some("library-profile")
        ));
        assert!(requires_reconfigure(
            "profiles.activate",
            &payload,
            Some("running-profile")
        ));
        assert!(!requires_reconfigure(
            "backups.restore",
            &payload,
            Some("running-profile")
        ));
        assert!(!requires_reconfigure(
            "backups.webdavRestore",
            &payload,
            Some("running-profile")
        ));
    }
}

fields!(Probe {
    group: String,
    url: String
});
fields!(ConverterDraft {
    draft: crate::converter::SubscriptionConverterDraft
});
fields!(ShortUrl {
    api: String,
    long_url: String
});
fields!(UploadConfig {
    api: String,
    content: String
});
fields!(Backend { backend: String });
