use base64::Engine;
use hmeta_model::{
    GeodataFileSummary, HMetaError, ManualRuleMatchKind, ManualRuleMutation,
    ManualRuleMutationKind, ManualRuleSpec, PerAppMode, ProfileSummary, ProviderSummary,
    RuleSummary, RuntimeMode, SubscriptionMetadata, SubscriptionUserInfo, VpnOptions,
    DEFAULT_CHINA_DNS_SERVERS, DEFAULT_GLOBAL_DNS_FALLBACKS,
};
use ipnet::IpNet;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::{Host, Url};

const STORE_VERSION: u32 = 1;
const MEOW_V4_CLIENT: &str = "172.19.0.1/30";
const MEOW_V4_ROUTER: &str = "172.19.0.2";
const MEOW_V6_CLIENT: &str = "fdfe:dcba:9876::1/126";
pub const MANUAL_ACTIVITY_RULE_SOURCE: &str = "manual:activity";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDocument {
    pub id: String,
    pub name: String,
    pub source: String,
    pub raw_yaml_path: String,
    #[serde(default)]
    pub yaml_backup_path: Option<String>,
    pub subscription_url: Option<String>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub last_refresh_at: Option<String>,
    #[serde(default)]
    pub last_refresh_error: Option<String>,
    #[serde(default)]
    pub selected_proxies: BTreeMap<String, String>,
    #[serde(default)]
    pub upload_bytes: u64,
    #[serde(default)]
    pub download_bytes: u64,
    #[serde(default)]
    pub subscription_user_info: Option<SubscriptionUserInfo>,
    #[serde(default)]
    pub subscription_metadata: Option<SubscriptionMetadata>,
}

impl ProfileDocument {
    pub fn summary(
        &self,
        active_profile: Option<&str>,
        rule_count: usize,
        now_nanos: u128,
        store_root: &Path,
    ) -> ProfileSummary {
        let next_refresh_at = self.next_refresh_at();
        let refresh_due = self
            .subscription_url
            .as_ref()
            .is_some_and(|_| self.refresh_due_at(now_nanos));
        ProfileSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            source: self.source.clone(),
            raw_yaml_path: store_root
                .join(&self.raw_yaml_path)
                .to_string_lossy()
                .into_owned(),
            runtime_yaml_path: store_root
                .join("runtime")
                .join(format!("{}.yaml", self.id))
                .to_string_lossy()
                .into_owned(),
            active: active_profile == Some(self.id.as_str()),
            updated_at: self.updated_at.clone(),
            last_refresh_at: self.last_refresh_at.clone(),
            last_refresh_error: self.last_refresh_error.clone(),
            subscription_url: self.subscription_url.clone(),
            rule_count,
            selected_proxies: self.selected_proxies.clone(),
            has_backup: self.yaml_backup_path.is_some(),
            upload_bytes: self.upload_bytes,
            download_bytes: self.download_bytes,
            subscription_user_info: self.subscription_user_info.clone(),
            subscription_metadata: self.subscription_metadata.clone(),
            next_refresh_at,
            refresh_due,
        }
    }

    pub fn next_refresh_at(&self) -> Option<String> {
        let interval_hours = self
            .subscription_metadata
            .as_ref()
            .and_then(|metadata| metadata.update_interval_hours)
            .filter(|interval| *interval > 0)?;
        let base = self
            .last_refresh_at
            .as_ref()
            .or(self.updated_at.as_ref())?
            .parse::<u128>()
            .ok()?;
        let interval = u128::from(interval_hours)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000_000_000);
        Some(base.saturating_add(interval).to_string())
    }

    pub fn refresh_due_at(&self, now_nanos: u128) -> bool {
        self.next_refresh_at()
            .and_then(|value| value.parse::<u128>().ok())
            .is_some_and(|next| now_nanos >= next)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDocument {
    pub id: String,
    pub profile_id: String,
    pub line: String,
    pub enabled: bool,
    pub order: u32,
    pub source: String,
}

impl RuleDocument {
    pub fn summary(&self) -> RuleSummary {
        RuleSummary {
            id: self.id.clone(),
            profile_id: self.profile_id.clone(),
            line: self.line.clone(),
            enabled: self.enabled,
            order: self.order,
            source: self.source.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreIndex {
    version: u32,
    active_profile: Option<String>,
    profiles: Vec<ProfileDocument>,
    rules: Vec<RuleDocument>,
}

impl Default for StoreIndex {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            active_profile: None,
            profiles: Vec::new(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    root: PathBuf,
    active_profile: Option<String>,
    profiles: BTreeMap<String, ProfileDocument>,
    rules: BTreeMap<String, RuleDocument>,
}

impl ProfileStore {
    pub fn open_default() -> Result<Self, HMetaError> {
        let root = std::env::var("HMETA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_store_root());
        Self::open(root)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, HMetaError> {
        let root = root.into();
        fs::create_dir_all(root.join("profiles")).map_err(io_error)?;
        fs::create_dir_all(root.join("backups")).map_err(io_error)?;
        fs::create_dir_all(root.join("runtime")).map_err(io_error)?;
        fs::create_dir_all(root.join("providers/proxy")).map_err(io_error)?;
        fs::create_dir_all(root.join("providers/rule")).map_err(io_error)?;
        fs::create_dir_all(root.join("geodata")).map_err(io_error)?;

        let index_path = root.join("profiles.json");
        if !index_path.exists() {
            let store = Self {
                root,
                active_profile: None,
                profiles: BTreeMap::new(),
                rules: BTreeMap::new(),
            };
            store.save()?;
            return Ok(store);
        }

        let content = fs::read_to_string(&index_path).map_err(io_error)?;
        let index: StoreIndex = serde_json::from_str(&content)
            .map_err(|err| HMetaError::InvalidJson(err.to_string()))?;
        Ok(Self {
            root,
            active_profile: index.active_profile,
            profiles: index
                .profiles
                .into_iter()
                .map(|profile| (profile.id.clone(), profile))
                .collect(),
            rules: index
                .rules
                .into_iter()
                .map(|rule| (rule.id.clone(), rule))
                .collect(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn seed_empty() -> Self {
        Self::open_default().unwrap_or_else(|_| Self {
            root: default_store_root(),
            active_profile: None,
            profiles: BTreeMap::new(),
            rules: BTreeMap::new(),
        })
    }

    pub fn import_profile_content(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        raw_yaml: impl Into<String>,
        subscription_url: Option<String>,
    ) -> Result<String, HMetaError> {
        self.import_profile_content_with_subscription_info(
            name,
            source,
            raw_yaml,
            subscription_url,
            None,
        )
    }

    pub fn import_profile_content_with_subscription_info(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        raw_yaml: impl Into<String>,
        subscription_url: Option<String>,
        subscription_user_info: Option<SubscriptionUserInfo>,
    ) -> Result<String, HMetaError> {
        self.import_profile_content_with_subscription_metadata(
            name,
            source,
            raw_yaml,
            subscription_url,
            subscription_user_info,
            None,
        )
    }

    pub fn import_profile_content_with_subscription_metadata(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        raw_yaml: impl Into<String>,
        subscription_url: Option<String>,
        subscription_user_info: Option<SubscriptionUserInfo>,
        subscription_metadata: Option<SubscriptionMetadata>,
    ) -> Result<String, HMetaError> {
        let raw_profile = raw_yaml.into();
        let subscription_user_info =
            subscription_user_info.or_else(|| parse_subscription_userinfo_comment(&raw_profile));
        let subscription_metadata = merge_subscription_metadata(
            subscription_metadata,
            parse_subscription_metadata_comment(&raw_profile),
        );
        let raw_yaml = normalize_profile_content(&raw_profile)?;
        let id = next_id("profile");
        let raw_yaml_path = format!("profiles/{id}.yaml");
        let yaml_backup_path = format!("backups/{id}.yaml");
        fs::write(self.root.join(&raw_yaml_path), &raw_yaml).map_err(io_error)?;
        fs::write(self.root.join(&yaml_backup_path), &raw_yaml).map_err(io_error)?;
        self.profiles.insert(
            id.clone(),
            ProfileDocument {
                id: id.clone(),
                name: name.into(),
                source: source.into(),
                raw_yaml_path,
                yaml_backup_path: Some(yaml_backup_path),
                subscription_url,
                updated_at: Some(now_string()),
                last_refresh_at: None,
                last_refresh_error: None,
                selected_proxies: BTreeMap::new(),
                upload_bytes: 0,
                download_bytes: 0,
                subscription_user_info,
                subscription_metadata,
            },
        );
        if self.active_profile.is_none() {
            self.active_profile = Some(id.clone());
        }
        self.save()?;
        Ok(id)
    }

    pub fn replace_profile_content(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
    ) -> Result<(), HMetaError> {
        self.replace_profile_content_with_subscription_info(profile_id, raw_yaml, None)
    }

    pub fn replace_profile_content_with_subscription_info(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
        subscription_user_info: Option<SubscriptionUserInfo>,
    ) -> Result<(), HMetaError> {
        self.replace_profile_content_with_subscription_metadata(
            profile_id,
            raw_yaml,
            subscription_user_info,
            None,
        )
    }

    pub fn replace_profile_content_with_subscription_metadata(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
        subscription_user_info: Option<SubscriptionUserInfo>,
        subscription_metadata: Option<SubscriptionMetadata>,
    ) -> Result<(), HMetaError> {
        let raw_profile = raw_yaml.into();
        let subscription_user_info =
            subscription_user_info.or_else(|| parse_subscription_userinfo_comment(&raw_profile));
        let subscription_metadata = merge_subscription_metadata(
            subscription_metadata,
            parse_subscription_metadata_comment(&raw_profile),
        );
        let raw_yaml = normalize_profile_content(&raw_profile)?;
        let (raw_path, backup_path) = {
            let refreshed_at = now_string();
            let profile = self
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
            profile.updated_at = Some(refreshed_at.clone());
            profile.last_refresh_at = Some(refreshed_at);
            profile.last_refresh_error = None;
            if subscription_user_info.is_some() {
                profile.subscription_user_info = subscription_user_info;
            }
            if subscription_metadata.is_some() {
                profile.subscription_metadata = subscription_metadata;
            }
            if profile.yaml_backup_path.is_none() {
                profile.yaml_backup_path = Some(format!("backups/{profile_id}.yaml"));
            }
            (
                profile.raw_yaml_path.clone(),
                profile.yaml_backup_path.clone(),
            )
        };
        fs::write(self.root.join(raw_path), &raw_yaml).map_err(io_error)?;
        if let Some(backup_path) = backup_path {
            fs::write(self.root.join(backup_path), &raw_yaml).map_err(io_error)?;
        }
        self.save()
    }

    pub fn mark_profile_refresh_failed(
        &mut self,
        profile_id: &str,
        error: impl Into<String>,
    ) -> Result<(), HMetaError> {
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
        profile.last_refresh_at = Some(now_string());
        profile.last_refresh_error = Some(error.into());
        self.save()
    }

    pub fn update_profile_content(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
    ) -> Result<(), HMetaError> {
        let raw_yaml = normalize_profile_content(&raw_yaml.into())?;
        let raw_path = {
            let profile = self
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
            profile.updated_at = Some(now_string());
            profile.raw_yaml_path.clone()
        };
        fs::write(self.root.join(raw_path), raw_yaml).map_err(io_error)?;
        self.save()
    }

    pub fn update_profile_subscription(
        &mut self,
        profile_id: &str,
        name: impl Into<String>,
        subscription_url: impl Into<String>,
    ) -> Result<(), HMetaError> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(HMetaError::Core("profile name cannot be empty".to_owned()));
        }
        let subscription_url = subscription_url.into().trim().to_owned();
        let parsed = Url::parse(&subscription_url).map_err(subscription_error)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(HMetaError::Core(
                "subscription URL must use http or https".to_owned(),
            ));
        }
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
        profile.name = name;
        profile.source = subscription_url.clone();
        profile.subscription_url = Some(subscription_url);
        self.save()
    }

    pub fn restore_profile_backup(&mut self, profile_id: &str) -> Result<(), HMetaError> {
        let (raw_path, backup_path) = {
            let profile = self
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
            let backup_path = profile
                .yaml_backup_path
                .clone()
                .ok_or_else(|| HMetaError::Core(format!("profile {profile_id} has no backup")))?;
            profile.updated_at = Some(now_string());
            (profile.raw_yaml_path.clone(), backup_path)
        };
        let backup = fs::read_to_string(self.root.join(backup_path)).map_err(io_error)?;
        fs::write(self.root.join(raw_path), backup).map_err(io_error)?;
        self.save()
    }

    pub fn delete_profile(&mut self, profile_id: &str) -> Result<(), HMetaError> {
        let profile = self
            .profiles
            .remove(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
        let _ = fs::remove_file(self.root.join(profile.raw_yaml_path));
        if let Some(backup_path) = profile.yaml_backup_path {
            let _ = fs::remove_file(self.root.join(backup_path));
        }
        let _ = fs::remove_file(self.root.join("runtime").join(format!("{profile_id}.yaml")));
        let _ = fs::remove_dir_all(self.root.join("providers/proxy").join(profile_id));
        let _ = fs::remove_dir_all(self.root.join("providers/rule").join(profile_id));
        self.rules.retain(|_, rule| rule.profile_id != profile_id);
        if self.active_profile.as_deref() == Some(profile_id) {
            self.active_profile = self.profiles.keys().next().cloned();
        }
        self.save()
    }

    pub fn add_profile_traffic(
        &mut self,
        profile_id: &str,
        upload_delta: u64,
        download_delta: u64,
    ) -> Result<(), HMetaError> {
        if upload_delta == 0 && download_delta == 0 {
            return Ok(());
        }
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
        profile.upload_bytes = profile.upload_bytes.saturating_add(upload_delta);
        profile.download_bytes = profile.download_bytes.saturating_add(download_delta);
        self.save()
    }

    pub fn set_profile_per_app_config(
        &mut self,
        profile_id: &str,
        mode: PerAppMode,
        trusted_applications: Vec<String>,
        blocked_applications: Vec<String>,
    ) -> Result<(), HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(HMetaError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let hmeta_key = value_key("hmeta");
        let mut hmeta = root
            .remove(&hmeta_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        put_string(&mut hmeta, "per-app-mode", mode.as_str());
        hmeta.insert(
            value_key("trusted-applications"),
            Value::Sequence(
                normalize_applications(trusted_applications)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        hmeta.insert(
            value_key("blocked-applications"),
            Value::Sequence(
                normalize_applications(blocked_applications)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        root.insert(hmeta_key, Value::Mapping(hmeta));

        let raw_yaml =
            serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)
    }

    pub fn set_profile_dns_servers(
        &mut self,
        profile_id: &str,
        dns_servers: Vec<String>,
    ) -> Result<(), HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let options = vpn_options_from_yaml(&raw_yaml)?;
        self.set_profile_dns_config(
            profile_id,
            dns_servers,
            options.dns_fallbacks,
            options.dns_nameserver_policy,
        )
    }

    pub fn set_profile_dns_config(
        &mut self,
        profile_id: &str,
        dns_servers: Vec<String>,
        dns_fallbacks: Vec<String>,
        dns_nameserver_policy: BTreeMap<String, Vec<String>>,
    ) -> Result<(), HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(HMetaError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let dns_key = value_key("dns");
        let mut dns = root
            .remove(&dns_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        dns.insert(
            value_key("nameserver"),
            Value::Sequence(
                normalize_dns_servers(dns_servers)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        let dns_fallbacks = normalize_dns_optional_servers(dns_fallbacks);
        if dns_fallbacks.is_empty() {
            dns.remove(&value_key("fallback"));
        } else {
            dns.insert(
                value_key("fallback"),
                Value::Sequence(dns_fallbacks.into_iter().map(Value::String).collect()),
            );
        }
        let dns_nameserver_policy = normalize_dns_policy(dns_nameserver_policy);
        if dns_nameserver_policy.is_empty() {
            dns.remove(&value_key("nameserver-policy"));
        } else {
            dns.insert(
                value_key("nameserver-policy"),
                Value::Mapping(
                    dns_nameserver_policy
                        .into_iter()
                        .map(|(matcher, servers)| {
                            (
                                value_key(&matcher),
                                Value::Sequence(servers.into_iter().map(Value::String).collect()),
                            )
                        })
                        .collect(),
                ),
            );
        }
        root.insert(dns_key, Value::Mapping(dns));

        let raw_yaml =
            serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)
    }

    pub fn set_profile_vpn_config(
        &mut self,
        profile_id: &str,
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    ) -> Result<(), HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(HMetaError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let hmeta_key = value_key("hmeta");
        let mut hmeta = root
            .remove(&hmeta_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        put_bool(&mut hmeta, "system-proxy", system_proxy);
        put_bool(&mut hmeta, "allow-bypass", allow_bypass);
        root.insert(hmeta_key, Value::Mapping(hmeta));

        let tun_key = value_key("tun");
        let mut tun = root
            .remove(&tun_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        let stack = normalize_vpn_stack(stack);
        put_string(&mut tun, "stack", &stack);
        if dns_hijacking {
            tun.insert(
                value_key("dns-hijack"),
                Value::Sequence(vec![Value::String("any:53".to_owned())]),
            );
        } else {
            put_bool(&mut tun, "dns-hijack", false);
        }
        root.insert(tun_key, Value::Mapping(tun));

        let raw_yaml =
            serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)
    }

    pub fn set_active(&mut self, id: &str) -> Result<(), HMetaError> {
        if !self.profiles.contains_key(id) {
            return Err(HMetaError::ProfileNotFound(id.to_owned()));
        }
        if self.active_profile.as_deref() == Some(id) {
            return Ok(());
        }
        self.active_profile = Some(id.to_owned());
        self.save()
    }

    pub fn active_profile(&self) -> Option<&str> {
        self.active_profile.as_deref()
    }

    pub fn profile(&self, profile_id: &str) -> Result<&ProfileDocument, HMetaError> {
        self.profiles
            .get(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))
    }

    pub fn selected_proxies(
        &self,
        profile_id: &str,
    ) -> Result<BTreeMap<String, String>, HMetaError> {
        Ok(self.profile(profile_id)?.selected_proxies.clone())
    }

    pub fn set_selected_proxy(
        &mut self,
        profile_id: &str,
        group_name: impl Into<String>,
        proxy_name: impl Into<String>,
    ) -> Result<(), HMetaError> {
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| HMetaError::ProfileNotFound(profile_id.to_owned()))?;
        profile
            .selected_proxies
            .insert(group_name.into(), proxy_name.into());
        self.save()
    }

    pub fn raw_yaml(&self, profile_id: &str) -> Result<String, HMetaError> {
        let profile = self.profile(profile_id)?;
        fs::read_to_string(self.root.join(&profile.raw_yaml_path)).map_err(io_error)
    }

    pub fn active_raw_yaml(&self) -> Result<String, HMetaError> {
        let id = self
            .active_profile
            .as_deref()
            .ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
        self.raw_yaml(id)
    }

    pub fn runtime_yaml_path(&self, profile_id: &str) -> PathBuf {
        self.root.join("runtime").join(format!("{profile_id}.yaml"))
    }

    pub fn vpn_options_for_profile(&self, profile_id: &str) -> Result<VpnOptions, HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        vpn_options_from_yaml(&raw_yaml)
    }

    pub fn active_vpn_options(&self) -> Result<VpnOptions, HMetaError> {
        let id = self
            .active_profile
            .as_deref()
            .ok_or_else(|| HMetaError::ProfileNotFound("<active>".to_owned()))?;
        self.vpn_options_for_profile(id)
    }

    pub fn summaries(&self) -> Vec<ProfileSummary> {
        let now = now_nanos();
        self.profiles
            .values()
            .map(|profile| {
                let rule_count = self
                    .rules
                    .values()
                    .filter(|rule| rule.profile_id == profile.id && rule.enabled)
                    .count();
                profile.summary(self.active_profile(), rule_count, now, &self.root)
            })
            .collect()
    }

    pub fn due_subscription_summaries(&self) -> Vec<ProfileSummary> {
        self.summaries()
            .into_iter()
            .filter(|profile| profile.subscription_url.is_some() && profile.refresh_due)
            .collect()
    }

    pub fn rules_for_profile(&self, profile_id: &str) -> Vec<RuleSummary> {
        let mut rules: Vec<_> = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id)
            .cloned()
            .collect();
        rules.sort_by_key(|rule| rule.order);
        rules.into_iter().map(|rule| rule.summary()).collect()
    }

    pub fn active_rules(&self) -> Vec<RuleSummary> {
        self.active_profile()
            .map(|id| self.rules_for_profile(id))
            .unwrap_or_default()
    }

    pub fn import_rules_for_profile(
        &mut self,
        profile_id: &str,
        source: impl Into<String>,
        rules_text: &str,
    ) -> Result<Vec<String>, HMetaError> {
        self.profile(profile_id)?;
        let source = source.into();
        let mut next_order = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id)
            .map(|rule| rule.order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut ids = Vec::new();
        for line in rules_text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let id = next_id("rule");
            self.rules.insert(
                id.clone(),
                RuleDocument {
                    id: id.clone(),
                    profile_id: profile_id.to_owned(),
                    line: line.to_owned(),
                    enabled: true,
                    order: next_order,
                    source: source.clone(),
                },
            );
            next_order = next_order.saturating_add(1);
            ids.push(id);
        }
        self.save()?;
        Ok(ids)
    }

    /// Stage a validated activity rule in memory. The caller can render and
    /// validate the resulting runtime configuration before committing it with
    /// [`ProfileStore::persist`]. Rules with the same selector are replaced
    /// instead of stacked, and the latest explicit override is kept first.
    pub fn stage_manual_rule(
        &mut self,
        profile_id: &str,
        spec: &ManualRuleSpec,
    ) -> Result<ManualRuleMutation, HMetaError> {
        self.profile(profile_id)?;
        let normalized = normalize_manual_rule_spec(spec)?;
        let line = normalized.line();
        let mut matching = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id)
            .filter_map(|rule| {
                manual_rule_selector(&rule.line)
                    .filter(|selector| selector == &normalized.selector())
                    .map(|_| rule.clone())
            })
            .collect::<Vec<_>>();
        matching.sort_by_key(|rule| rule.order);

        let (rule_id, kind, replaced_line) = if let Some(existing) = matching.first() {
            let reordered = existing.order != 0 || matching.len() > 1;
            let kind = if existing.line.trim() != line {
                ManualRuleMutationKind::Updated
            } else if !existing.enabled {
                ManualRuleMutationKind::Reenabled
            } else if reordered {
                ManualRuleMutationKind::Updated
            } else {
                ManualRuleMutationKind::Unchanged
            };
            let replaced_line = (existing.line.trim() != line).then(|| existing.line.clone());
            let rule = self
                .rules
                .get_mut(&existing.id)
                .ok_or_else(|| HMetaError::RuleNotFound(existing.id.clone()))?;
            rule.line = line.clone();
            rule.enabled = true;
            rule.source = MANUAL_ACTIVITY_RULE_SOURCE.to_owned();
            (existing.id.clone(), kind, replaced_line)
        } else {
            let id = next_id("rule");
            self.rules.insert(
                id.clone(),
                RuleDocument {
                    id: id.clone(),
                    profile_id: profile_id.to_owned(),
                    line: line.clone(),
                    enabled: true,
                    order: 0,
                    source: MANUAL_ACTIVITY_RULE_SOURCE.to_owned(),
                },
            );
            (id, ManualRuleMutationKind::Added, None)
        };

        let duplicate_ids = matching
            .into_iter()
            .skip(1)
            .map(|rule| rule.id)
            .collect::<Vec<_>>();
        for duplicate_id in &duplicate_ids {
            self.rules.remove(duplicate_id);
        }

        let mut ordered_ids = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id && rule.id != rule_id)
            .map(|rule| (rule.order, rule.id.clone()))
            .collect::<Vec<_>>();
        ordered_ids.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        if let Some(rule) = self.rules.get_mut(&rule_id) {
            rule.order = 0;
        }
        for (index, (_, id)) in ordered_ids.into_iter().enumerate() {
            if let Some(rule) = self.rules.get_mut(&id) {
                rule.order = index.saturating_add(1) as u32;
            }
        }

        Ok(ManualRuleMutation {
            rule_id,
            line,
            kind,
            replaced_line,
            removed_duplicates: duplicate_ids.len(),
        })
    }

    pub fn set_rule_enabled(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
    ) -> Result<(), HMetaError> {
        let rule = self
            .rules
            .get_mut(rule_id)
            .ok_or_else(|| HMetaError::RuleNotFound(rule_id.to_owned()))?;
        if rule.profile_id != profile_id {
            return Err(HMetaError::RuleNotFound(rule_id.to_owned()));
        }
        rule.enabled = enabled;
        self.save()
    }

    pub fn reorder_rules(
        &mut self,
        profile_id: &str,
        ordered_rule_ids: &[String],
    ) -> Result<(), HMetaError> {
        for (index, rule_id) in ordered_rule_ids.iter().enumerate() {
            let rule = self
                .rules
                .get_mut(rule_id)
                .ok_or_else(|| HMetaError::RuleNotFound(rule_id.clone()))?;
            if rule.profile_id != profile_id {
                return Err(HMetaError::RuleNotFound(rule_id.clone()));
            }
            rule.order = index as u32;
        }
        self.save()
    }

    pub fn delete_rule(&mut self, rule_id: &str) -> Result<(), HMetaError> {
        self.rules
            .remove(rule_id)
            .ok_or_else(|| HMetaError::RuleNotFound(rule_id.to_owned()))?;
        self.save()
    }

    pub fn build_runtime_yaml(
        &self,
        profile_id: &str,
        mode: RuntimeMode,
        vpn_options: &VpnOptions,
    ) -> Result<String, HMetaError> {
        let yaml = self.render_runtime_yaml(profile_id, mode, vpn_options)?;
        self.write_runtime_yaml(profile_id, &yaml)?;
        Ok(yaml)
    }

    pub fn render_runtime_yaml(
        &self,
        profile_id: &str,
        mode: RuntimeMode,
        vpn_options: &VpnOptions,
    ) -> Result<String, HMetaError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(HMetaError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        sanitize_app_managed_config(root);
        put_string(root, "mode", mode.as_str());
        put_bool(root, "ipv6", vpn_options.ipv6);
        put_string(root, "log-level", "info");
        put_i64(root, "mixed-port", 7890);
        put_string(root, "external-controller", "127.0.0.1:9090");
        patch_geox_url(root);
        patch_geodata_paths(root, &self.root)?;
        patch_dns(root, vpn_options);
        patch_tun(root, vpn_options);
        rewrite_provider_paths(root, &self.root, profile_id)?;
        merge_rules(root, self.enabled_rule_lines(profile_id));

        serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
    }

    pub fn write_runtime_yaml(&self, profile_id: &str, yaml: &str) -> Result<(), HMetaError> {
        fs::write(
            self.root.join("runtime").join(format!("{profile_id}.yaml")),
            yaml,
        )
        .map_err(io_error)
    }

    pub fn persist(&self) -> Result<(), HMetaError> {
        self.save()
    }

    pub fn providers_from_yaml(&self, raw_yaml: &str) -> Vec<ProviderSummary> {
        let Ok(value) = serde_yaml::from_str::<Value>(raw_yaml) else {
            return Vec::new();
        };
        let Some(root) = value.as_mapping() else {
            return Vec::new();
        };
        let mut providers = Vec::new();
        collect_provider_summaries(root, "proxy-providers", "proxy", &mut providers);
        collect_provider_summaries(root, "rule-providers", "rule", &mut providers);
        providers
    }

    pub fn geodata_files(&self) -> Vec<GeodataFileSummary> {
        [
            ("Country.mmdb", "GeoIP Country MMDB"),
            ("GeoLite2-ASN.mmdb", "GeoLite2 ASN MMDB"),
            ("geosite.dat", "GEOSITE DAT"),
        ]
        .into_iter()
        .map(|(file_name, label)| {
            let path = self.root.join("geodata").join(file_name);
            let metadata = path.metadata().ok().filter(std::fs::Metadata::is_file);
            GeodataFileSummary {
                name: label.to_owned(),
                path: path.to_string_lossy().into_owned(),
                exists: metadata.is_some(),
                bytes: metadata.as_ref().map(std::fs::Metadata::len),
                updated_at: metadata
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(system_time_secs),
            }
        })
        .collect()
    }

    fn save(&self) -> Result<(), HMetaError> {
        let index = StoreIndex {
            version: STORE_VERSION,
            active_profile: self.active_profile.clone(),
            profiles: self.profiles.values().cloned().collect(),
            rules: self.rules.values().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&index)
            .map_err(|err| HMetaError::InvalidJson(err.to_string()))?;
        fs::write(self.root.join("profiles.json"), json).map_err(io_error)
    }

    #[cfg(test)]
    fn seed_default(&mut self) -> Result<(), HMetaError> {
        let id = "default".to_owned();
        let raw_yaml_path = "profiles/default.yaml".to_owned();
        fs::write(self.root.join(&raw_yaml_path), default_runtime_yaml()).map_err(io_error)?;
        self.profiles.insert(
            id.clone(),
            ProfileDocument {
                id: id.clone(),
                name: "Default".to_owned(),
                source: "local".to_owned(),
                raw_yaml_path,
                yaml_backup_path: None,
                subscription_url: None,
                updated_at: Some(now_string()),
                last_refresh_at: None,
                last_refresh_error: None,
                selected_proxies: BTreeMap::new(),
                upload_bytes: 0,
                download_bytes: 0,
                subscription_user_info: None,
                subscription_metadata: None,
            },
        );
        self.active_profile = Some(id);
        self.save()
    }

    fn enabled_rule_lines(&self, profile_id: &str) -> Vec<String> {
        let mut rules: Vec<_> = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id && rule.enabled)
            .cloned()
            .collect();
        rules.sort_by_key(|rule| rule.order);
        rules.into_iter().map(|rule| rule.line).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedManualRule {
    match_kind: ManualRuleMatchKind,
    value: String,
    target: String,
    ipv6: bool,
}

impl NormalizedManualRule {
    fn selector(&self) -> (ManualRuleMatchKind, String) {
        (self.match_kind, self.value.clone())
    }

    fn line(&self) -> String {
        format!(
            "{},{},{}",
            self.match_kind.rule_type(self.ipv6),
            self.value,
            self.target
        )
    }
}

fn normalize_manual_rule_spec(spec: &ManualRuleSpec) -> Result<NormalizedManualRule, HMetaError> {
    let target = spec.target.trim();
    if target.is_empty() || target.contains(',') || target.contains('\n') || target.contains('\r') {
        return Err(HMetaError::Core(
            "manual rule target must be a proxy group or DIRECT".to_owned(),
        ));
    }
    let target = if target.eq_ignore_ascii_case("DIRECT") {
        "DIRECT".to_owned()
    } else {
        target.to_owned()
    };
    let (value, ipv6) = normalize_manual_rule_value(spec.match_kind, &spec.value)?;
    Ok(NormalizedManualRule {
        match_kind: spec.match_kind,
        value,
        target,
        ipv6,
    })
}

fn normalize_manual_rule_value(
    match_kind: ManualRuleMatchKind,
    value: &str,
) -> Result<(String, bool), HMetaError> {
    match match_kind {
        ManualRuleMatchKind::Domain | ManualRuleMatchKind::DomainSuffix => {
            let mut value = value.trim().trim_end_matches('.');
            if match_kind == ManualRuleMatchKind::DomainSuffix {
                value = value.trim_start_matches('.');
            }
            if value.is_empty() || value.contains(['/', ',', '\n', '\r']) {
                return Err(HMetaError::Core("invalid manual rule domain".to_owned()));
            }
            let domain = match Host::parse(value)
                .map_err(|_| HMetaError::Core(format!("invalid manual rule domain: {value}")))?
            {
                Host::Domain(domain) => domain,
                Host::Ipv4(_) | Host::Ipv6(_) => {
                    return Err(HMetaError::Core(
                        "use IP/CIDR matching for an IP address".to_owned(),
                    ));
                }
            };
            Ok((domain.to_ascii_lowercase(), false))
        }
        ManualRuleMatchKind::IpCidr => {
            let network = if value.trim().contains('/') {
                value
                    .trim()
                    .parse::<IpNet>()
                    .map_err(|_| HMetaError::Core(format!("invalid IP/CIDR: {}", value.trim())))?
            } else {
                let ip = value.trim().parse::<IpAddr>().map_err(|_| {
                    HMetaError::Core(format!("invalid IP address: {}", value.trim()))
                })?;
                IpNet::new(ip, if ip.is_ipv4() { 32 } else { 128 })
                    .map_err(|error| HMetaError::Core(format!("invalid IP address: {error}")))?
            }
            .trunc();
            Ok((network.to_string(), network.addr().is_ipv6()))
        }
    }
}

fn manual_rule_selector(line: &str) -> Option<(ManualRuleMatchKind, String)> {
    let mut fields = line.split(',').map(str::trim);
    let rule_type = fields.next()?.to_ascii_uppercase();
    let value = fields.next()?;
    let match_kind = match rule_type.as_str() {
        "DOMAIN" => ManualRuleMatchKind::Domain,
        "DOMAIN-SUFFIX" => ManualRuleMatchKind::DomainSuffix,
        "IP-CIDR" | "IP-CIDR6" => ManualRuleMatchKind::IpCidr,
        _ => return None,
    };
    normalize_manual_rule_value(match_kind, value)
        .ok()
        .map(|(value, _)| (match_kind, value))
}

pub fn normalize_profile_content(raw_profile: &str) -> Result<String, HMetaError> {
    if let Ok(yaml) = normalize_yaml(raw_profile) {
        return Ok(yaml);
    }

    for candidate in subscription_candidates(raw_profile) {
        if let Some(yaml) = subscription_links_to_yaml(&candidate)? {
            return Ok(yaml);
        }
    }

    normalize_yaml(raw_profile)
}

pub fn parse_subscription_userinfo(header: &str) -> Option<SubscriptionUserInfo> {
    let mut upload_bytes = None;
    let mut download_bytes = None;
    let mut total_bytes = None;
    let mut expire_at = None;

    for part in header.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "upload" => upload_bytes = value.parse::<u64>().ok(),
            "download" => download_bytes = value.parse::<u64>().ok(),
            "total" => total_bytes = value.parse::<u64>().ok(),
            "expire" => {
                expire_at = value
                    .parse::<u64>()
                    .ok()
                    .filter(|expire| *expire > 0)
                    .map(|expire| expire.to_string())
            }
            _ => {}
        }
    }

    if upload_bytes.is_none()
        && download_bytes.is_none()
        && total_bytes.is_none()
        && expire_at.is_none()
    {
        return None;
    }
    Some(SubscriptionUserInfo {
        upload_bytes: upload_bytes.unwrap_or(0),
        download_bytes: download_bytes.unwrap_or(0),
        total_bytes,
        expire_at,
    })
}

pub fn parse_subscription_userinfo_comment(raw_profile: &str) -> Option<SubscriptionUserInfo> {
    leading_subscription_comments(raw_profile).find_map(|comment| {
        let payload = comment
            .split_once(':')
            .and_then(|(key, value)| {
                key.trim()
                    .eq_ignore_ascii_case("subscription-userinfo")
                    .then_some(value.trim())
            })
            .unwrap_or(comment);
        parse_subscription_userinfo(payload)
    })
}

pub fn parse_subscription_metadata(
    title: Option<&str>,
    update_interval: Option<&str>,
    web_page_url: Option<&str>,
    support_url: Option<&str>,
) -> Option<SubscriptionMetadata> {
    let title = title.and_then(decode_subscription_header_text);
    let update_interval_hours = update_interval.and_then(|value| value.trim().parse::<u64>().ok());
    let web_page_url = web_page_url.and_then(clean_subscription_url);
    let support_url = support_url.and_then(clean_subscription_url);
    if title.is_none()
        && update_interval_hours.is_none()
        && web_page_url.is_none()
        && support_url.is_none()
    {
        return None;
    }
    Some(SubscriptionMetadata {
        title,
        update_interval_hours,
        web_page_url,
        support_url,
    })
}

pub fn parse_subscription_metadata_comment(raw_profile: &str) -> Option<SubscriptionMetadata> {
    let mut title = None;
    let mut update_interval = None;
    let mut web_page_url = None;
    let mut support_url = None;
    for comment in leading_subscription_comments(raw_profile) {
        let payload = comment
            .split_once(':')
            .and_then(|(key, value)| {
                key.trim()
                    .eq_ignore_ascii_case("subscription-metadata")
                    .then_some(value.trim())
            })
            .unwrap_or(comment);
        for part in payload.split(';') {
            let Some((key, value)) = split_subscription_metadata_part(part) else {
                continue;
            };
            match key.as_str() {
                "profile-title" if title.is_none() => title = Some(value),
                "profile-update-interval" | "update-interval" if update_interval.is_none() => {
                    update_interval = Some(value)
                }
                "profile-web-page-url" | "web-page-url" if web_page_url.is_none() => {
                    web_page_url = Some(value)
                }
                "support-url" if support_url.is_none() => support_url = Some(value),
                _ => {}
            }
        }
    }
    parse_subscription_metadata(title, update_interval, web_page_url, support_url)
}

fn leading_subscription_comments(raw_profile: &str) -> impl Iterator<Item = &str> {
    raw_profile
        .lines()
        .map(str::trim)
        .scan(false, |seen_content, line| {
            if *seen_content {
                return None;
            }
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() {
                return Some(None);
            }
            if let Some(comment) = line.strip_prefix('#') {
                return Some(Some(comment.trim()));
            }
            *seen_content = true;
            None
        })
        .flatten()
}

fn split_subscription_metadata_part(part: &str) -> Option<(String, &str)> {
    let (key, value) = part.split_once('=').or_else(|| part.split_once(':'))?;
    let key = key.trim().replace('_', "-").to_ascii_lowercase();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

pub fn merge_subscription_metadata(
    primary: Option<SubscriptionMetadata>,
    fallback: Option<SubscriptionMetadata>,
) -> Option<SubscriptionMetadata> {
    match (primary, fallback) {
        (Some(mut primary), Some(fallback)) => {
            if primary.title.is_none() {
                primary.title = fallback.title;
            }
            if primary.update_interval_hours.is_none() {
                primary.update_interval_hours = fallback.update_interval_hours;
            }
            if primary.web_page_url.is_none() {
                primary.web_page_url = fallback.web_page_url;
            }
            if primary.support_url.is_none() {
                primary.support_url = fallback.support_url;
            }
            Some(primary)
        }
        (Some(primary), None) => Some(primary),
        (None, fallback) => fallback,
    }
}

pub fn parse_content_disposition_filename(header: &str) -> Option<String> {
    let mut fallback = None;
    for part in header.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "filename*" {
            let value = value.strip_prefix("UTF-8''").unwrap_or(value);
            return decode_subscription_header_text(value);
        }
        if key == "filename" {
            fallback = decode_subscription_header_text(value);
        }
    }
    fallback
}

pub fn decode_subscription_header_text(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    let text = decoded.trim().trim_matches('"').trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn clean_subscription_url(value: &str) -> Option<String> {
    let value = decode_subscription_header_text(value)?;
    Url::parse(&value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|_| value)
}

pub fn normalize_yaml(raw_yaml: &str) -> Result<String, HMetaError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    if !matches!(value, Value::Mapping(_)) {
        return Err(HMetaError::Core(
            "profile root must be a YAML map or supported proxy subscription".to_owned(),
        ));
    }
    serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
}

pub fn sanitize_profile_for_meow_validation(raw_yaml: &str) -> Result<String, HMetaError> {
    let mut value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping_mut() else {
        return Err(HMetaError::Core(
            "profile root must be a YAML map or supported proxy subscription".to_owned(),
        ));
    };
    sanitize_app_managed_config(root);
    sanitize_app_managed_dns_for_validation(root);
    remove_app_managed_geodata_fields(root);
    serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
}

fn subscription_candidates(raw_profile: &str) -> Vec<String> {
    let mut candidates = vec![raw_profile.trim().to_owned()];
    let compact = raw_profile
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    for candidate in [raw_profile.trim(), compact.as_str()] {
        if let Some(decoded) = decode_base64_text(candidate) {
            candidates.push(decoded);
        }
    }
    candidates.dedup();
    candidates
}

fn decode_base64_text(value: &str) -> Option<String> {
    for text in decode_base64_candidates(value) {
        if text.contains("://") || serde_yaml::from_str::<Value>(&text).is_ok() {
            return Some(text);
        }
    }
    None
}

fn decode_base64_component(value: &str) -> Option<String> {
    decode_base64_candidates(value).into_iter().next()
}

fn decode_base64_candidates(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut padded = value.trim().to_owned();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::URL_SAFE,
    ]
    .into_iter()
    .filter_map(|engine| {
        let bytes = engine.decode(&padded).ok()?;
        String::from_utf8(bytes).ok()
    })
    .collect()
}

fn link_scheme(link: &str) -> &str {
    link.split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or_default()
}

fn strip_link_scheme<'a>(link: &'a str, scheme: &str) -> Option<&'a str> {
    let prefix = link.get(..scheme.len())?;
    prefix
        .eq_ignore_ascii_case(scheme)
        .then(|| link.get(scheme.len()..))
        .flatten()
}

fn subscription_links_to_yaml(content: &str) -> Result<Option<String>, HMetaError> {
    let mut proxies = Vec::new();
    let mut first_parse_error = None;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_subscription_comment_line(line))
    {
        match parse_subscription_link(line) {
            Ok(Some(proxy)) => proxies.push(proxy),
            Ok(None) => {}
            Err(err) => {
                first_parse_error.get_or_insert(err);
            }
        }
    }
    if proxies.is_empty() {
        if let Some(err) = first_parse_error {
            return Err(err);
        }
        return Ok(None);
    }
    Ok(Some(proxy_subscription_yaml(proxies)?))
}

fn is_subscription_comment_line(line: &str) -> bool {
    line.starts_with('#') || line.starts_with("//") || line.starts_with(';')
}

fn parse_subscription_link(link: &str) -> Result<Option<Mapping>, HMetaError> {
    match link_scheme(link).to_ascii_lowercase().as_str() {
        "vless" => parse_vless_link(link).map(Some),
        "trojan" => parse_trojan_link(link).map(Some),
        "ss" => parse_ss_link(link).map(Some),
        "ssr" => parse_ssr_link(link).map(Some),
        "vmess" => parse_vmess_link(link).map(Some),
        "hysteria2" | "hy2" => parse_hysteria2_link(link).map(Some),
        "hysteria" => parse_hysteria_link(link).map(Some),
        "tuic" => parse_tuic_link(link).map(Some),
        "http" | "https" => parse_http_link(link).map(Some),
        "socks" | "socks5" => parse_socks5_link(link).map(Some),
        _ => Ok(None),
    }
}

fn parse_vless_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "VLESS"),
        "vless",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "uuid", &decode_component(url.username()));
    apply_query_udp_option(&mut proxy, &query, true);
    if vless_tls_enabled(&query) {
        put_bool(&mut proxy, "tls", true);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "serverName", "servername"]) {
        put_string(&mut proxy, "servername", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(flow) = query.get("flow") {
        put_string(&mut proxy, "flow", flow);
    }
    if let Some(encryption) = query.get("encryption") {
        let encryption = encryption.trim();
        if !encryption.is_empty() {
            put_string(&mut proxy, "encryption", &encryption.to_ascii_lowercase());
        }
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    apply_query_transport_options(&mut proxy, &query);
    apply_query_tls_options(&mut proxy, &query);
    Ok(proxy)
}

fn parse_trojan_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Trojan"),
        "trojan",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "password", &decode_component(url.username()));
    apply_query_udp_option(&mut proxy, &query, true);
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    apply_query_transport_options(&mut proxy, &query);
    apply_query_tls_options(&mut proxy, &query);
    Ok(proxy)
}

fn parse_ss_link(link: &str) -> Result<Mapping, HMetaError> {
    let without_scheme = strip_link_scheme(link, "ss://")
        .ok_or_else(|| HMetaError::Core("invalid ss subscription link".to_owned()))?;
    let (body, fragment) = without_scheme
        .split_once('#')
        .map(|(body, fragment)| (body, Some(fragment)))
        .unwrap_or((without_scheme, None));
    let body = body.split_once('?').map(|(body, _)| body).unwrap_or(body);
    let query = query_without_fragment(without_scheme);
    let sip002 = if body.contains('@') {
        body.to_owned()
    } else {
        decode_base64_component(body)
            .ok_or_else(|| HMetaError::Core("invalid ss subscription link".to_owned()))?
    };
    let parsed = Url::parse(&format!("ss://{sip002}")).map_err(subscription_error)?;
    let query_values = raw_query_map(query);
    let user = decode_component(parsed.username());
    let (cipher, password) = if let Some(password) = parsed.password() {
        (user, decode_component(password))
    } else {
        let decoded = decode_base64_text(&user)
            .ok_or_else(|| HMetaError::Core("invalid ss user info".to_owned()))?;
        decoded
            .split_once(':')
            .map(|(cipher, password)| (cipher.to_owned(), password.to_owned()))
            .ok_or_else(|| HMetaError::Core("invalid ss cipher/password".to_owned()))?
    };
    let mut proxy = proxy_base(
        fragment
            .map(decode_component)
            .filter(|name| !name.is_empty())
            .or_else(|| proxy_name_from_query(&query_values))
            .unwrap_or_else(|| default_proxy_name("SS", &parsed)),
        "ss",
        url_host(&parsed)?,
        url_port(&parsed)?,
    );
    put_string(&mut proxy, "cipher", &cipher);
    put_string(&mut proxy, "password", &password);
    apply_query_udp_option(&mut proxy, &query_values, true);
    apply_ss_plugin_options(&mut proxy, query);
    apply_raw_query_common_proxy_options(&mut proxy, query);
    Ok(proxy)
}

fn parse_ssr_link(link: &str) -> Result<Mapping, HMetaError> {
    let encoded = strip_link_scheme(link, "ssr://")
        .ok_or_else(|| HMetaError::Core("invalid ssr subscription link".to_owned()))?
        .trim();
    let decoded = decode_base64_component(encoded)
        .ok_or_else(|| HMetaError::Core("invalid ssr subscription link".to_owned()))?;
    let (endpoint, query) = decoded
        .split_once("/?")
        .or_else(|| decoded.split_once('?'))
        .map(|(endpoint, query)| (endpoint, query))
        .unwrap_or((decoded.as_str(), ""));
    let mut parts = endpoint.splitn(6, ':');
    let server = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing server".to_owned()))?;
    let port = parts
        .next()
        .and_then(|port| port.parse::<u16>().ok())
        .ok_or_else(|| HMetaError::Core("ssr missing port".to_owned()))?;
    let protocol = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing protocol".to_owned()))?;
    let cipher = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing cipher".to_owned()))?;
    let obfs = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing obfs".to_owned()))?;
    let password = parts
        .next()
        .and_then(decode_base64_component)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing password".to_owned()))?;
    let query = raw_query_map(query);
    let name = ssr_query_base64_any(&query, &["remarks", "remark", "name", "ps"])
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("SSR-{server}"));
    let mut proxy = proxy_base(name, "ssr", server.to_owned(), port);
    put_string(&mut proxy, "cipher", cipher);
    put_string(&mut proxy, "password", &password);
    put_string(&mut proxy, "protocol", protocol);
    put_string(&mut proxy, "obfs", obfs);
    if let Some(protocol_param) = ssr_query_base64_any(
        &query,
        &[
            "protoparam",
            "proto-param",
            "protoParam",
            "protocol-param",
            "protocolParam",
            "protocol_param",
        ],
    )
    .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "protocol-param", &protocol_param);
    }
    if let Some(obfs_param) = ssr_query_base64_any(
        &query,
        &["obfsparam", "obfs-param", "obfsParam", "obfs_param"],
    )
    .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "obfs-param", &obfs_param);
    }
    if let Some(group) = ssr_query_base64_any(&query, &["group", "groupName", "group-name"])
        .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "group", &group);
    }
    apply_query_udp_option(&mut proxy, &query, true);
    Ok(proxy)
}

fn ssr_query_base64_any(query: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    query_get_any(query, keys).and_then(decode_base64_component)
}

fn parse_vmess_link(link: &str) -> Result<Mapping, HMetaError> {
    let encoded = strip_link_scheme(link, "vmess://")
        .ok_or_else(|| HMetaError::Core("invalid vmess subscription link".to_owned()))?
        .trim();
    let decoded = decode_base64_text(encoded)
        .ok_or_else(|| HMetaError::Core("invalid vmess subscription link".to_owned()))?;
    let value: serde_json::Value = serde_json::from_str(&decoded)
        .map_err(|err| HMetaError::Core(format!("invalid vmess JSON: {err}")))?;
    let server = json_str_any(&value, &["add", "server", "address", "addr"])
        .ok_or_else(|| HMetaError::Core("vmess missing server".to_owned()))?
        .to_owned();
    let port = json_u16_any(&value, &["port"])
        .ok_or_else(|| HMetaError::Core("vmess missing port".to_owned()))?;
    let mut proxy = proxy_base(
        json_str_any(
            &value,
            &["ps", "name", "remarks", "remark", "alias", "node-name"],
        )
        .unwrap_or("VMess")
        .to_owned(),
        "vmess",
        server,
        port,
    );
    if let Some(uuid) = json_str_any(&value, &["id", "uuid"]) {
        put_string(&mut proxy, "uuid", uuid);
    }
    if let Some(alter_id) = json_i64_any(&value, &["aid", "alterId", "alter_id"]) {
        put_i64(&mut proxy, "alterId", alter_id);
    }
    let security = json_str_any(&value, &["security"]);
    let security_is_tls = security.is_some_and(|security| security.eq_ignore_ascii_case("tls"));
    if let Some(cipher) = json_str_any(&value, &["scy", "cipher"]).or_else(|| {
        security
            .filter(|security| !security.eq_ignore_ascii_case("tls"))
            .filter(|security| !security.eq_ignore_ascii_case("none"))
    }) {
        put_string(&mut proxy, "cipher", cipher);
    }
    if json_str_any(&value, &["tls"]).is_some_and(tls_enabled_value) || security_is_tls {
        put_bool(&mut proxy, "tls", true);
    }
    if json_truthy_any(
        &value,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    let network = json_str_any(&value, &["net", "type", "network"]).map(str::to_ascii_lowercase);
    if let Some(network) = &network {
        put_string(&mut proxy, "network", network);
    }
    if let Some(servername) = json_str_any(&value, &["sni", "servername", "serverName"]) {
        put_string(&mut proxy, "servername", servername);
    }
    if let Some(fingerprint) = json_str_any(
        &value,
        &[
            "fp",
            "fingerprint",
            "client-fingerprint",
            "clientFingerprint",
        ],
    ) {
        put_string(&mut proxy, "client-fingerprint", fingerprint);
    }
    if let Some(alpn) = json_str_any(&value, &["alpn"]) {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if json_truthy_any(&value, &["udp"]) {
        put_bool(&mut proxy, "udp", true);
    }
    if json_truthy_any(&value, &["tfo", "fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "tfo", true);
    }
    match network.as_deref() {
        Some("ws") => put_ws_opts(
            &mut proxy,
            json_str_any(&value, &["path", "wsPath"]),
            json_str_any(&value, &["host", "wsHost"]),
            json_str_any(&value, &["ed", "maxEarlyData", "max-early-data"]),
            json_str_any(
                &value,
                &["eh", "earlyDataHeaderName", "early-data-header-name"],
            ),
        ),
        Some("grpc") => put_grpc_opts(
            &mut proxy,
            json_str_any(
                &value,
                &[
                    "path",
                    "serviceName",
                    "service",
                    "service-name",
                    "grpc-service-name",
                ],
            ),
            json_str_any(&value, &["mode", "grpc-mode"]),
        ),
        Some("h2") => put_h2_opts(
            &mut proxy,
            json_str_any(&value, &["path", "h2Path"]),
            json_str_any(&value, &["host", "h2Host"]),
        ),
        Some("httpupgrade") => put_http_upgrade_opts(
            &mut proxy,
            json_str_any(&value, &["path", "httpUpgradePath"]),
            json_str_any(&value, &["host", "httpUpgradeHost"]),
        ),
        _ => {}
    }
    Ok(proxy)
}

fn parse_hysteria2_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Hysteria2"),
        "hysteria2",
        url_host(&url)?,
        url_port(&url)?,
    );
    let username = decode_component(url.username());
    if let Some(password) = query_get_any(
        &query,
        &["password", "auth", "auth-str", "auth_str", "authStr"],
    )
    .map(ToOwned::to_owned)
    .or_else(|| (!username.is_empty()).then_some(username))
    {
        put_string(&mut proxy, "password", &password);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(obfs) = query.get("obfs").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "obfs", obfs);
    }
    if let Some(obfs_password) =
        query_get_any(&query, &["obfs-password", "obfsPassword", "obfs_password"])
    {
        put_string(&mut proxy, "obfs-password", obfs_password);
    }
    if let Some(up) = query_get_any(&query, &["up", "upmbps", "upMbps"]) {
        put_string(&mut proxy, "up", up);
    }
    if let Some(down) = query_get_any(&query, &["down", "downmbps", "downMbps"]) {
        put_string(&mut proxy, "down", down);
    }
    if let Some(ports) = query_get_any(
        &query,
        &[
            "ports",
            "mport",
            "port-hopping",
            "port_hopping",
            "portHopping",
        ],
    ) {
        put_string(&mut proxy, "ports", ports);
    }
    if let Some(recv_window_conn) = query_get_any(
        &query,
        &["recv-window-conn", "recv_window_conn", "recvWindowConn"],
    ) {
        put_string(&mut proxy, "recv-window-conn", recv_window_conn);
    }
    if let Some(recv_window) = query_get_any(&query, &["recv-window", "recv_window", "recvWindow"])
    {
        put_string(&mut proxy, "recv-window", recv_window);
    }
    if truthy_query_any(
        &query,
        &[
            "disable-mtu-discovery",
            "disable_mtu_discovery",
            "disableMtuDiscovery",
        ],
    ) {
        put_bool(&mut proxy, "disable-mtu-discovery", true);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

fn parse_hysteria_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Hysteria"),
        "hysteria",
        url_host(&url)?,
        url_port(&url)?,
    );
    let username = decode_component(url.username());
    if let Some(auth) = query_get_any(
        &query,
        &["auth", "auth-str", "auth_str", "authStr", "authString"],
    )
    .map(str::to_owned)
    .or_else(|| (!username.is_empty()).then_some(username))
    {
        put_string(&mut proxy, "auth-str", &auth);
    }
    if let Some(protocol) = query.get("protocol").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "protocol", protocol);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(obfs) = query.get("obfs").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "obfs", obfs);
    }
    if let Some(up) = query_get_any(&query, &["up", "upmbps", "upMbps"]) {
        put_string(&mut proxy, "up", up);
    }
    if let Some(down) = query_get_any(&query, &["down", "downmbps", "downMbps"]) {
        put_string(&mut proxy, "down", down);
    }
    if let Some(ports) = query_get_any(
        &query,
        &[
            "ports",
            "mport",
            "port-hopping",
            "port_hopping",
            "portHopping",
        ],
    ) {
        put_string(&mut proxy, "ports", ports);
    }
    if let Some(recv_window_conn) = query_get_any(
        &query,
        &["recv-window-conn", "recv_window_conn", "recvWindowConn"],
    ) {
        put_string(&mut proxy, "recv-window-conn", recv_window_conn);
    }
    if let Some(recv_window) = query_get_any(&query, &["recv-window", "recv_window", "recvWindow"])
    {
        put_string(&mut proxy, "recv-window", recv_window);
    }
    if truthy_query_any(
        &query,
        &[
            "disable-mtu-discovery",
            "disable_mtu_discovery",
            "disableMtuDiscovery",
        ],
    ) {
        put_bool(&mut proxy, "disable-mtu-discovery", true);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

fn parse_tuic_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let uuid = decode_component(url.username());
    let password = url.password().map(decode_component).unwrap_or_default();
    if uuid.is_empty() {
        return Err(HMetaError::Core("tuic missing uuid".to_owned()));
    }
    if password.is_empty() {
        return Err(HMetaError::Core("tuic missing password".to_owned()));
    }
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "TUIC"),
        "tuic",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "uuid", &uuid);
    put_string(&mut proxy, "password", &password);
    if let Some(sni) = query_get_any(&query, &["sni", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(congestion_controller) = query_get_any(
        &query,
        &[
            "congestion-controller",
            "congestion_control",
            "congestionControl",
        ],
    ) {
        put_string(&mut proxy, "congestion-controller", congestion_controller);
    }
    if let Some(udp_relay_mode) = query_get_any(
        &query,
        &["udp-relay-mode", "udp_relay_mode", "udpRelayMode"],
    ) {
        put_string(&mut proxy, "udp-relay-mode", udp_relay_mode);
    }
    if truthy_query_any(&query, &["disable-sni", "disable_sni", "disableSni"]) {
        put_bool(&mut proxy, "disable-sni", true);
    }
    if truthy_query_any(&query, &["reduce-rtt", "reduce_rtt", "reduceRtt"]) {
        put_bool(&mut proxy, "reduce-rtt", true);
    }
    if let Some(request_timeout) = query_get_any(
        &query,
        &["request-timeout", "request_timeout", "requestTimeout"],
    ) {
        put_string(&mut proxy, "request-timeout", request_timeout);
    }
    if let Some(heartbeat_interval) = query_get_any(
        &query,
        &[
            "heartbeat-interval",
            "heartbeat_interval",
            "heartbeatInterval",
        ],
    ) {
        put_string(&mut proxy, "heartbeat-interval", heartbeat_interval);
    }
    if let Some(max_packet_size) = query_get_any(
        &query,
        &[
            "max-udp-relay-packet-size",
            "max_udp_relay_packet_size",
            "maxUdpRelayPacketSize",
        ],
    ) {
        put_string(&mut proxy, "max-udp-relay-packet-size", max_packet_size);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

fn parse_http_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "HTTP"),
        "http",
        url_host(&url)?,
        url_port(&url)?,
    );
    if let Some((username, password)) = url_credentials(&url) {
        put_string(&mut proxy, "username", &username);
        put_string(&mut proxy, "password", &password);
    }
    if url.scheme() == "https" || truthy_query(&query, "tls") {
        put_bool(&mut proxy, "tls", true);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_http_header_query_options(&mut proxy, &query);
    Ok(proxy)
}

fn parse_socks5_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "SOCKS5"),
        "socks5",
        url_host(&url)?,
        url_port(&url)?,
    );
    if let Some((username, password)) = url_credentials(&url) {
        put_string(&mut proxy, "username", &username);
        put_string(&mut proxy, "password", &password);
    }
    if truthy_query(&query, "tls") {
        put_bool(&mut proxy, "tls", true);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    Ok(proxy)
}

fn apply_ss_plugin_options(proxy: &mut Mapping, query: &str) {
    let query = raw_query_map(query);
    let Some(plugin_value) = query.get("plugin").map(String::as_str) else {
        return;
    };
    let mut plugin_parts = plugin_value.split(';').map(str::trim);
    let Some(plugin) = plugin_parts.next().filter(|plugin| !plugin.is_empty()) else {
        return;
    };
    let inline_opts = plugin_parts
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(";");
    let explicit_opts =
        query_get_any(&query, &["plugin-opts", "pluginOpts", "pluginopts"]).unwrap_or("");
    let opts = [inline_opts.as_str(), explicit_opts]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(";");
    match plugin.to_ascii_lowercase().as_str() {
        "obfs" | "obfs-local" | "simple-obfs" => {
            put_string(proxy, "plugin", "obfs");
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
        "v2ray-plugin" => {
            put_string(proxy, "plugin", "v2ray-plugin");
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
        other => {
            put_string(proxy, "plugin", other);
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
    }
}

fn query_without_fragment(value: &str) -> &str {
    value
        .split_once('?')
        .map(|(_, query)| {
            query
                .split_once('#')
                .map(|(query, _)| query)
                .unwrap_or(query)
        })
        .unwrap_or("")
}

fn raw_query_map(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .fold(HashMap::new(), |mut map, (key, value)| {
            insert_query_value(&mut map, key, value);
            map
        })
}

fn apply_raw_query_common_proxy_options(proxy: &mut Mapping, query: &str) {
    let query = raw_query_map(query);
    apply_query_common_proxy_options(proxy, &query);
}

fn apply_query_common_proxy_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    if truthy_query(query, "udp") {
        put_bool(proxy, "udp", true);
    }
    if truthy_query_any(query, &["tfo", "fast-open", "fast_open", "fastOpen"]) {
        put_bool(proxy, "tfo", true);
    }
}

fn apply_http_header_query_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    let Some(raw_headers) = query_get_any(query, &["headers", "header"]) else {
        return;
    };
    let mut headers = Mapping::new();
    for part in raw_headers
        .split([';', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((key, value)) = part.split_once('=').or_else(|| part.split_once(':')) else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if !key.is_empty() && !value.is_empty() {
            put_string(&mut headers, key, value);
        }
    }
    if !headers.is_empty() {
        proxy.insert(value_key("headers"), Value::Mapping(headers));
    }
}

fn apply_query_udp_option(proxy: &mut Mapping, query: &HashMap<String, String>, default: bool) {
    match query_bool(query, "udp") {
        Some(udp) => put_bool(proxy, "udp", udp),
        None if default => put_bool(proxy, "udp", true),
        None => {}
    }
}

fn vless_tls_enabled(query: &HashMap<String, String>) -> bool {
    query
        .get("security")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "tls" | "reality"
            )
        })
        .unwrap_or(false)
        || truthy_query_any(query, &["tls", "enable-tls", "enable_tls", "enableTls"])
}

fn apply_query_transport_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    let network = query
        .get("type")
        .or_else(|| query.get("network"))
        .map(|network| network.to_ascii_lowercase());
    if let Some(network) = &network {
        put_string(proxy, "network", network);
    }
    match network.as_deref() {
        Some("ws") => put_ws_opts(
            proxy,
            query_get_any(query, &["path", "wsPath"]),
            query_get_any(query, &["host", "wsHost"]),
            query_get_any(query, &["ed", "maxEarlyData", "max-early-data"]),
            query_get_any(
                query,
                &["eh", "earlyDataHeaderName", "early-data-header-name"],
            ),
        ),
        Some("grpc") => put_grpc_opts(
            proxy,
            query_get_any(
                query,
                &[
                    "serviceName",
                    "service",
                    "service-name",
                    "grpc-service-name",
                    "path",
                ],
            ),
            query_get_any(query, &["mode", "grpc-mode"]),
        ),
        Some("h2") => put_h2_opts(
            proxy,
            query_get_any(query, &["path", "h2Path"]),
            query_get_any(query, &["host", "h2Host"]),
        ),
        Some("httpupgrade") => put_http_upgrade_opts(
            proxy,
            query_get_any(query, &["path", "httpUpgradePath"]),
            query_get_any(query, &["host", "httpUpgradeHost"]),
        ),
        _ => {}
    }
}

fn put_ws_opts(
    proxy: &mut Mapping,
    path: Option<&str>,
    host: Option<&str>,
    max_early_data: Option<&str>,
    early_data_header_name: Option<&str>,
) {
    let mut ws_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut ws_opts, "path", path);
    }
    if let Some(host) = host {
        let mut headers = Mapping::new();
        put_string(&mut headers, "Host", host);
        ws_opts.insert(value_key("headers"), Value::Mapping(headers));
    }
    if let Some(max_early_data) = max_early_data.and_then(parse_positive_i64) {
        put_i64(&mut ws_opts, "max-early-data", max_early_data);
    }
    if let Some(early_data_header_name) = early_data_header_name {
        put_string(
            &mut ws_opts,
            "early-data-header-name",
            early_data_header_name,
        );
    }
    if !ws_opts.is_empty() {
        proxy.insert(value_key("ws-opts"), Value::Mapping(ws_opts));
    }
}

fn put_grpc_opts(proxy: &mut Mapping, service_name: Option<&str>, mode: Option<&str>) {
    let mut grpc_opts = Mapping::new();
    if let Some(service_name) = service_name {
        put_string(&mut grpc_opts, "grpc-service-name", service_name);
    }
    if let Some(mode) = mode {
        put_string(&mut grpc_opts, "grpc-mode", mode);
    }
    if !grpc_opts.is_empty() {
        proxy.insert(value_key("grpc-opts"), Value::Mapping(grpc_opts));
    }
}

fn put_h2_opts(proxy: &mut Mapping, path: Option<&str>, host: Option<&str>) {
    let mut h2_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut h2_opts, "path", path);
    }
    if let Some(host) = host {
        let hosts = split_list(host);
        if !hosts.is_empty() {
            put_string_sequence(&mut h2_opts, "host", hosts);
        }
    }
    if !h2_opts.is_empty() {
        proxy.insert(value_key("h2-opts"), Value::Mapping(h2_opts));
    }
}

fn put_http_upgrade_opts(proxy: &mut Mapping, path: Option<&str>, host: Option<&str>) {
    let mut http_upgrade_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut http_upgrade_opts, "path", path);
    }
    if let Some(host) = host {
        put_string(&mut http_upgrade_opts, "host", host);
    }
    if !http_upgrade_opts.is_empty() {
        proxy.insert(
            value_key("http-upgrade-opts"),
            Value::Mapping(http_upgrade_opts),
        );
    }
}

fn apply_query_tls_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    if let Some(fingerprint) = query_get_any(
        query,
        &[
            "fp",
            "fingerprint",
            "client-fingerprint",
            "clientFingerprint",
        ],
    ) {
        put_string(proxy, "client-fingerprint", fingerprint);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(proxy, "alpn", split_list(alpn));
    }
    if query
        .get("security")
        .is_some_and(|security| security.eq_ignore_ascii_case("reality"))
    {
        let mut reality_opts = Mapping::new();
        if let Some(public_key) = query_get_any(query, &["pbk", "publicKey", "public-key"]) {
            put_string(&mut reality_opts, "public-key", public_key);
        }
        if let Some(short_id) = query_get_any(query, &["sid", "shortId", "short-id"]) {
            put_string(&mut reality_opts, "short-id", short_id);
        }
        if let Some(spider_x) = query_get_any(query, &["spx", "spiderX", "spider-x"]) {
            put_string(&mut reality_opts, "spider-x", spider_x);
        }
        if !reality_opts.is_empty() {
            proxy.insert(value_key("reality-opts"), Value::Mapping(reality_opts));
        }
    }
}

fn proxy_subscription_yaml(mut proxies: Vec<Mapping>) -> Result<String, HMetaError> {
    dedup_proxy_names(&mut proxies);
    let proxy_names = proxies
        .iter()
        .filter_map(|proxy| get_string(proxy, "name"))
        .collect::<Vec<_>>();
    let mut root = Mapping::new();
    put_i64(&mut root, "mixed-port", 7890);
    put_string(&mut root, "mode", "rule");
    put_string(&mut root, "log-level", "info");
    root.insert(
        value_key("proxies"),
        Value::Sequence(proxies.into_iter().map(Value::Mapping).collect()),
    );

    let mut group = Mapping::new();
    put_string(&mut group, "name", "Proxy");
    put_string(&mut group, "type", "select");
    let mut group_proxies = proxy_names
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    group_proxies.push(Value::String("DIRECT".to_owned()));
    group.insert(value_key("proxies"), Value::Sequence(group_proxies));
    root.insert(
        value_key("proxy-groups"),
        Value::Sequence(vec![Value::Mapping(group)]),
    );
    root.insert(
        value_key("rules"),
        Value::Sequence(vec![Value::String("MATCH,Proxy".to_owned())]),
    );
    serde_yaml::to_string(&Value::Mapping(root)).map_err(|err| HMetaError::Core(err.to_string()))
}

fn proxy_base(name: String, proxy_type: &str, server: String, port: u16) -> Mapping {
    let mut proxy = Mapping::new();
    put_string(&mut proxy, "name", &name);
    put_string(&mut proxy, "type", proxy_type);
    put_string(&mut proxy, "server", &server);
    put_i64(&mut proxy, "port", i64::from(port));
    proxy
}

fn dedup_proxy_names(proxies: &mut [Mapping]) {
    let mut counts = HashMap::<String, usize>::new();
    for proxy in proxies {
        let Some(name) = get_string(proxy, "name") else {
            continue;
        };
        let count = counts.entry(name.clone()).or_insert(0);
        if *count > 0 {
            put_string(proxy, "name", &format!("{name} {}", *count + 1));
        }
        *count += 1;
    }
}

fn query_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .fold(HashMap::new(), |mut map, (key, value)| {
            insert_query_value(&mut map, key, value);
            map
        })
}

fn insert_query_value(map: &mut HashMap<String, String>, key: String, value: String) {
    let lower_key = key.to_ascii_lowercase();
    map.entry(lower_key).or_insert_with(|| value.clone());
    map.entry(key).or_insert(value);
}

fn truthy_query(query: &HashMap<String, String>, key: &str) -> bool {
    query_get(query, key).is_some_and(truthy_value)
}

fn query_bool(query: &HashMap<String, String>, key: &str) -> Option<bool> {
    query_get(query, key).and_then(parse_boolish_value)
}

fn truthy_query_any(query: &HashMap<String, String>, keys: &[&str]) -> bool {
    keys.iter().any(|key| truthy_query(query, key))
}

fn truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "allow"
    )
}

fn parse_boolish_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "allow" | "on" => Some(true),
        "" | "0" | "false" | "no" | "none" | "off" | "deny" => Some(false),
        _ => None,
    }
}

fn tls_enabled_value(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "none"
    )
}

fn query_get_any<'a>(query: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| query_get(query, key))
        .filter(|value| !value.is_empty())
}

fn query_get<'a>(query: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    query
        .get(key)
        .or_else(|| query.get(&key.to_ascii_lowercase()))
        .map(String::as_str)
}

fn json_value_any<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| value.get(*key)).or_else(|| {
        let object = value.as_object()?;
        keys.iter().find_map(|key| {
            object
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
        })
    })
}

fn json_str_any<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    json_value_any(value, keys)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

fn json_u16_any(value: &serde_json::Value, keys: &[&str]) -> Option<u16> {
    json_value_any(value, keys).and_then(|value| match value {
        serde_json::Value::String(value) => value.parse::<u16>().ok(),
        serde_json::Value::Number(value) => value.as_u64().and_then(|value| {
            if value <= u16::MAX as u64 {
                Some(value as u16)
            } else {
                None
            }
        }),
        _ => None,
    })
}

fn json_i64_any(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    json_value_any(value, keys).and_then(|value| match value {
        serde_json::Value::String(value) => value.parse::<i64>().ok(),
        serde_json::Value::Number(value) => value.as_i64(),
        _ => None,
    })
}

fn json_truthy_any(value: &serde_json::Value, keys: &[&str]) -> bool {
    json_value_any(value, keys).is_some_and(|value| match value {
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::String(value) => truthy_value(value),
        serde_json::Value::Number(value) => value.as_u64() == Some(1),
        _ => false,
    })
}

fn url_host(url: &Url) -> Result<String, HMetaError> {
    url.host_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| HMetaError::Core("subscription link missing host".to_owned()))
}

fn url_port(url: &Url) -> Result<u16, HMetaError> {
    url.port()
        .ok_or_else(|| HMetaError::Core("subscription link missing port".to_owned()))
}

fn url_credentials(url: &Url) -> Option<(String, String)> {
    let username = decode_component(url.username());
    let password = url.password().map(decode_component).unwrap_or_default();
    if username.is_empty() && password.is_empty() {
        None
    } else {
        Some((username, password))
    }
}

fn fragment_name(url: &Url) -> Option<String> {
    url.fragment()
        .map(decode_component)
        .filter(|name| !name.is_empty())
}

fn proxy_name_from_url(url: &Url, query: &HashMap<String, String>, prefix: &str) -> String {
    fragment_name(url)
        .or_else(|| proxy_name_from_query(query))
        .unwrap_or_else(|| default_proxy_name(prefix, url))
}

fn proxy_name_from_query(query: &HashMap<String, String>) -> Option<String> {
    query_get_any(
        query,
        &[
            "remarks",
            "remark",
            "name",
            "ps",
            "alias",
            "node",
            "node-name",
            "nodeName",
        ],
    )
    .map(ToOwned::to_owned)
}

fn default_proxy_name(prefix: &str, url: &Url) -> String {
    format!(
        "{prefix}-{}",
        url.host_str().unwrap_or("proxy").replace(['[', ']'], "")
    )
}

fn decode_component(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn subscription_error(error: url::ParseError) -> HMetaError {
    HMetaError::Core(format!("invalid subscription link: {error}"))
}

pub fn vpn_options_from_yaml(raw_yaml: &str) -> Result<VpnOptions, HMetaError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping() else {
        return Err(HMetaError::Core(
            "profile root must be a YAML map".to_owned(),
        ));
    };

    let mut options = VpnOptions::default();
    if let Some(ipv6) = get_bool(root, "ipv6") {
        options.ipv6 = ipv6;
    }

    if let Some(Value::Mapping(dns)) = root.get(&value_key("dns")) {
        let nameservers = get_string_list(dns, "nameserver");
        if !nameservers.is_empty() {
            options.dns_servers = nameservers;
        }
        if dns.contains_key(&value_key("fallback")) {
            options.dns_fallbacks = get_string_list(dns, "fallback");
        }
        if dns.contains_key(&value_key("nameserver-policy")) {
            options.dns_nameserver_policy = get_string_list_map(dns, "nameserver-policy");
        }
    }

    if let Some(Value::Mapping(hmeta)) = root.get(&value_key("hmeta")) {
        if let Some(system_proxy) =
            get_bool(hmeta, "system-proxy").or_else(|| get_bool(hmeta, "systemProxy"))
        {
            options.system_proxy = system_proxy;
        }
        if let Some(allow_bypass) =
            get_bool(hmeta, "allow-bypass").or_else(|| get_bool(hmeta, "allowBypass"))
        {
            options.allow_bypass = allow_bypass;
        }
        if let Some(mode) = get_string(hmeta, "per-app-mode")
            .or_else(|| get_string(hmeta, "per-app"))
            .and_then(|mode| PerAppMode::try_from(mode.as_str()).ok())
        {
            options.per_app_mode = mode;
        }
        let trusted = get_string_list(hmeta, "trusted-applications");
        if !trusted.is_empty() {
            options.trusted_applications = trusted;
        }
        let blocked = get_string_list(hmeta, "blocked-applications");
        if !blocked.is_empty() {
            options.blocked_applications = blocked;
        }
    }

    if let Some(Value::Mapping(tun)) = root.get(&value_key("tun")) {
        if let Some(mtu) = get_u16(tun, "mtu") {
            options.mtu = mtu;
        }
        if let Some(stack) = get_string(tun, "stack") {
            options.stack = stack;
        }
        if let Some(Value::Bool(enabled)) = tun.get(&value_key("dns-hijack")) {
            options.dns_hijacking = *enabled;
        } else if let Some(Value::Sequence(items)) = tun.get(&value_key("dns-hijack")) {
            options.dns_hijacking = !items.is_empty();
        }

        let inet4 = get_string_list(tun, "inet4-address");
        let inet6 = get_string_list(tun, "inet6-address");
        let route_addresses = get_string_list(tun, "route-address");

        options.addresses.clear();
        if inet4.is_empty() {
            options.addresses.push(MEOW_V4_CLIENT.to_owned());
        } else {
            options.addresses.extend(inet4);
        }
        if !inet6.is_empty() {
            options.ipv6 = true;
            options.addresses.extend(inet6);
        }

        if !route_addresses.is_empty() {
            if route_addresses.iter().any(|route| route.contains(':')) {
                options.ipv6 = true;
            }
            options.routes = route_addresses;
        }
    }

    if options.per_app_mode == PerAppMode::Off {
        if !options.trusted_applications.is_empty() {
            options.per_app_mode = PerAppMode::Proxy;
        } else if !options.blocked_applications.is_empty() {
            options.per_app_mode = PerAppMode::Bypass;
        }
    }

    if options.ipv6
        && !options
            .addresses
            .iter()
            .any(|address| address.contains(':'))
    {
        options.addresses.push(MEOW_V6_CLIENT.to_owned());
    }
    if options.ipv6 && !options.routes.iter().any(|route| route.contains(':')) {
        options.routes.push("::/0".to_owned());
    }
    if options.dns_addresses.is_empty() {
        options.dns_addresses.push(MEOW_V4_ROUTER.to_owned());
    }

    Ok(options)
}

pub fn default_runtime_yaml() -> String {
    r#"mixed-port: 7890
mode: rule
log-level: info
external-controller: 127.0.0.1:9090
dns:
  enable: true
  listen: 127.0.0.1:1053
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - 223.5.5.5
    - 119.29.29.29
  fallback:
    - 1.1.1.1
    - 8.8.8.8
  nameserver-policy:
    geosite:cn:
      - 223.5.5.5
      - 119.29.29.29
    geosite:geolocation-!cn:
      - 1.1.1.1
      - 8.8.8.8
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#
    .to_owned()
}

fn patch_dns(root: &mut Mapping, options: &VpnOptions) {
    let key = value_key("dns");
    let mut dns = root
        .remove(&key)
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    put_bool(&mut dns, "enable", true);
    put_string(&mut dns, "listen", "127.0.0.1:1053");
    dns.insert(
        value_key("nameserver"),
        Value::Sequence(
            options
                .dns_servers
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    if !options.dns_fallbacks.is_empty() {
        dns.insert(
            value_key("fallback"),
            Value::Sequence(
                options
                    .dns_fallbacks
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    } else {
        dns.remove(&value_key("fallback"));
    }
    if !options.dns_nameserver_policy.is_empty() {
        dns.insert(
            value_key("nameserver-policy"),
            Value::Mapping(
                options
                    .dns_nameserver_policy
                    .iter()
                    .map(|(matcher, servers)| {
                        (
                            value_key(matcher),
                            Value::Sequence(servers.iter().cloned().map(Value::String).collect()),
                        )
                    })
                    .collect(),
            ),
        );
    } else {
        dns.remove(&value_key("nameserver-policy"));
    }
    remove_app_managed_dns_fields(&mut dns);
    put_bool(&mut dns, "use-system-hosts", false);
    dns.insert(
        value_key("default-nameserver"),
        Value::Sequence(
            default_dns_bootstrap_servers(options)
                .map(|server| Value::String(server.to_owned()))
                .collect(),
        ),
    );
    root.insert(key, Value::Mapping(dns));
}

fn remove_app_managed_dns_fields(dns: &mut Mapping) {
    for key in [
        "enhanced-mode",
        "fake-ip-range",
        "fake-ip-filter",
        "fake-ip-filter-mode",
        "fallback-filter",
    ] {
        dns.remove(&value_key(key));
    }
}

fn sanitize_app_managed_dns_for_validation(root: &mut Mapping) {
    let Some(Value::Mapping(dns)) = root.get_mut(&value_key("dns")) else {
        return;
    };
    remove_app_managed_dns_fields(dns);
    dns.remove(&value_key("listen"));
    dns.remove(&value_key("default-nameserver"));
    put_bool(dns, "use-system-hosts", false);
}

fn sanitize_app_managed_config(root: &mut Mapping) {
    for key in [
        "port",
        "socks-port",
        "redir-port",
        "tproxy-port",
        "mixed-port",
        "allow-lan",
        "bind-address",
        "lan-allowed-ips",
        "lan-disallowed-ips",
        "authentication",
        "skip-auth-prefixes",
        "external-controller",
        "external-controller-tls",
        "external-controller-unix",
        "external-controller-pipe",
        "external-ui",
        "external-ui-name",
        "external-ui-url",
        "external-controller-cors",
        "secret",
        "routing-mark",
        "interface-name",
        "tproxy-sni",
        "subscriptions",
        "listeners",
    ] {
        root.remove(&value_key(key));
    }
}

fn patch_geox_url(root: &mut Mapping) {
    let key = value_key("geox-url");
    let mut geox = root
        .remove(&key)
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    put_string(
        &mut geox,
        "geoip",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
    );
    put_string(
        &mut geox,
        "geosite",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
    );
    put_string(
        &mut geox,
        "mmdb",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
    );
    put_string(
        &mut geox,
        "asn",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/GeoLite2-ASN.mmdb",
    );
    root.insert(key, Value::Mapping(geox));
}

fn patch_geodata_paths(root: &mut Mapping, store_root: &Path) -> Result<(), HMetaError> {
    let geodata_dir = store_root.join("geodata");
    fs::create_dir_all(&geodata_dir).map_err(io_error)?;
    let key = value_key("geodata");
    root.remove(&key);
    let mut geodata = Mapping::new();
    put_string(
        &mut geodata,
        "mmdb-path",
        &geodata_dir.join("Country.mmdb").to_string_lossy(),
    );
    put_string(
        &mut geodata,
        "asn-path",
        &geodata_dir.join("GeoLite2-ASN.mmdb").to_string_lossy(),
    );
    put_string(
        &mut geodata,
        "geosite-path",
        &geodata_dir.join("geosite.dat").to_string_lossy(),
    );
    root.insert(key, Value::Mapping(geodata));
    Ok(())
}

fn remove_app_managed_geodata_fields(root: &mut Mapping) {
    let Some(Value::Mapping(geodata)) = root.get_mut(&value_key("geodata")) else {
        return;
    };
    for key in [
        "auto-update",
        "auto-update-interval",
        "url",
        "geodata-mode",
        "geodata-loader",
        "geoip-matcher",
    ] {
        geodata.remove(&value_key(key));
    }
}

fn patch_tun(root: &mut Mapping, options: &VpnOptions) {
    let mut tun = Mapping::new();
    put_bool(&mut tun, "enable", true);
    put_string(&mut tun, "device", "HMeta");
    put_i64(&mut tun, "mtu", i64::from(options.mtu));
    put_string(&mut tun, "stack", &options.stack);
    put_bool(&mut tun, "auto-route", true);
    if options.dns_hijacking {
        tun.insert(
            value_key("dns-hijack"),
            Value::Sequence(vec![Value::String("any:53".to_owned())]),
        );
    }
    root.insert(value_key("tun"), Value::Mapping(tun));
}

fn rewrite_provider_paths(
    root: &mut Mapping,
    store_root: &Path,
    profile_id: &str,
) -> Result<(), HMetaError> {
    rewrite_provider_kind(
        root,
        "proxy-providers",
        store_root.join("providers/proxy").join(profile_id),
        false,
    )?;
    rewrite_provider_kind(
        root,
        "rule-providers",
        store_root.join("providers/rule").join(profile_id),
        true,
    )
}

fn rewrite_provider_kind(
    root: &mut Mapping,
    key: &str,
    cache_dir: PathBuf,
    trim_inline_rule_provider_fields: bool,
) -> Result<(), HMetaError> {
    let Some(Value::Mapping(providers)) = root.get_mut(&value_key(key)) else {
        return Ok(());
    };
    fs::create_dir_all(&cache_dir).map_err(io_error)?;
    for (name, provider) in providers {
        let Value::String(name) = name else {
            continue;
        };
        let Value::Mapping(provider) = provider else {
            continue;
        };
        if trim_inline_rule_provider_fields && provider_type_is(provider, "inline") {
            provider.remove(&value_key("path"));
            provider.remove(&value_key("interval"));
            continue;
        }
        let path = cache_dir.join(provider_cache_file_name(name));
        provider.insert(
            value_key("path"),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    Ok(())
}

fn provider_type_is(provider: &Mapping, expected: &str) -> bool {
    provider
        .get(&value_key("type"))
        .and_then(Value::as_str)
        .is_some_and(|provider_type| provider_type.eq_ignore_ascii_case(expected))
}

fn provider_cache_file_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = sanitized.trim_matches('.');
    if !base.is_empty() && base == name {
        return format!("{base}.yaml");
    }

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();
    let base = if base.is_empty() { "provider" } else { base };
    format!("{base}-{hash:016x}.yaml")
}

fn merge_rules(root: &mut Mapping, extra_rules: Vec<String>) {
    if extra_rules.is_empty() {
        return;
    }
    let mut original = root
        .remove(&value_key("rules"))
        .and_then(|value| value.as_sequence().cloned())
        .unwrap_or_default();
    let mut merged: Vec<Value> = extra_rules.into_iter().map(Value::String).collect();
    merged.append(&mut original);
    root.insert(value_key("rules"), Value::Sequence(dedup_rules(merged)));
}

fn dedup_rules(rules: Vec<Value>) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen_match = false;
    for rule in rules {
        let is_match = rule
            .as_str()
            .map(|line| line.trim_start().to_ascii_uppercase().starts_with("MATCH,"))
            .unwrap_or(false);
        if is_match {
            if seen_match {
                continue;
            }
            seen_match = true;
        }
        result.push(rule);
    }
    result
}

fn collect_provider_summaries(
    root: &Mapping,
    key: &str,
    provider_type: &str,
    providers: &mut Vec<ProviderSummary>,
) {
    let Some(Value::Mapping(items)) = root.get(&value_key(key)) else {
        return;
    };
    for (name, item) in items {
        let name = name.as_str().unwrap_or("<unnamed>").to_owned();
        let map = item.as_mapping();
        let path = map.and_then(|m| get_string(m, "path"));
        let cache_metadata = path.as_deref().and_then(provider_cache_metadata);
        let health_check = map
            .and_then(|m| m.get(&value_key("health-check")))
            .and_then(Value::as_mapping);
        providers.push(ProviderSummary {
            name,
            provider_type: provider_type.to_owned(),
            path,
            url: map.and_then(|m| get_string(m, "url")),
            vehicle_type: map.and_then(|m| get_string(m, "type")),
            interval_seconds: map.and_then(|m| get_u64(m, "interval")),
            filter: map.and_then(|m| get_string(m, "filter")),
            exclude_filter: map.and_then(|m| get_string(m, "exclude-filter")),
            behavior: map.and_then(|m| get_string(m, "behavior")),
            format: map.and_then(|m| get_string(m, "format")),
            health_check_enabled: health_check
                .and_then(|m| get_bool(m, "enable"))
                .unwrap_or(false),
            health_check_url: health_check.and_then(|m| get_string(m, "url")),
            health_check_interval_seconds: health_check.and_then(|m| get_u64(m, "interval")),
            expected_status: health_check.and_then(|m| get_string(m, "expected-status")),
            members: Vec::new(),
            cache_exists: cache_metadata
                .as_ref()
                .is_some_and(std::fs::Metadata::is_file),
            cache_bytes: cache_metadata
                .as_ref()
                .filter(|metadata| metadata.is_file())
                .map(std::fs::Metadata::len),
            cache_updated_at: cache_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_secs),
            stale_cache_available: false,
            last_refresh_at: None,
            last_refresh_error: None,
        });
    }
}

fn provider_cache_metadata(path: &str) -> Option<std::fs::Metadata> {
    Path::new(path).metadata().ok()
}

fn system_time_secs(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().to_string())
}

fn put_string(map: &mut Mapping, key: &str, value: &str) {
    map.insert(value_key(key), Value::String(value.to_owned()));
}

fn put_bool(map: &mut Mapping, key: &str, value: bool) {
    map.insert(value_key(key), Value::Bool(value));
}

fn put_i64(map: &mut Mapping, key: &str, value: i64) {
    map.insert(value_key(key), Value::Number(value.into()));
}

fn put_string_sequence(map: &mut Mapping, key: &str, values: Vec<String>) {
    if values.is_empty() {
        return;
    }
    map.insert(
        value_key(key),
        Value::Sequence(values.into_iter().map(Value::String).collect()),
    );
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_positive_i64(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok().filter(|value| *value > 0)
}

fn get_string(map: &Mapping, key: &str) -> Option<String> {
    map.get(&value_key(key))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn get_bool(map: &Mapping, key: &str) -> Option<bool> {
    map.get(&value_key(key)).and_then(Value::as_bool)
}

fn get_u64(map: &Mapping, key: &str) -> Option<u64> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_u64() {
        return Some(number).filter(|value| *value > 0);
    }
    value
        .as_str()
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn get_u16(map: &Mapping, key: &str) -> Option<u16> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_u64() {
        return u16::try_from(number).ok().filter(|value| *value > 0);
    }
    value
        .as_str()
        .and_then(|text| text.parse::<u16>().ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
fn get_i64(map: &Mapping, key: &str) -> Option<i64> {
    let value = map.get(&value_key(key))?;
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    value.as_str().and_then(|text| text.parse::<i64>().ok())
}

fn get_string_list(map: &Mapping, key: &str) -> Vec<String> {
    let Some(value) = map.get(&value_key(key)) else {
        return Vec::new();
    };
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Sequence(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(ToOwned::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn get_string_list_map(map: &Mapping, key: &str) -> BTreeMap<String, Vec<String>> {
    let Some(Value::Mapping(values)) = map.get(&value_key(key)) else {
        return BTreeMap::new();
    };
    values
        .iter()
        .filter_map(|(matcher, servers)| {
            let matcher = matcher.as_str()?.trim();
            if matcher.is_empty() {
                return None;
            }
            let servers = match servers {
                Value::String(server) => vec![server.clone()],
                Value::Sequence(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect(),
                _ => Vec::new(),
            };
            let servers = normalize_dns_optional_servers(servers);
            if servers.is_empty() {
                None
            } else {
                Some((matcher.to_owned(), servers))
            }
        })
        .collect()
}

fn normalize_applications(applications: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for application in applications {
        let application = application.trim();
        if application.is_empty() || normalized.iter().any(|item| item == application) {
            continue;
        }
        normalized.push(application.to_owned());
    }
    normalized
}

fn normalize_dns_servers(servers: Vec<String>) -> Vec<String> {
    let normalized = normalize_dns_optional_servers(servers);
    if normalized.is_empty() {
        VpnOptions::default().dns_servers
    } else {
        normalized
    }
}

fn default_dns_bootstrap_servers(options: &VpnOptions) -> impl Iterator<Item = &'static str> {
    let needs_global_bootstrap = dns_config_needs_default_nameserver(options);
    DEFAULT_CHINA_DNS_SERVERS.iter().copied().chain(
        DEFAULT_GLOBAL_DNS_FALLBACKS
            .iter()
            .copied()
            .filter(move |_| needs_global_bootstrap),
    )
}

fn normalize_dns_optional_servers(servers: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for server in servers {
        let server = server.trim();
        if server.is_empty() || normalized.iter().any(|item| item == server) {
            continue;
        }
        normalized.push(server.to_owned());
    }
    normalized
}

fn normalize_dns_policy(policy: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    policy
        .into_iter()
        .filter_map(|(matcher, servers)| {
            let matcher = matcher.trim();
            if matcher.is_empty() {
                return None;
            }
            let servers = normalize_dns_optional_servers(servers);
            if servers.is_empty() {
                None
            } else {
                Some((matcher.to_owned(), servers))
            }
        })
        .collect()
}

fn normalize_vpn_stack(stack: String) -> String {
    let stack = stack.trim();
    if stack.is_empty() {
        VpnOptions::default().stack
    } else {
        stack.to_owned()
    }
}

fn dns_config_needs_default_nameserver(options: &VpnOptions) -> bool {
    options
        .dns_servers
        .iter()
        .chain(options.dns_fallbacks.iter())
        .chain(
            options
                .dns_nameserver_policy
                .values()
                .flat_map(|servers| servers.iter()),
        )
        .any(|server| encrypted_dns_server_uses_hostname(server))
}

fn encrypted_dns_server_uses_hostname(server: &str) -> bool {
    let Some((scheme, _)) = server.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "https" | "tls" | "quic" | "h3") {
        return false;
    }
    Url::parse(server)
        .ok()
        .and_then(|url| url.host().map(|host| matches!(host, url::Host::Domain(_))))
        .unwrap_or(true)
}

fn value_key(key: &str) -> Value {
    Value::String(key.to_owned())
}

fn default_store_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(".hmeta")
}

fn next_id(prefix: &str) -> String {
    format!("{prefix}-{}", now_nanos())
}

fn now_string() -> String {
    now_nanos().to_string()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn io_error(error: std::io::Error) -> HMetaError {
    HMetaError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn new_store_starts_without_profiles() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("seed")));
        let store = ProfileStore::open(root).unwrap();
        assert_eq!(store.active_profile(), None);
        assert!(store.summaries().is_empty());
        assert!(store.active_raw_yaml().is_err());
    }

    #[test]
    fn profile_summary_exposes_raw_and_runtime_yaml_paths() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("summary-yaml-paths")
        ));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        store.seed_default().unwrap();

        let summary = store.summaries().into_iter().next().expect("summary");

        assert_eq!(
            summary.raw_yaml_path,
            root.join("profiles/default.yaml").to_string_lossy()
        );
        assert_eq!(
            summary.runtime_yaml_path,
            root.join("runtime/default.yaml").to_string_lossy()
        );
    }

    #[test]
    fn runtime_yaml_merges_rules_and_options() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("runtime")));
        let mut store = ProfileStore::open(root).unwrap();
        store.seed_default().unwrap();
        let rule_ids = store
            .import_rules_for_profile("default", "manual", "DOMAIN-SUFFIX,example.com,DIRECT")
            .unwrap();
        let yaml = store
            .build_runtime_yaml("default", RuntimeMode::Global, &VpnOptions::default())
            .unwrap();
        assert!(yaml.contains("mode: global"));
        assert!(yaml.contains("DOMAIN-SUFFIX,example.com,DIRECT"));
        assert!(yaml.contains("tun:"));

        store
            .set_rule_enabled("default", &rule_ids[0], false)
            .unwrap();
        let yaml = store
            .build_runtime_yaml("default", RuntimeMode::Global, &VpnOptions::default())
            .unwrap();
        assert!(!yaml.contains("DOMAIN-SUFFIX,example.com,DIRECT"));
        assert!(store
            .rules_for_profile("default")
            .iter()
            .any(|rule| rule.id == rule_ids[0] && !rule.enabled));

        store.delete_rule(&rule_ids[0]).unwrap();
        assert!(store.rules_for_profile("default").is_empty());
    }

    #[test]
    fn activity_rules_are_normalized_and_replace_conflicting_targets() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("activity-rule-upsert")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        store.seed_default().unwrap();
        store
            .import_rules_for_profile(
                "default",
                "manual-file",
                "DOMAIN,other.example,DIRECT\nDOMAIN-SUFFIX,Example.COM.,Proxy",
            )
            .unwrap();

        let mutation = store
            .stage_manual_rule(
                "default",
                &ManualRuleSpec {
                    match_kind: ManualRuleMatchKind::DomainSuffix,
                    value: ".EXAMPLE.com.".to_owned(),
                    target: "DIRECT".to_owned(),
                },
            )
            .unwrap();

        assert_eq!(mutation.kind, ManualRuleMutationKind::Updated);
        assert_eq!(mutation.line, "DOMAIN-SUFFIX,example.com,DIRECT");
        assert_eq!(
            mutation.replaced_line.as_deref(),
            Some("DOMAIN-SUFFIX,Example.COM.,Proxy")
        );
        let rules = store.rules_for_profile("default");
        assert_eq!(rules[0].line, mutation.line);
        assert_eq!(rules[0].source, MANUAL_ACTIVITY_RULE_SOURCE);
        assert_eq!(rules[1].line, "DOMAIN,other.example,DIRECT");
    }

    #[test]
    fn activity_ip_rules_use_canonical_host_prefixes_and_deduplicate() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("activity-ip-rule")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        store.seed_default().unwrap();
        store
            .import_rules_for_profile(
                "default",
                "manual-a",
                "IP-CIDR,192.0.2.7/32,DIRECT\nIP-CIDR,192.0.2.7/32,Proxy",
            )
            .unwrap();

        let mutation = store
            .stage_manual_rule(
                "default",
                &ManualRuleSpec {
                    match_kind: ManualRuleMatchKind::IpCidr,
                    value: "192.0.2.7".to_owned(),
                    target: "Proxy".to_owned(),
                },
            )
            .unwrap();

        assert_eq!(mutation.line, "IP-CIDR,192.0.2.7/32,Proxy");
        assert_eq!(mutation.removed_duplicates, 1);
        assert_eq!(
            store
                .rules_for_profile("default")
                .iter()
                .filter(|rule| rule.line.contains("192.0.2.7/32"))
                .count(),
            1
        );
    }

    #[test]
    fn activity_rules_reject_domain_ip_mixups_and_invalid_prefixes() {
        assert!(normalize_manual_rule_spec(&ManualRuleSpec {
            match_kind: ManualRuleMatchKind::Domain,
            value: "192.0.2.1".to_owned(),
            target: "DIRECT".to_owned(),
        })
        .is_err());
        assert!(normalize_manual_rule_spec(&ManualRuleSpec {
            match_kind: ManualRuleMatchKind::IpCidr,
            value: "192.0.2.1/64".to_owned(),
            target: "DIRECT".to_owned(),
        })
        .is_err());
    }

    #[test]
    fn rule_reorder_updates_runtime_yaml_order() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("reorder-rules")));
        let mut store = ProfileStore::open(root).unwrap();
        store.seed_default().unwrap();
        let rule_ids = store
            .import_rules_for_profile(
                "default",
                "manual",
                "DOMAIN-SUFFIX,alpha.example,DIRECT\nDOMAIN-SUFFIX,beta.example,Proxy",
            )
            .unwrap();

        store
            .reorder_rules("default", &[rule_ids[1].clone(), rule_ids[0].clone()])
            .unwrap();
        let rules = store.rules_for_profile("default");
        assert_eq!(rules[0].id, rule_ids[1]);
        assert_eq!(rules[1].id, rule_ids[0]);

        let yaml = store
            .build_runtime_yaml("default", RuntimeMode::Rule, &VpnOptions::default())
            .unwrap();
        let beta = yaml.find("DOMAIN-SUFFIX,beta.example,Proxy").unwrap();
        let alpha = yaml.find("DOMAIN-SUFFIX,alpha.example,DIRECT").unwrap();
        assert!(beta < alpha);
    }

    #[test]
    fn runtime_yaml_sanitizes_app_managed_fields_and_injects_geox_url() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("sanitize")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Noisy",
                "local",
                r#"
port: 8080
socks-port: 1080
mixed-port: 9999
external-controller: 0.0.0.0:9990
external-ui: ui
external-ui-name: metacubexd
external-ui-url: https://example.invalid/ui.zip
external-controller-cors:
  allow-origins:
    - "*"
secret: unsafe
authentication:
  - user:pass
skip-auth-prefixes:
  - 127.0.0.1/8
allow-lan: true
bind-address: 0.0.0.0
lan-allowed-ips:
  - 0.0.0.0/0
lan-disallowed-ips:
  - 127.0.0.1/8
external-controller-tls: 0.0.0.0:9443
external-controller-unix: /tmp/meow.sock
external-controller-pipe: meow
routing-mark: 666
interface-name: eth0
tproxy-sni: true
subscriptions:
  - ignored
listeners:
  - name: ignored
geodata:
  mmdb-path: /tmp/user.mmdb
  asn-path: /tmp/user-asn.mmdb
  geosite-path: /tmp/user-geosite.mrs
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
    asn: https://example.invalid/GeoLite2-ASN.mmdb
    geosite: https://example.invalid/geosite.mrs
  geodata-mode: memconservative
  geodata-loader: standard
  geoip-matcher: trie
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
                None,
            )
            .unwrap();

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let runtime_root = value.as_mapping().unwrap();

        assert!(runtime_root.get(&value_key("port")).is_none());
        assert!(runtime_root.get(&value_key("socks-port")).is_none());
        assert!(runtime_root.get(&value_key("allow-lan")).is_none());
        assert!(runtime_root.get(&value_key("bind-address")).is_none());
        assert!(runtime_root.get(&value_key("lan-allowed-ips")).is_none());
        assert!(runtime_root.get(&value_key("lan-disallowed-ips")).is_none());
        assert!(runtime_root.get(&value_key("authentication")).is_none());
        assert!(runtime_root.get(&value_key("skip-auth-prefixes")).is_none());
        assert!(runtime_root.get(&value_key("subscriptions")).is_none());
        assert!(runtime_root.get(&value_key("listeners")).is_none());
        assert!(runtime_root
            .get(&value_key("external-controller-tls"))
            .is_none());
        assert!(runtime_root
            .get(&value_key("external-controller-unix"))
            .is_none());
        assert!(runtime_root
            .get(&value_key("external-controller-pipe"))
            .is_none());
        assert!(runtime_root.get(&value_key("external-ui")).is_none());
        assert!(runtime_root.get(&value_key("external-ui-name")).is_none());
        assert!(runtime_root.get(&value_key("external-ui-url")).is_none());
        assert!(runtime_root
            .get(&value_key("external-controller-cors"))
            .is_none());
        assert!(runtime_root.get(&value_key("secret")).is_none());
        assert!(runtime_root.get(&value_key("routing-mark")).is_none());
        assert!(runtime_root.get(&value_key("interface-name")).is_none());
        assert!(runtime_root.get(&value_key("tproxy-sni")).is_none());
        assert_eq!(
            get_string(runtime_root, "external-controller").as_deref(),
            Some("127.0.0.1:9090")
        );
        assert_eq!(
            runtime_root
                .get(&value_key("mixed-port"))
                .and_then(Value::as_i64),
            Some(7890)
        );
        assert!(matches!(
            runtime_root.get(&value_key("geox-url")),
            Some(Value::Mapping(geox)) if geox.get(&value_key("geoip")).is_some()
        ));
        assert!(root.join("geodata").exists());
        let geodata = runtime_root
            .get(&value_key("geodata"))
            .and_then(Value::as_mapping)
            .expect("geodata paths");
        assert_eq!(
            get_string(geodata, "mmdb-path").as_deref(),
            Some(
                root.join("geodata")
                    .join("Country.mmdb")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            get_string(geodata, "asn-path").as_deref(),
            Some(
                root.join("geodata")
                    .join("GeoLite2-ASN.mmdb")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            get_string(geodata, "geosite-path").as_deref(),
            Some(
                root.join("geodata")
                    .join("geosite.dat")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(geodata.get(&value_key("auto-update")).is_none());
        assert!(geodata.get(&value_key("auto-update-interval")).is_none());
        assert!(geodata.get(&value_key("url")).is_none());
        assert!(geodata.get(&value_key("geodata-mode")).is_none());
        assert!(geodata.get(&value_key("geodata-loader")).is_none());
        assert!(geodata.get(&value_key("geoip-matcher")).is_none());
    }

    #[test]
    fn imported_subscription_profile_persists_metadata_and_yaml() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("subscription")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Remote Demo",
                "https://example.test/demo.yaml",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                Some("https://example.test/demo.yaml".to_owned()),
            )
            .unwrap();
        drop(store);

        let store = ProfileStore::open(root).unwrap();
        let profile = store.profile(&profile_id).unwrap();
        assert_eq!(profile.name, "Remote Demo");
        assert_eq!(
            profile.subscription_url.as_deref(),
            Some("https://example.test/demo.yaml")
        );
        assert!(store.raw_yaml(&profile_id).unwrap().contains("mixed-port"));
        assert!(profile.yaml_backup_path.is_some());
        assert!(profile.last_refresh_at.is_none());
        assert!(profile.last_refresh_error.is_none());
    }

    #[test]
    fn subscription_name_and_url_can_be_edited_like_reference_client() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("edit-subscription")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Old Name",
                "https://old.example.test/profile.yaml",
                "proxies: []\nproxy-groups: []\nrules: []\n",
                Some("https://old.example.test/profile.yaml".to_owned()),
            )
            .unwrap();

        store
            .update_profile_subscription(
                &profile_id,
                "New Name",
                "https://new.example.test/profile.yaml",
            )
            .unwrap();
        let profile = store.profile(&profile_id).unwrap();
        assert_eq!(profile.name, "New Name");
        assert_eq!(
            profile.subscription_url.as_deref(),
            Some("https://new.example.test/profile.yaml")
        );
        assert_eq!(profile.source, "https://new.example.test/profile.yaml");

        assert!(store
            .update_profile_subscription(&profile_id, "", "https://example.test/profile")
            .is_err());
        assert!(store
            .update_profile_subscription(&profile_id, "Name", "file:///tmp/profile.yaml")
            .is_err());
    }

    #[test]
    fn validation_yaml_removes_app_managed_geodata_fields() {
        let yaml = sanitize_profile_for_meow_validation(
            r#"
geodata:
  mmdb-path: /tmp/user.mmdb
  auto-update: true
  auto-update-interval: 0
  url:
    mmdb: https://example.invalid/Country.mmdb
  geodata-mode: memconservative
  geodata-loader: standard
  geoip-matcher: trie
proxies: []
"#,
        )
        .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let geodata = value
            .as_mapping()
            .and_then(|root| root.get(&value_key("geodata")))
            .and_then(Value::as_mapping)
            .expect("geodata");

        assert_eq!(
            get_string(geodata, "mmdb-path").as_deref(),
            Some("/tmp/user.mmdb")
        );
        assert!(geodata.get(&value_key("auto-update")).is_none());
        assert!(geodata.get(&value_key("auto-update-interval")).is_none());
        assert!(geodata.get(&value_key("url")).is_none());
        assert!(geodata.get(&value_key("geodata-mode")).is_none());
        assert!(geodata.get(&value_key("geodata-loader")).is_none());
        assert!(geodata.get(&value_key("geoip-matcher")).is_none());
    }

    #[test]
    fn validation_yaml_removes_app_managed_listener_and_dns_fields() {
        let yaml = sanitize_profile_for_meow_validation(
            r#"
port: 7890
mixed-port: 7890
external-controller: 0.0.0.0:9090
listeners:
  - name: duplicated
    type: mixed
    port: 7890
dns:
  enable: true
  listen: 0.0.0.0:53
  default-nameserver:
    - bad bootstrap
  nameserver:
    - 223.5.5.5
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fallback-filter:
    geoip: true
  use-system-hosts: true
proxies: []
"#,
        )
        .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().expect("root");
        let dns = root
            .get(&value_key("dns"))
            .and_then(Value::as_mapping)
            .expect("dns");

        assert!(root.get(&value_key("port")).is_none());
        assert!(root.get(&value_key("mixed-port")).is_none());
        assert!(root.get(&value_key("external-controller")).is_none());
        assert!(root.get(&value_key("listeners")).is_none());
        assert!(dns.get(&value_key("listen")).is_none());
        assert!(dns.get(&value_key("default-nameserver")).is_none());
        assert!(dns.get(&value_key("enhanced-mode")).is_none());
        assert!(dns.get(&value_key("fake-ip-range")).is_none());
        assert!(dns.get(&value_key("fallback-filter")).is_none());
        assert_eq!(get_bool(dns, "use-system-hosts"), Some(false));
        assert_eq!(get_string_list(dns, "nameserver"), vec!["223.5.5.5"]);
    }

    #[test]
    fn subscription_userinfo_header_is_parsed() {
        let info = parse_subscription_userinfo(
            "upload=1024; download=2048; total=4096; expire=1893456000",
        )
        .expect("subscription userinfo");
        assert_eq!(info.upload_bytes, 1024);
        assert_eq!(info.download_bytes, 2048);
        assert_eq!(info.total_bytes, Some(4096));
        assert_eq!(info.expire_at.as_deref(), Some("1893456000"));

        let partial = parse_subscription_userinfo("download=7").expect("partial userinfo");
        assert_eq!(partial.upload_bytes, 0);
        assert_eq!(partial.download_bytes, 7);
        assert_eq!(partial.total_bytes, None);
        assert_eq!(partial.expire_at, None);
        assert!(parse_subscription_userinfo("profile-title=demo").is_none());
    }

    #[test]
    fn subscription_userinfo_comment_is_parsed() {
        let info = parse_subscription_userinfo_comment(
            "# subscription-userinfo: upload=11; download=22; total=33; expire=1893456000;\nproxies: []",
        )
        .expect("comment userinfo");
        assert_eq!(info.upload_bytes, 11);
        assert_eq!(info.download_bytes, 22);
        assert_eq!(info.total_bytes, Some(33));
        assert_eq!(info.expire_at.as_deref(), Some("1893456000"));

        let compact = parse_subscription_userinfo_comment(
            "# upload=44; download=55; total=66; expire=0;\nproxies: []",
        )
        .expect("compact comment userinfo");
        assert_eq!(compact.upload_bytes, 44);
        assert_eq!(compact.download_bytes, 55);
        assert_eq!(compact.total_bytes, Some(66));
        assert_eq!(compact.expire_at, None);
        let second_line = parse_subscription_userinfo_comment(
            "# profile-title: Demo\n# subscription-userinfo: upload=77; download=88; total=99\nproxies: []",
        )
        .expect("second line userinfo");
        assert_eq!(second_line.upload_bytes, 77);
        assert_eq!(second_line.download_bytes, 88);
        assert_eq!(second_line.total_bytes, Some(99));
        assert!(parse_subscription_userinfo_comment("proxies: []").is_none());
        assert!(
            parse_subscription_userinfo_comment("proxies: []\n# upload=1; download=2").is_none()
        );
    }

    #[test]
    fn imported_profile_uses_subscription_userinfo_comment() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("comment-info")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Comment Info",
                "local",
                "# upload=12; download=34; total=100; expire=1893456000;\nmixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();
        let profile = store.profile(&profile_id).unwrap();
        let info = profile
            .subscription_user_info
            .as_ref()
            .expect("comment userinfo");
        assert_eq!(info.upload_bytes, 12);
        assert_eq!(info.download_bytes, 34);
        assert_eq!(info.total_bytes, Some(100));
        assert_eq!(info.expire_at.as_deref(), Some("1893456000"));
    }

    #[test]
    fn subscription_metadata_headers_are_parsed() {
        let metadata = parse_subscription_metadata(
            Some("%E6%B5%8B%E8%AF%95%E8%AE%A2%E9%98%85"),
            Some("24"),
            Some("https://example.com/dashboard"),
            Some("https://example.com/support"),
        )
        .expect("subscription metadata");
        assert_eq!(metadata.title.as_deref(), Some("测试订阅"));
        assert_eq!(metadata.update_interval_hours, Some(24));
        assert_eq!(
            metadata.web_page_url.as_deref(),
            Some("https://example.com/dashboard")
        );
        assert_eq!(
            metadata.support_url.as_deref(),
            Some("https://example.com/support")
        );
        assert!(parse_subscription_metadata(None, Some("bad"), Some("ftp://bad"), None).is_none());
    }

    #[test]
    fn subscription_metadata_comment_is_parsed() {
        let metadata = parse_subscription_metadata_comment(
            "# subscription-metadata: profile-title=%E6%B5%8B%E8%AF%95; profile-update-interval=12; profile-web-page-url=https://example.com/portal; support-url=https://example.com/support\nproxies: []",
        )
        .expect("subscription metadata comment");
        assert_eq!(metadata.title.as_deref(), Some("测试"));
        assert_eq!(metadata.update_interval_hours, Some(12));
        assert_eq!(
            metadata.web_page_url.as_deref(),
            Some("https://example.com/portal")
        );
        assert_eq!(
            metadata.support_url.as_deref(),
            Some("https://example.com/support")
        );

        let compact = parse_subscription_metadata_comment(
            "# profile-title=Compact; update_interval=6; web_page_url=https://example.com/home\nproxies: []",
        )
        .expect("compact metadata comment");
        assert_eq!(compact.title.as_deref(), Some("Compact"));
        assert_eq!(compact.update_interval_hours, Some(6));
        assert_eq!(
            compact.web_page_url.as_deref(),
            Some("https://example.com/home")
        );
        let multiline = parse_subscription_metadata_comment(
            "# profile-title: Multi Line\n# profile-update-interval: 18\n# profile-web-page-url: https://example.com/multi\n# support-url: https://example.com/help\nproxies: []",
        )
        .expect("multiline metadata comment");
        assert_eq!(multiline.title.as_deref(), Some("Multi Line"));
        assert_eq!(multiline.update_interval_hours, Some(18));
        assert_eq!(
            multiline.web_page_url.as_deref(),
            Some("https://example.com/multi")
        );
        assert_eq!(
            multiline.support_url.as_deref(),
            Some("https://example.com/help")
        );
        assert!(
            parse_subscription_metadata_comment("# upload=1; download=2\nproxies: []").is_none()
        );
        assert!(
            parse_subscription_metadata_comment("proxies: []\n# profile-title: Ignored").is_none()
        );
    }

    #[test]
    fn imported_profile_uses_subscription_metadata_comment_as_fallback() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("comment-metadata")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content_with_subscription_metadata(
                "Metadata Comment",
                "local",
                "# profile-title=Body Title; profile-update-interval=8; profile-web-page-url=https://example.com/body; support-url=https://example.com/support\nmixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
                None,
                Some(SubscriptionMetadata {
                    title: Some("Header Title".to_owned()),
                    update_interval_hours: None,
                    web_page_url: None,
                    support_url: None,
                }),
            )
            .unwrap();
        let profile = store.profile(&profile_id).unwrap();
        let metadata = profile
            .subscription_metadata
            .as_ref()
            .expect("merged metadata");
        assert_eq!(metadata.title.as_deref(), Some("Header Title"));
        assert_eq!(metadata.update_interval_hours, Some(8));
        assert_eq!(
            metadata.web_page_url.as_deref(),
            Some("https://example.com/body")
        );
        assert_eq!(
            metadata.support_url.as_deref(),
            Some("https://example.com/support")
        );
    }

    #[test]
    fn content_disposition_filename_is_parsed() {
        assert_eq!(
            parse_content_disposition_filename(
                "attachment; filename*=UTF-8''%E6%B5%8B%E8%AF%95.yaml"
            )
            .as_deref(),
            Some("测试.yaml")
        );
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"abc.yaml\"").as_deref(),
            Some("abc.yaml")
        );
    }

    #[test]
    fn subscription_update_interval_marks_due_profiles() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("due-refresh")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content_with_subscription_metadata(
                "Due Demo",
                "https://example.test/due.yaml",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                Some("https://example.test/due.yaml".to_owned()),
                None,
                Some(SubscriptionMetadata {
                    title: None,
                    update_interval_hours: Some(1),
                    web_page_url: None,
                    support_url: None,
                }),
            )
            .unwrap();
        {
            let profile = store.profiles.get_mut(&profile_id).unwrap();
            profile.updated_at = Some("1000".to_owned());
            profile.last_refresh_at = Some("1000".to_owned());
        }

        let profile = store.profile(&profile_id).unwrap();
        assert_eq!(profile.next_refresh_at().as_deref(), Some("3600000001000"));
        assert!(!profile.refresh_due_at(3_600_000_000_999));
        assert!(profile.refresh_due_at(3_600_000_001_000));

        let summaries = store.due_subscription_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, profile_id);
        assert!(summaries[0].refresh_due);
        assert_eq!(
            summaries[0].next_refresh_at.as_deref(),
            Some("3600000001000")
        );
    }

    #[test]
    fn refresh_success_and_failure_metadata_persist_with_profile() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("refresh-meta")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Remote Demo",
                "https://example.test/demo.yaml",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                Some("https://example.test/demo.yaml".to_owned()),
            )
            .unwrap();
        store
            .mark_profile_refresh_failed(&profile_id, "HTTP 500")
            .unwrap();
        let failed = store.profile(&profile_id).unwrap();
        assert_eq!(failed.last_refresh_error.as_deref(), Some("HTTP 500"));
        assert!(failed.last_refresh_at.is_some());

        store
            .replace_profile_content(
                &profile_id,
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
            )
            .unwrap();
        drop(store);

        let store = ProfileStore::open(root).unwrap();
        let profile = store.profile(&profile_id).unwrap();
        assert!(profile.last_refresh_at.is_some());
        assert!(profile.last_refresh_error.is_none());
        let summary = store
            .summaries()
            .into_iter()
            .find(|summary| summary.id == profile_id)
            .unwrap();
        assert!(summary.last_refresh_at.is_some());
        assert!(summary.last_refresh_error.is_none());
    }

    #[test]
    fn imported_base64_share_subscription_is_normalized_to_clash_yaml() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("share-sub")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let links = "\
vless://00000000-0000-0000-0000-000000000001@127.0.0.1:443?type=tcp&security=none#VLESS%20A
trojan://secret@proxy.example.test:443?sni=proxy.example.test#Trojan%20A
";
        let encoded = base64::engine::general_purpose::STANDARD.encode(links);
        let profile_id = store
            .import_profile_content(
                "Share Links",
                "subscription",
                encoded,
                Some("https://example.test/links".to_owned()),
            )
            .unwrap();
        let yaml = store.raw_yaml(&profile_id).unwrap();
        assert!(yaml.contains("type: vless"));
        assert!(yaml.contains("type: trojan"));
        assert!(yaml.contains("name: VLESS A"));
        assert!(yaml.contains("MATCH,Proxy"));
        assert!(store.vpn_options_for_profile(&profile_id).is_ok());
    }

    #[test]
    fn multiline_share_subscription_skips_comments_and_bad_links_when_valid_links_exist() {
        let links = "\
# generated by provider
not-a-share-link
ss://not-valid-base64
// disabled node
vless://00000000-0000-0000-0000-000000000001@127.0.0.1:443?type=tcp&security=none#VLESS%20Good
vmess://not-json
trojan://secret@proxy.example.test:443?sni=proxy.example.test#Trojan%20Good
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let names = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .filter_map(|proxy| get_string(proxy, "name"))
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["VLESS Good", "Trojan Good"]);
        assert!(yaml.contains("MATCH,Proxy"));
    }

    #[test]
    fn single_bad_share_link_still_reports_parse_error() {
        let err = normalize_profile_content("ss://not-valid-base64").unwrap_err();
        assert!(err.to_string().contains("invalid ss subscription link"));
    }

    #[test]
    fn imported_single_share_link_is_normalized_to_clash_yaml() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("single-share")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Single",
                "clipboard",
                "vless://00000000-0000-0000-0000-000000000001@example.test:8443?security=tls&sni=edge.example.test#Edge",
                None,
            )
            .unwrap();
        let yaml = store.raw_yaml(&profile_id).unwrap();
        assert!(yaml.contains("server: example.test"));
        assert!(yaml.contains("port: 8443"));
        assert!(yaml.contains("servername: edge.example.test"));
    }

    #[test]
    fn ssr_share_link_is_normalized_to_clash_yaml() {
        let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let body = format!(
            "ssr.example.test:8388:auth_sha1_v4:aes-256-cfb:http_simple:{}/?obfsparam={}&protoparam={}&remarks={}&group={}",
            b64("ssr-pass"),
            b64("cdn.example.test"),
            b64("proto-param"),
            b64("SSR Alias"),
            b64("HMeta")
        );
        let link = format!("ssr://{}", b64(&body));
        let yaml = normalize_profile_content(&link).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let ssr = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SSR Alias"))
            .unwrap();

        assert_eq!(get_string(ssr, "type").as_deref(), Some("ssr"));
        assert_eq!(
            get_string(ssr, "server").as_deref(),
            Some("ssr.example.test")
        );
        assert_eq!(get_u16(ssr, "port"), Some(8388));
        assert_eq!(get_string(ssr, "cipher").as_deref(), Some("aes-256-cfb"));
        assert_eq!(get_string(ssr, "password").as_deref(), Some("ssr-pass"));
        assert_eq!(get_string(ssr, "protocol").as_deref(), Some("auth_sha1_v4"));
        assert_eq!(
            get_string(ssr, "protocol-param").as_deref(),
            Some("proto-param")
        );
        assert_eq!(get_string(ssr, "obfs").as_deref(), Some("http_simple"));
        assert_eq!(
            get_string(ssr, "obfs-param").as_deref(),
            Some("cdn.example.test")
        );
        assert_eq!(get_string(ssr, "group").as_deref(), Some("HMeta"));
        assert_eq!(get_bool(ssr, "udp"), Some(true));
    }

    #[test]
    fn ssr_share_link_query_aliases_are_normalized() {
        let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let body = format!(
            "alias.example.test:9443:auth_chain_a:aes-128-gcm:tls1.2_ticket_auth:{}/?Remark={}&ProtocolParam={}&ObfsParam={}&GroupName={}",
            b64("alias-pass"),
            b64("SSR Alias Case"),
            b64("protocol-case"),
            b64("obfs-case.example.test"),
            b64("Alias Group")
        );
        let link = format!("ssr://{}", b64(&body));
        let yaml = normalize_profile_content(&link).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let ssr = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SSR Alias Case"))
            .unwrap();

        assert_eq!(get_string(ssr, "type").as_deref(), Some("ssr"));
        assert_eq!(
            get_string(ssr, "server").as_deref(),
            Some("alias.example.test")
        );
        assert_eq!(get_u16(ssr, "port"), Some(9443));
        assert_eq!(
            get_string(ssr, "protocol-param").as_deref(),
            Some("protocol-case")
        );
        assert_eq!(
            get_string(ssr, "obfs-param").as_deref(),
            Some("obfs-case.example.test")
        );
        assert_eq!(get_string(ssr, "group").as_deref(), Some("Alias Group"));
        assert_eq!(get_bool(ssr, "udp"), Some(true));
    }

    #[test]
    fn legacy_full_shadowsocks_share_link_is_normalized_to_clash_yaml() {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode("aes-128-gcm:legacy-pass@legacy-ss.example.test:8388");
        let link = format!("ss://{encoded}#Legacy%20SS");
        let yaml = normalize_profile_content(&link).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let ss = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("Legacy SS"))
            .unwrap();

        assert_eq!(get_string(ss, "type").as_deref(), Some("ss"));
        assert_eq!(
            get_string(ss, "server").as_deref(),
            Some("legacy-ss.example.test")
        );
        assert_eq!(get_u16(ss, "port"), Some(8388));
        assert_eq!(get_string(ss, "cipher").as_deref(), Some("aes-128-gcm"));
        assert_eq!(get_string(ss, "password").as_deref(), Some("legacy-pass"));
        assert_eq!(get_bool(ss, "udp"), Some(true));
    }

    #[test]
    fn share_link_schemes_are_case_insensitive() {
        let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let ssr_body = format!(
            "ssr-upper.example.test:8388:auth_sha1_v4:aes-256-cfb:http_simple:{}/?remarks={}",
            b64("ssr-pass"),
            b64("SSR Upper")
        );
        let ssr = b64(&ssr_body);
        let vmess = base64::engine::general_purpose::STANDARD.encode(
            r#"{"V":"2","PS":"VMess Upper","ADD":"vmess-upper.example.test","PORT":"443","ID":"00000000-0000-0000-0000-000000000009","AID":"0","SCY":"auto","NET":"tcp","TLS":"tls","SNI":"vmess-sni.example.test","ALPN":"h2,http/1.1","UDP":true,"FASTOPEN":true}"#,
        );
        let links = format!(
            "\
VLESS://00000000-0000-0000-0000-000000000001@vless-upper.example.test:443?security=TLS&sni=edge.example.test#VLESS%20Upper
TROJAN://secret@trojan-upper.example.test:443?sni=edge.example.test#Trojan%20Upper
SS://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-upper.example.test:8388#SS%20Upper
SSR://{ssr}
VMESS://{vmess}
HY2://hy-pass@hy2-upper.example.test:443?sni=hy2-sni.example.test#HY2%20Upper
TUIC://00000000-0000-0000-0000-000000000008:tuic-pass@tuic-upper.example.test:443?sni=tuic-sni.example.test#TUIC%20Upper
HTTPS://user:pass@https-upper.example.test:8443?allow_insecure=true#HTTPS%20Upper
SOCKS5://user:pass@socks-upper.example.test:1080?tls=true#SOCKS%20Upper
"
        );
        let yaml = normalize_profile_content(&links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let find = |name: &str| {
            proxies
                .iter()
                .filter_map(Value::as_mapping)
                .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
                .unwrap()
        };

        assert_eq!(
            get_string(find("VLESS Upper"), "type").as_deref(),
            Some("vless")
        );
        assert_eq!(
            get_string(find("Trojan Upper"), "type").as_deref(),
            Some("trojan")
        );
        assert_eq!(get_string(find("SS Upper"), "type").as_deref(), Some("ss"));
        assert_eq!(
            get_string(find("SSR Upper"), "type").as_deref(),
            Some("ssr")
        );
        assert_eq!(
            get_string(find("VMess Upper"), "type").as_deref(),
            Some("vmess")
        );
        assert_eq!(
            get_string(find("VMess Upper"), "server").as_deref(),
            Some("vmess-upper.example.test")
        );
        assert_eq!(get_u16(find("VMess Upper"), "port"), Some(443));
        assert_eq!(get_bool(find("VMess Upper"), "tls"), Some(true));
        assert_eq!(
            get_string(find("VMess Upper"), "servername").as_deref(),
            Some("vmess-sni.example.test")
        );
        assert_eq!(
            get_string_list(find("VMess Upper"), "alpn"),
            vec!["h2", "http/1.1"]
        );
        assert_eq!(get_bool(find("VMess Upper"), "udp"), Some(true));
        assert_eq!(get_bool(find("VMess Upper"), "tfo"), Some(true));
        assert_eq!(
            get_string(find("HY2 Upper"), "type").as_deref(),
            Some("hysteria2")
        );
        assert_eq!(
            get_string(find("TUIC Upper"), "type").as_deref(),
            Some("tuic")
        );
        assert_eq!(
            get_string(find("HTTPS Upper"), "type").as_deref(),
            Some("http")
        );
        assert_eq!(
            get_string(find("SOCKS Upper"), "type").as_deref(),
            Some("socks5")
        );
        assert_eq!(get_bool(find("HTTPS Upper"), "tls"), Some(true));
        assert_eq!(get_bool(find("SOCKS Upper"), "tls"), Some(true));
    }

    #[test]
    fn share_link_query_parameters_are_case_insensitive() {
        let links = "\
vless://00000000-0000-0000-0000-000000000001@case.example.test:443?TYPE=WS&SECURITY=Reality&SNI=edge.example.test&WsHost=cdn.example.test&WsPath=%2Fws&FP=chrome&ALPN=h2%2Chttp%2F1.1&PBK=pub&SID=abcd&SPX=%2Fspider&ED=2048&EH=Sec-WebSocket-Protocol&ALLOWINSECURE=TRUE#VLESS%20Case
trojan://secret@trojan-case.example.test:443?TYPE=GRPC&ServiceName=svc&MODE=gun&SERVERNAME=trojan-sni.example.test&Allow-Insecure=Allow#Trojan%20Case
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-case.example.test:8388?PLUGIN=v2ray-plugin&PLUGINOPTS=mode%3Dwebsocket%3Bhost%3Dedge.example.test%3Bpath%3D%2Fcase#SS%20Case
https://user:pass@http-case.example.test:8443?ALLOW_INSECURE=1#HTTPS%20Case
socks5://user:pass@socks-case.example.test:1080?TLS=TRUE&SKIP-CERT-VERIFY=TRUE#SOCKS%20Case
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let find = |name: &str| {
            proxies
                .iter()
                .filter_map(Value::as_mapping)
                .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
                .unwrap()
        };

        let vless = find("VLESS Case");
        assert_eq!(get_bool(vless, "tls"), Some(true));
        assert_eq!(get_bool(vless, "skip-cert-verify"), Some(true));
        assert_eq!(get_string(vless, "network").as_deref(), Some("ws"));
        assert_eq!(
            get_string(vless, "client-fingerprint").as_deref(),
            Some("chrome")
        );
        assert_eq!(get_string_list(vless, "alpn"), vec!["h2", "http/1.1"]);
        let ws_opts = vless
            .get(&value_key("ws-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(get_string(ws_opts, "path").as_deref(), Some("/ws"));
        assert_eq!(get_i64(ws_opts, "max-early-data"), Some(2048));
        assert_eq!(
            get_string(ws_opts, "early-data-header-name").as_deref(),
            Some("Sec-WebSocket-Protocol")
        );
        assert_eq!(
            get_string(
                ws_opts
                    .get(&value_key("headers"))
                    .and_then(Value::as_mapping)
                    .unwrap(),
                "Host"
            )
            .as_deref(),
            Some("cdn.example.test")
        );
        let reality_opts = vless
            .get(&value_key("reality-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(reality_opts, "public-key").as_deref(),
            Some("pub")
        );
        assert_eq!(
            get_string(reality_opts, "short-id").as_deref(),
            Some("abcd")
        );
        assert_eq!(
            get_string(reality_opts, "spider-x").as_deref(),
            Some("/spider")
        );

        let trojan = find("Trojan Case");
        assert_eq!(get_bool(trojan, "skip-cert-verify"), Some(true));
        assert_eq!(
            get_string(trojan, "sni").as_deref(),
            Some("trojan-sni.example.test")
        );
        let grpc_opts = trojan
            .get(&value_key("grpc-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(grpc_opts, "grpc-service-name").as_deref(),
            Some("svc")
        );
        assert_eq!(get_string(grpc_opts, "grpc-mode").as_deref(), Some("gun"));

        let ss = find("SS Case");
        assert_eq!(get_string(ss, "plugin").as_deref(), Some("v2ray-plugin"));
        assert_eq!(
            get_string(ss, "plugin-opts").as_deref(),
            Some("mode=websocket;host=edge.example.test;path=/case")
        );
        assert_eq!(get_bool(find("HTTPS Case"), "skip-cert-verify"), Some(true));
        assert_eq!(get_bool(find("SOCKS Case"), "tls"), Some(true));
        assert_eq!(get_bool(find("SOCKS Case"), "skip-cert-verify"), Some(true));
    }

    #[test]
    fn vless_tls_query_aliases_are_normalized() {
        let links = "\
vless://00000000-0000-0000-0000-000000000001@tls-query.example.test:443?tls=true&sni=tls-query.example.test#VLESS%20TLS%20Query
vless://00000000-0000-0000-0000-000000000002@enable-tls.example.test:443?enable-tls=1&sni=enable-tls.example.test#VLESS%20Enable%20TLS
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let find = |name: &str| {
            proxies
                .iter()
                .filter_map(Value::as_mapping)
                .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
                .unwrap()
        };

        assert_eq!(get_bool(find("VLESS TLS Query"), "tls"), Some(true));
        assert_eq!(
            get_string(find("VLESS TLS Query"), "servername").as_deref(),
            Some("tls-query.example.test")
        );
        assert_eq!(get_bool(find("VLESS Enable TLS"), "tls"), Some(true));
        assert_eq!(
            get_string(find("VLESS Enable TLS"), "servername").as_deref(),
            Some("enable-tls.example.test")
        );
    }

    #[test]
    fn share_link_query_names_are_used_when_fragment_is_missing() {
        let links = "\
vless://00000000-0000-0000-0000-000000000001@name-vless.example.test:443?security=tls&remarks=VLESS%20Query
trojan://secret@name-trojan.example.test:443?name=Trojan%20Query
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@name-ss.example.test:8388?ps=SS%20Query
hysteria2://hy-pass@name-hy2.example.test:443?alias=HY2%20Query
tuic://00000000-0000-0000-0000-000000000008:tuic-pass@name-tuic.example.test:443?node-name=TUIC%20Query
http://user:pass@name-http.example.test:8080?nodeName=HTTP%20Query
socks5://user:pass@name-socks.example.test:1080?remark=SOCKS%20Query#SOCKS%20Fragment
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let names = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .filter_map(|proxy| get_string(proxy, "name"))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "VLESS Query",
                "Trojan Query",
                "SS Query",
                "HY2 Query",
                "TUIC Query",
                "HTTP Query",
                "SOCKS Fragment"
            ]
        );
    }

    #[test]
    fn vmess_json_field_aliases_are_normalized() {
        let vmess = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            r#"{"name":"VMess Query Alias","server":"vmess-alias.example.test","port":"443","uuid":"00000000-0000-0000-0000-000000000011","alter_id":"0","security":"tls","network":"ws","wsHost":"alias-cdn.example.test","wsPath":"/alias","serverName":"alias-sni.example.test","allow_insecure":"true"}"#,
        );
        let yaml = normalize_profile_content(&format!("vmess://{vmess}")).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let vmess = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess Query Alias"))
            .unwrap();

        assert_eq!(get_string(vmess, "type").as_deref(), Some("vmess"));
        assert_eq!(
            get_string(vmess, "server").as_deref(),
            Some("vmess-alias.example.test")
        );
        assert_eq!(get_u16(vmess, "port"), Some(443));
        assert_eq!(
            get_string(vmess, "uuid").as_deref(),
            Some("00000000-0000-0000-0000-000000000011")
        );
        assert_eq!(get_i64(vmess, "alterId"), Some(0));
        assert_eq!(get_bool(vmess, "tls"), Some(true));
        assert!(get_string(vmess, "cipher").is_none());
        assert_eq!(get_bool(vmess, "skip-cert-verify"), Some(true));
        assert_eq!(get_string(vmess, "network").as_deref(), Some("ws"));
        assert_eq!(
            get_string(vmess, "servername").as_deref(),
            Some("alias-sni.example.test")
        );
        let ws_opts = vmess
            .get(&value_key("ws-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(get_string(ws_opts, "path").as_deref(), Some("/alias"));
        assert_eq!(
            get_string(
                ws_opts
                    .get(&value_key("headers"))
                    .and_then(Value::as_mapping)
                    .unwrap(),
                "Host"
            )
            .as_deref(),
            Some("alias-cdn.example.test")
        );
    }

    #[test]
    fn hysteria_family_and_tuic_share_links_are_normalized_to_clash_yaml() {
        let links = "\
hysteria://hy1-auth@hy1.example.test:443?protocol=udp&peer=hy1-sni.example.test&insecure=1&alpn=h3&obfs=obfs-pass&upmbps=20&downmbps=80&mport=10000-10010&recv-window-conn=1048576&recv-window=2097152&disable-mtu-discovery=true&fast-open=true#HY1%20Alias
hysteria2://hy-pass@hy2.example.test:443?sni=hy2-sni.example.test&insecure=1&alpn=h3,h2&obfs=salamander&obfs-password=obfs-pass&upmbps=50&downmbps=100&mport=20000-20010&recvWindowConn=3145728&recvWindow=4194304&disableMtuDiscovery=true&fastOpen=true#HY2%20Alias
tuic://00000000-0000-0000-0000-000000000008:tuic-pass@tuic.example.test:443?sni=tuic-sni.example.test&allow_insecure=true&alpn=h3&congestion_control=bbr&udp_relay_mode=native&disableSni=true&reduceRtt=true&requestTimeout=8s&heartbeatInterval=10s&maxUdpRelayPacketSize=1500&fastOpen=true#TUIC%20Alias
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();

        let hy1 = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY1 Alias"))
            .unwrap();
        assert_eq!(get_string(hy1, "type").as_deref(), Some("hysteria"));
        assert_eq!(
            get_string(hy1, "server").as_deref(),
            Some("hy1.example.test")
        );
        assert_eq!(get_u16(hy1, "port"), Some(443));
        assert_eq!(get_string(hy1, "auth-str").as_deref(), Some("hy1-auth"));
        assert_eq!(get_string(hy1, "protocol").as_deref(), Some("udp"));
        assert_eq!(
            get_string(hy1, "sni").as_deref(),
            Some("hy1-sni.example.test")
        );
        assert_eq!(get_bool(hy1, "skip-cert-verify"), Some(true));
        assert_eq!(get_string_list(hy1, "alpn"), vec!["h3"]);
        assert_eq!(get_string(hy1, "obfs").as_deref(), Some("obfs-pass"));
        assert_eq!(get_string(hy1, "up").as_deref(), Some("20"));
        assert_eq!(get_string(hy1, "down").as_deref(), Some("80"));
        assert_eq!(get_string(hy1, "ports").as_deref(), Some("10000-10010"));
        assert_eq!(
            get_string(hy1, "recv-window-conn").as_deref(),
            Some("1048576")
        );
        assert_eq!(get_string(hy1, "recv-window").as_deref(), Some("2097152"));
        assert_eq!(get_bool(hy1, "disable-mtu-discovery"), Some(true));
        assert_eq!(get_bool(hy1, "fast-open"), Some(true));

        let hy2 = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY2 Alias"))
            .unwrap();
        assert_eq!(get_string(hy2, "type").as_deref(), Some("hysteria2"));
        assert_eq!(
            get_string(hy2, "server").as_deref(),
            Some("hy2.example.test")
        );
        assert_eq!(get_u16(hy2, "port"), Some(443));
        assert_eq!(get_string(hy2, "password").as_deref(), Some("hy-pass"));
        assert_eq!(
            get_string(hy2, "sni").as_deref(),
            Some("hy2-sni.example.test")
        );
        assert_eq!(get_bool(hy2, "skip-cert-verify"), Some(true));
        assert_eq!(get_string_list(hy2, "alpn"), vec!["h3", "h2"]);
        assert_eq!(get_string(hy2, "obfs").as_deref(), Some("salamander"));
        assert_eq!(
            get_string(hy2, "obfs-password").as_deref(),
            Some("obfs-pass")
        );
        assert_eq!(get_string(hy2, "up").as_deref(), Some("50"));
        assert_eq!(get_string(hy2, "down").as_deref(), Some("100"));
        assert_eq!(get_string(hy2, "ports").as_deref(), Some("20000-20010"));
        assert_eq!(
            get_string(hy2, "recv-window-conn").as_deref(),
            Some("3145728")
        );
        assert_eq!(get_string(hy2, "recv-window").as_deref(), Some("4194304"));
        assert_eq!(get_bool(hy2, "disable-mtu-discovery"), Some(true));
        assert_eq!(get_bool(hy2, "fast-open"), Some(true));

        let tuic = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("TUIC Alias"))
            .unwrap();
        assert_eq!(get_string(tuic, "type").as_deref(), Some("tuic"));
        assert_eq!(
            get_string(tuic, "uuid").as_deref(),
            Some("00000000-0000-0000-0000-000000000008")
        );
        assert_eq!(get_string(tuic, "password").as_deref(), Some("tuic-pass"));
        assert_eq!(
            get_string(tuic, "sni").as_deref(),
            Some("tuic-sni.example.test")
        );
        assert_eq!(get_bool(tuic, "skip-cert-verify"), Some(true));
        assert_eq!(get_string_list(tuic, "alpn"), vec!["h3"]);
        assert_eq!(
            get_string(tuic, "congestion-controller").as_deref(),
            Some("bbr")
        );
        assert_eq!(
            get_string(tuic, "udp-relay-mode").as_deref(),
            Some("native")
        );
        assert_eq!(get_bool(tuic, "disable-sni"), Some(true));
        assert_eq!(get_bool(tuic, "reduce-rtt"), Some(true));
        assert_eq!(get_string(tuic, "request-timeout").as_deref(), Some("8s"));
        assert_eq!(
            get_string(tuic, "heartbeat-interval").as_deref(),
            Some("10s")
        );
        assert_eq!(
            get_string(tuic, "max-udp-relay-packet-size").as_deref(),
            Some("1500")
        );
        assert_eq!(get_bool(tuic, "fast-open"), Some(true));
    }

    #[test]
    fn hysteria2_query_auth_aliases_are_normalized() {
        let link = "hysteria2://hy2-query.example.test:443?auth=query-pass&peer=query-sni.example.test&remarks=HY2%20Query%20Auth";
        let yaml = normalize_profile_content(link).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let hy2 = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("HY2 Query Auth"))
            .unwrap();

        assert_eq!(get_string(hy2, "type").as_deref(), Some("hysteria2"));
        assert_eq!(
            get_string(hy2, "server").as_deref(),
            Some("hy2-query.example.test")
        );
        assert_eq!(get_u16(hy2, "port"), Some(443));
        assert_eq!(get_string(hy2, "password").as_deref(), Some("query-pass"));
        assert_eq!(
            get_string(hy2, "sni").as_deref(),
            Some("query-sni.example.test")
        );
    }

    #[test]
    fn share_link_transport_options_are_normalized_to_clash_yaml() {
        let vmess_h2 = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess H2","add":"vmess-h2.example.test","port":443,"id":"00000000-0000-0000-0000-000000000003","aid":0,"scy":"auto","net":"h2","host":"h2a.example.test,h2b.example.test","path":"/vmess-h2","tls":"tls","sni":"edge.example.test"}"#,
        );
        let vmess_httpupgrade = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess HTTPUpgrade","add":"vmess-up.example.test","port":"443","id":"00000000-0000-0000-0000-000000000004","aid":"0","scy":"auto","net":"httpupgrade","host":"upgrade.example.test","path":"/vmess-upgrade","tls":"tls","sni":"edge.example.test"}"#,
        );
        let vmess_ws_alias = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess WS Alias","add":"vmess-ws-alias.example.test","port":"443","id":"00000000-0000-0000-0000-000000000006","aid":"0","cipher":"auto","type":"ws","wsHost":"alias.example.test","wsPath":"/alias-ws","tls":"tls","serverName":"alias-sni.example.test","clientFingerprint":"chrome","allowInsecure":"allow"}"#,
        );
        let vmess_grpc_alias = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess GRPC Alias","add":"vmess-grpc-alias.example.test","port":"443","id":"00000000-0000-0000-0000-000000000007","aid":"0","scy":"auto","type":"grpc","grpc-service-name":"vmess-svc","grpc-mode":"gun","tls":"tls","sni":"grpc-sni.example.test","allow_insecure":true}"#,
        );
        let links = format!(
            "\
vless://00000000-0000-0000-0000-000000000001@example.test:443?type=ws&security=Reality&sni=edge.example.test&host=cdn.example.test&path=%2Fws&fp=chrome&alpn=h2%2Chttp%2F1.1&pbk=public-key&sid=abcd&spx=%2Ffingerprint&ed=2048&eh=Sec-WebSocket-Protocol&flow=xtls-rprx-vision&encryption=NONE#VLESS%20WS
trojan://secret@example.test:443?type=grpc&serviceName=svc&mode=gun&sni=edge.example.test&allowInsecure=1#Trojan%20GRPC
trojan://secret@example.test:443?type=grpc&grpc-service-name=alias-svc&grpc-mode=gun&serverName=trojan-alias.example.test&allow-insecure=allow#Trojan%20GRPC%20Alias
vless://00000000-0000-0000-0000-000000000002@example.test:443?type=h2&security=tls&sni=edge.example.test&host=h2a.example.test,h2b.example.test&path=%2Fh2#VLESS%20H2
vless://00000000-0000-0000-0000-000000000005@example.test:443?type=httpupgrade&security=tls&sni=edge.example.test&host=upgrade.example.test&path=%2Fupgrade#VLESS%20HTTPUpgrade
vless://00000000-0000-0000-0000-000000000006@example.test:443?network=ws&security=tls&serverName=alias-sni.example.test&wsHost=alias.example.test&wsPath=%2Falias-ws&client-fingerprint=chrome&allow-insecure=allow#VLESS%20WS%20Alias
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8388?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dcdn.example.test#SS%20OBFS
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8389?plugin=v2ray-plugin%3Bmode%3Dwebsocket%3Bhost%3Dedge.example.test%3Bpath%3D%2Fss-ws%3Btls#SS%20V2Ray
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8390?plugin=obfs-local&plugin-opts=obfs%3Dtls%3Bobfs-host%3Dexplicit.example.test#SS%20OBFS%20Explicit
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8391?plugin=v2ray-plugin&pluginOpts=mode%3Dwebsocket%3Bhost%3Dpluginopts.example.test%3Bpath%3D%2Fexplicit#SS%20V2Ray%20Explicit
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@example.test:8392?plugin=Simple-Obfs%3Bobfs%3Dhttp%3Bobfs-host%3Dcase.example.test#SS%20OBFS%20Case
vmess://{vmess_h2}
vmess://{vmess_httpupgrade}
vmess://{vmess_ws_alias}
vmess://{vmess_grpc_alias}
"
        );
        let yaml = normalize_profile_content(&links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let vless = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS WS"))
            .unwrap();
        assert_eq!(get_bool(vless, "tls"), Some(true));
        assert_eq!(get_string(vless, "network").as_deref(), Some("ws"));
        assert_eq!(
            get_string(vless, "client-fingerprint").as_deref(),
            Some("chrome")
        );
        assert_eq!(
            get_string(vless, "flow").as_deref(),
            Some("xtls-rprx-vision")
        );
        assert_eq!(get_string(vless, "encryption").as_deref(), Some("none"));
        let reality_opts = vless
            .get(&value_key("reality-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(reality_opts, "public-key").as_deref(),
            Some("public-key")
        );
        assert_eq!(
            get_string(reality_opts, "short-id").as_deref(),
            Some("abcd")
        );
        assert_eq!(
            get_string(reality_opts, "spider-x").as_deref(),
            Some("/fingerprint")
        );
        let vless_ws_opts = vless
            .get(&value_key("ws-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(get_string(vless_ws_opts, "path").as_deref(), Some("/ws"));
        assert_eq!(get_i64(vless_ws_opts, "max-early-data"), Some(2048));
        assert_eq!(
            get_string(vless_ws_opts, "early-data-header-name").as_deref(),
            Some("Sec-WebSocket-Protocol")
        );
        assert_eq!(get_string_list(vless, "alpn"), vec!["h2", "http/1.1"]);

        let trojan = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("Trojan GRPC"))
            .unwrap();
        let grpc_opts = trojan
            .get(&value_key("grpc-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(grpc_opts, "grpc-service-name").as_deref(),
            Some("svc")
        );
        assert_eq!(get_string(grpc_opts, "grpc-mode").as_deref(), Some("gun"));
        let trojan_alias = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("Trojan GRPC Alias"))
            .unwrap();
        assert_eq!(
            get_string(trojan_alias, "sni").as_deref(),
            Some("trojan-alias.example.test")
        );
        assert_eq!(get_bool(trojan_alias, "skip-cert-verify"), Some(true));
        let grpc_alias_opts = trojan_alias
            .get(&value_key("grpc-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(grpc_alias_opts, "grpc-service-name").as_deref(),
            Some("alias-svc")
        );
        assert_eq!(
            get_string(grpc_alias_opts, "grpc-mode").as_deref(),
            Some("gun")
        );

        let vless_h2 = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS H2"))
            .unwrap();
        assert_eq!(get_string(vless_h2, "network").as_deref(), Some("h2"));
        let h2_opts = vless_h2
            .get(&value_key("h2-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(get_string(h2_opts, "path").as_deref(), Some("/h2"));
        assert_eq!(
            get_string_list(h2_opts, "host"),
            vec!["h2a.example.test", "h2b.example.test"]
        );

        let vless_httpupgrade = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS HTTPUpgrade"))
            .unwrap();
        assert_eq!(
            get_string(vless_httpupgrade, "network").as_deref(),
            Some("httpupgrade")
        );
        let http_upgrade_opts = vless_httpupgrade
            .get(&value_key("http-upgrade-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(http_upgrade_opts, "path").as_deref(),
            Some("/upgrade")
        );
        assert_eq!(
            get_string(http_upgrade_opts, "host").as_deref(),
            Some("upgrade.example.test")
        );
        let vless_ws_alias = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VLESS WS Alias"))
            .unwrap();
        assert_eq!(
            get_string(vless_ws_alias, "servername").as_deref(),
            Some("alias-sni.example.test")
        );
        assert_eq!(
            get_string(vless_ws_alias, "client-fingerprint").as_deref(),
            Some("chrome")
        );
        assert_eq!(get_bool(vless_ws_alias, "skip-cert-verify"), Some(true));
        let vless_ws_alias_opts = vless_ws_alias
            .get(&value_key("ws-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(vless_ws_alias_opts, "path").as_deref(),
            Some("/alias-ws")
        );
        assert_eq!(
            get_string(
                vless_ws_alias_opts
                    .get(&value_key("headers"))
                    .and_then(Value::as_mapping)
                    .unwrap(),
                "Host"
            )
            .as_deref(),
            Some("alias.example.test")
        );

        let vmess_h2 = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess H2"))
            .unwrap();
        assert_eq!(get_string(vmess_h2, "network").as_deref(), Some("h2"));
        let vmess_h2_opts = vmess_h2
            .get(&value_key("h2-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(vmess_h2_opts, "path").as_deref(),
            Some("/vmess-h2")
        );
        assert_eq!(
            get_string_list(vmess_h2_opts, "host"),
            vec!["h2a.example.test", "h2b.example.test"]
        );

        let vmess_httpupgrade = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess HTTPUpgrade"))
            .unwrap();
        assert_eq!(
            get_string(vmess_httpupgrade, "network").as_deref(),
            Some("httpupgrade")
        );
        let vmess_http_upgrade_opts = vmess_httpupgrade
            .get(&value_key("http-upgrade-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(vmess_http_upgrade_opts, "path").as_deref(),
            Some("/vmess-upgrade")
        );
        assert_eq!(
            get_string(vmess_http_upgrade_opts, "host").as_deref(),
            Some("upgrade.example.test")
        );
        let vmess_ws_alias = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess WS Alias"))
            .unwrap();
        assert_eq!(get_string(vmess_ws_alias, "network").as_deref(), Some("ws"));
        assert_eq!(
            get_string(vmess_ws_alias, "servername").as_deref(),
            Some("alias-sni.example.test")
        );
        assert_eq!(
            get_string(vmess_ws_alias, "client-fingerprint").as_deref(),
            Some("chrome")
        );
        assert_eq!(get_bool(vmess_ws_alias, "skip-cert-verify"), Some(true));
        let vmess_ws_alias_opts = vmess_ws_alias
            .get(&value_key("ws-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(vmess_ws_alias_opts, "path").as_deref(),
            Some("/alias-ws")
        );
        assert_eq!(
            get_string(
                vmess_ws_alias_opts
                    .get(&value_key("headers"))
                    .and_then(Value::as_mapping)
                    .unwrap(),
                "Host"
            )
            .as_deref(),
            Some("alias.example.test")
        );
        let vmess_grpc_alias = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("VMess GRPC Alias"))
            .unwrap();
        assert_eq!(
            get_string(vmess_grpc_alias, "network").as_deref(),
            Some("grpc")
        );
        assert_eq!(
            get_string(vmess_grpc_alias, "servername").as_deref(),
            Some("grpc-sni.example.test")
        );
        assert_eq!(get_bool(vmess_grpc_alias, "skip-cert-verify"), Some(true));
        let vmess_grpc_alias_opts = vmess_grpc_alias
            .get(&value_key("grpc-opts"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            get_string(vmess_grpc_alias_opts, "grpc-service-name").as_deref(),
            Some("vmess-svc")
        );
        assert_eq!(
            get_string(vmess_grpc_alias_opts, "grpc-mode").as_deref(),
            Some("gun")
        );

        let ss_obfs = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS"))
            .unwrap();
        assert_eq!(get_string(ss_obfs, "plugin").as_deref(), Some("obfs"));
        assert_eq!(
            get_string(ss_obfs, "plugin-opts").as_deref(),
            Some("obfs=http;obfs-host=cdn.example.test")
        );

        let ss_v2ray = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS V2Ray"))
            .unwrap();
        assert_eq!(
            get_string(ss_v2ray, "plugin").as_deref(),
            Some("v2ray-plugin")
        );
        assert_eq!(
            get_string(ss_v2ray, "plugin-opts").as_deref(),
            Some("mode=websocket;host=edge.example.test;path=/ss-ws;tls")
        );

        let ss_obfs_explicit = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS Explicit"))
            .unwrap();
        assert_eq!(
            get_string(ss_obfs_explicit, "plugin").as_deref(),
            Some("obfs")
        );
        assert_eq!(
            get_string(ss_obfs_explicit, "plugin-opts").as_deref(),
            Some("obfs=tls;obfs-host=explicit.example.test")
        );

        let ss_obfs_case = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS OBFS Case"))
            .unwrap();
        assert_eq!(get_string(ss_obfs_case, "plugin").as_deref(), Some("obfs"));
        assert_eq!(
            get_string(ss_obfs_case, "plugin-opts").as_deref(),
            Some("obfs=http;obfs-host=case.example.test")
        );

        let ss_v2ray_explicit = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SS V2Ray Explicit"))
            .unwrap();
        assert_eq!(
            get_string(ss_v2ray_explicit, "plugin").as_deref(),
            Some("v2ray-plugin")
        );
        assert_eq!(
            get_string(ss_v2ray_explicit, "plugin-opts").as_deref(),
            Some("mode=websocket;host=pluginopts.example.test;path=/explicit")
        );
    }

    #[test]
    fn http_and_socks5_share_links_are_normalized_to_clash_yaml() {
        let links = "\
http://user:pass@example.test:8080?allow_insecure=true&headers=User-Agent%3DHMeta%3BProxy-Authorization%3DBearer%20token#HTTP%20Proxy
socks5://sock%20user:sock%20pass@example.test:1080?tls=true&skip-cert-verify=true#SOCKS5%20Proxy
";
        let yaml = normalize_profile_content(links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();

        let http = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("HTTP Proxy"))
            .unwrap();
        assert_eq!(get_string(http, "type").as_deref(), Some("http"));
        assert_eq!(get_string(http, "username").as_deref(), Some("user"));
        assert_eq!(get_string(http, "password").as_deref(), Some("pass"));
        assert_eq!(get_bool(http, "skip-cert-verify"), Some(true));
        let headers = http
            .get(&value_key("headers"))
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(get_string(headers, "User-Agent").as_deref(), Some("HMeta"));
        assert_eq!(
            get_string(headers, "Proxy-Authorization").as_deref(),
            Some("Bearer token")
        );

        let socks = proxies
            .iter()
            .filter_map(Value::as_mapping)
            .find(|proxy| get_string(proxy, "name").as_deref() == Some("SOCKS5 Proxy"))
            .unwrap();
        assert_eq!(get_string(socks, "type").as_deref(), Some("socks5"));
        assert_eq!(get_string(socks, "username").as_deref(), Some("sock user"));
        assert_eq!(get_string(socks, "password").as_deref(), Some("sock pass"));
        assert!(matches!(
            socks.get(&value_key("tls")),
            Some(Value::Bool(true))
        ));
        assert!(matches!(
            socks.get(&value_key("skip-cert-verify")),
            Some(Value::Bool(true))
        ));
    }

    #[test]
    fn share_links_preserve_udp_and_tfo_options() {
        let vmess_tfo = base64::engine::general_purpose::STANDARD.encode(
            r#"{"v":"2","ps":"VMess TFO","add":"vmess-tfo.example.test","port":443,"id":"00000000-0000-0000-0000-000000000010","aid":0,"scy":"auto","net":"tcp","udp":true,"fastOpen":true}"#,
        );
        let links = format!(
            "\
vless://00000000-0000-0000-0000-000000000001@vless-tfo.example.test:443?security=none&tfo=1#VLESS%20TFO
trojan://secret@trojan-tfo.example.test:443?fast-open=true#Trojan%20TFO
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@ss-tfo.example.test:8388?TFO=true#SS%20TFO
socks5://sock:sock-pass@socks-tfo.example.test:1080?udp=true&fastOpen=true#SOCKS5%20UDP%20TFO
vmess://{vmess_tfo}
"
        );
        let yaml = normalize_profile_content(&links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let find = |name: &str| {
            proxies
                .iter()
                .filter_map(Value::as_mapping)
                .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
                .unwrap()
        };

        for name in ["VLESS TFO", "Trojan TFO", "SS TFO", "VMess TFO"] {
            let proxy = find(name);
            assert_eq!(get_bool(proxy, "udp"), Some(true));
            assert_eq!(get_bool(proxy, "tfo"), Some(true));
        }

        let socks = find("SOCKS5 UDP TFO");
        assert_eq!(get_string(socks, "type").as_deref(), Some("socks5"));
        assert_eq!(get_bool(socks, "udp"), Some(true));
        assert_eq!(get_bool(socks, "tfo"), Some(true));
    }

    #[test]
    fn share_links_preserve_explicit_udp_disabled_options() {
        let b64 = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let ssr_body = format!(
            "ssr-udp-off.example.test:8388:origin:aes-256-cfb:plain:{}/?remarks={}&udp=false",
            b64("ssr-pass"),
            b64("SSR UDP Off")
        );
        let ss_cipher = base64::engine::general_purpose::STANDARD.encode("aes-256-gcm:password");
        let links = format!(
            "\
vless://00000000-0000-0000-0000-000000000001@vless-udp-off.example.test:443?udp=0#VLESS%20UDP%20Off
trojan://secret@trojan-udp-off.example.test:443?udp=false#Trojan%20UDP%20Off
ss://{ss_cipher}@ss-udp-off.example.test:8388?udp=off#SS%20UDP%20Off
ssr://{}
",
            b64(&ssr_body)
        );
        let yaml = normalize_profile_content(&links).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let proxies = root
            .get(&value_key("proxies"))
            .and_then(Value::as_sequence)
            .unwrap();
        let find = |name: &str| {
            proxies
                .iter()
                .filter_map(Value::as_mapping)
                .find(|proxy| get_string(proxy, "name").as_deref() == Some(name))
                .unwrap()
        };

        for name in [
            "VLESS UDP Off",
            "Trojan UDP Off",
            "SS UDP Off",
            "SSR UDP Off",
        ] {
            assert_eq!(get_bool(find(name), "udp"), Some(false));
        }
    }

    #[test]
    fn selected_proxy_choices_persist_with_profile() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("select")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Remote Demo",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();
        store
            .set_selected_proxy(&profile_id, "Proxy", "DIRECT")
            .unwrap();
        drop(store);

        let store = ProfileStore::open(root).unwrap();
        let selections = store.selected_proxies(&profile_id).unwrap();
        assert_eq!(selections.get("Proxy").map(String::as_str), Some("DIRECT"));
        assert_eq!(
            store.summaries()[0]
                .selected_proxies
                .get("Proxy")
                .map(String::as_str),
            Some("DIRECT")
        );
    }

    #[test]
    fn profile_content_can_restore_original_backup_after_edit() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("backup")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Editable",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
                None,
            )
            .unwrap();

        store
            .update_profile_content(
                &profile_id,
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,REJECT\n",
            )
            .unwrap();
        assert!(store
            .raw_yaml(&profile_id)
            .unwrap()
            .contains("MATCH,REJECT"));

        store.restore_profile_backup(&profile_id).unwrap();
        let restored = store.raw_yaml(&profile_id).unwrap();
        assert!(restored.contains("MATCH,DIRECT"));
        assert!(!restored.contains("MATCH,REJECT"));
    }

    #[test]
    fn profile_traffic_is_accumulated_in_summary() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("traffic")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Traffic",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();
        store.add_profile_traffic(&profile_id, 10, 20).unwrap();
        store.add_profile_traffic(&profile_id, 5, 7).unwrap();

        let summary = store
            .summaries()
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .unwrap();
        assert_eq!(summary.upload_bytes, 15);
        assert_eq!(summary.download_bytes, 27);
    }

    #[test]
    fn delete_profile_removes_runtime_and_profile_scoped_provider_cache() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("delete")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Providers",
                "local",
                r#"
mixed-port: 7890
proxy-providers:
  remote:
    type: http
    url: https://example.test/proxies.yaml
    interval: 3600
    path: ignored.yaml
    proxy: DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - remote
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
                None,
            )
            .unwrap();
        store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
            .unwrap();

        let runtime_path = root.join("runtime").join(format!("{profile_id}.yaml"));
        let provider_dir = root.join("providers/proxy").join(&profile_id);
        let provider_path = provider_dir.join("remote.yaml");
        let runtime_yaml = std::fs::read_to_string(&runtime_path).unwrap();
        let provider = store
            .providers_from_yaml(&runtime_yaml)
            .into_iter()
            .find(|provider| provider.name == "remote")
            .expect("remote provider");
        assert_eq!(
            provider.path.as_deref(),
            Some(provider_path.to_string_lossy().as_ref())
        );
        assert!(!provider.cache_exists);
        assert!(provider.cache_bytes.is_none());

        std::fs::write(&provider_path, "proxies: []\n").unwrap();
        let provider = store
            .providers_from_yaml(&runtime_yaml)
            .into_iter()
            .find(|provider| provider.name == "remote")
            .expect("remote provider");
        assert!(provider.cache_exists);
        assert_eq!(provider.cache_bytes, Some("proxies: []\n".len() as u64));
        assert!(provider.cache_updated_at.is_some());
        assert!(runtime_path.exists());
        assert!(provider_dir.exists());

        store.delete_profile(&profile_id).unwrap();
        assert!(!runtime_path.exists());
        assert!(!provider_dir.exists());
    }

    #[test]
    fn provider_cache_paths_are_profile_scoped_and_sanitized() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("provider-path")));
        let mut store = ProfileStore::open(root.clone()).unwrap();
        let profile_id = store
            .import_profile_content(
                "Providers",
                "local",
                r#"
mixed-port: 7890
proxy-providers:
  "../escape":
    type: http
    url: https://example.test/proxies.yaml
    interval: 3600
    filter: "HK|香港"
    exclude-filter: Premium
    path: ../../escape.yaml
    proxy: DIRECT
    health-check:
      enable: true
      url: https://cp.cloudflare.com/generate_204
      interval: "600"
rule-providers:
  remote-rules:
    type: http
    behavior: domain
    format: mrs
    url: https://example.test/rules.mrs
    interval: "7200"
    path: ../../rules.mrs
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    use:
      - "../escape"
    proxies:
      - DIRECT
rules:
  - MATCH,DIRECT
"#,
                None,
            )
            .unwrap();

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
            .unwrap();
        let provider = store
            .providers_from_yaml(&yaml)
            .into_iter()
            .find(|provider| provider.name == "../escape")
            .expect("escaped provider");
        let provider_path = provider.path.as_deref().expect("provider path");
        let expected_dir = root.join("providers/proxy").join(&profile_id);
        let provider_path = Path::new(provider_path);
        assert!(provider_path.starts_with(&expected_dir));
        assert_eq!(provider_path.parent(), Some(expected_dir.as_path()));
        assert!(!provider_path.to_string_lossy().contains("../escape.yaml"));
        assert_eq!(provider.interval_seconds, Some(3600));
        assert_eq!(provider.filter.as_deref(), Some("HK|香港"));
        assert_eq!(provider.exclude_filter.as_deref(), Some("Premium"));
        assert!(provider.health_check_enabled);
        assert_eq!(
            provider.health_check_url.as_deref(),
            Some("https://cp.cloudflare.com/generate_204")
        );
        assert_eq!(provider.health_check_interval_seconds, Some(600));

        let rule_provider = store
            .providers_from_yaml(&yaml)
            .into_iter()
            .find(|provider| provider.name == "remote-rules")
            .expect("rule provider");
        let rule_provider_path = rule_provider.path.as_deref().expect("rule provider path");
        let expected_rule_dir = root.join("providers/rule").join(&profile_id);
        let rule_provider_path = Path::new(rule_provider_path);
        assert!(rule_provider_path.starts_with(&expected_rule_dir));
        assert_eq!(
            rule_provider_path.parent(),
            Some(expected_rule_dir.as_path())
        );
        assert_eq!(rule_provider.provider_type, "rule");
        assert_eq!(rule_provider.behavior.as_deref(), Some("domain"));
        assert_eq!(rule_provider.format.as_deref(), Some("mrs"));
        assert_eq!(rule_provider.interval_seconds, Some(7200));
    }

    #[test]
    fn inline_rule_provider_keeps_payload_without_runtime_cache_fields() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("inline-rule-provider")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Inline Rules",
                "local",
                r#"
mixed-port: 7890
rule-providers:
  InlineRules:
    type: inline
    behavior: classical
    interval: 3600
    path: ../../inline.yaml
    payload:
      - DOMAIN-SUFFIX,inline.example,DIRECT
proxies: []
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
rules:
  - RULE-SET,InlineRules,DIRECT
  - MATCH,DIRECT
"#,
                None,
            )
            .unwrap();

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &VpnOptions::default())
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().unwrap();
        let provider = root
            .get(&value_key("rule-providers"))
            .and_then(Value::as_mapping)
            .and_then(|providers| providers.get(&value_key("InlineRules")))
            .and_then(Value::as_mapping)
            .expect("inline rule provider");

        assert_eq!(get_string(provider, "type").as_deref(), Some("inline"));
        assert!(provider.get(&value_key("path")).is_none());
        assert!(provider.get(&value_key("interval")).is_none());
        assert_eq!(
            get_string(provider, "behavior").as_deref(),
            Some("classical")
        );
        assert_eq!(
            provider
                .get(&value_key("payload"))
                .and_then(Value::as_sequence)
                .and_then(|payload| payload.first())
                .and_then(Value::as_str),
            Some("DOMAIN-SUFFIX,inline.example,DIRECT")
        );
    }

    #[test]
    fn geodata_files_report_app_private_resource_state() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("geodata-state")));
        let store = ProfileStore::open(root.clone()).unwrap();
        let files = store.geodata_files();
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|file| file.path.ends_with("Country.mmdb")));
        assert!(files.iter().any(|file| file.path.ends_with("geosite.dat")));
        assert!(files.iter().all(|file| !file.exists));

        std::fs::write(root.join("geodata").join("geosite.dat"), b"dat").unwrap();
        let files = store.geodata_files();
        let geosite = files
            .iter()
            .find(|file| file.path.ends_with("geosite.dat"))
            .expect("geosite summary");
        assert!(geosite.exists);
        assert_eq!(geosite.bytes, Some(3));
        assert!(geosite.updated_at.is_some());
    }

    #[test]
    fn derives_vpn_options_from_profile_yaml() {
        let options = vpn_options_from_yaml(
            r#"
ipv6: true
dns:
  nameserver:
    - 9.9.9.9
  fallback:
    - https://dns.google/dns-query
  nameserver-policy:
    geosite:cn:
      - https://dns.alidns.com/dns-query
tun:
  mtu: 1400
  stack: gvisor
  inet4-address:
    - 198.18.0.1/16
  route-address:
    - 10.0.0.0/8
hmeta:
  system-proxy: true
  allow-bypass: true
  per-app-mode: bypass
  blocked-applications:
    - com.example.video
"#,
        )
        .unwrap();
        assert_eq!(options.mtu, 1400);
        assert_eq!(options.stack, "gvisor");
        assert_eq!(options.dns_servers, vec!["9.9.9.9"]);
        assert_eq!(options.dns_fallbacks, vec!["https://dns.google/dns-query"]);
        assert_eq!(
            options
                .dns_nameserver_policy
                .get("geosite:cn")
                .cloned()
                .unwrap_or_default(),
            vec!["https://dns.alidns.com/dns-query"]
        );
        assert_eq!(options.dns_addresses, vec![MEOW_V4_ROUTER]);
        assert!(options.addresses.contains(&"198.18.0.1/16".to_owned()));
        assert!(options.addresses.contains(&MEOW_V6_CLIENT.to_owned()));
        assert_eq!(
            options.routes,
            vec!["10.0.0.0/8".to_owned(), "::/0".to_owned()]
        );
        assert!(options.system_proxy);
        assert!(options.allow_bypass);
        assert_eq!(options.per_app_mode, PerAppMode::Bypass);
        assert_eq!(options.blocked_applications, vec!["com.example.video"]);
    }

    #[test]
    fn runtime_yaml_uses_default_china_dns_split_policy() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-defaults")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS Defaults",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        assert_eq!(
            options.dns_servers,
            vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
        );
        assert_eq!(
            options.dns_fallbacks,
            vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
        );
        assert_eq!(
            options
                .dns_nameserver_policy
                .get("geosite:cn")
                .cloned()
                .unwrap_or_default(),
            vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
        );
        assert_eq!(
            options
                .dns_nameserver_policy
                .get("geosite:geolocation-!cn")
                .cloned()
                .unwrap_or_default(),
            vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
        );

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let dns = value
            .as_mapping()
            .and_then(|root| root.get(&value_key("dns")))
            .and_then(Value::as_mapping)
            .expect("dns");
        assert_eq!(
            get_string_list(dns, "default-nameserver"),
            vec!["223.5.5.5".to_owned(), "119.29.29.29".to_owned()]
        );
        assert_eq!(
            get_string_list(dns, "fallback"),
            vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()]
        );
        let policy = get_string_list_map(dns, "nameserver-policy");
        assert!(policy.contains_key("geosite:cn"));
        assert!(policy.contains_key("geosite:geolocation-!cn"));
    }

    #[test]
    fn derives_per_app_proxy_mode_from_trusted_applications() {
        let options = vpn_options_from_yaml(
            r#"
hmeta:
  trusted-applications:
    - com.example.browser
"#,
        )
        .unwrap();
        assert_eq!(options.per_app_mode, PerAppMode::Proxy);
        assert_eq!(options.trusted_applications, vec!["com.example.browser"]);
    }

    #[test]
    fn updates_per_app_config_in_profile_yaml() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("per-app")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "Per App",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();

        store
            .set_profile_per_app_config(
                &profile_id,
                PerAppMode::Proxy,
                vec![
                    "com.example.browser".to_owned(),
                    "com.example.browser".to_owned(),
                    " ".to_owned(),
                ],
                vec!["com.example.video".to_owned()],
            )
            .unwrap();

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        assert_eq!(options.per_app_mode, PerAppMode::Proxy);
        assert_eq!(options.trusted_applications, vec!["com.example.browser"]);
        assert_eq!(options.blocked_applications, vec!["com.example.video"]);
    }

    #[test]
    fn updates_vpn_config_in_profile_yaml() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("vpn")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "VPN",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();

        store
            .set_profile_vpn_config(&profile_id, true, false, true, " gvisor ".to_owned())
            .unwrap();

        let raw_yaml = store.raw_yaml(&profile_id).unwrap();
        let value: Value = serde_yaml::from_str(&raw_yaml).unwrap();
        let root = value.as_mapping().expect("root");
        let hmeta = root
            .get(&value_key("hmeta"))
            .and_then(Value::as_mapping)
            .expect("hmeta");
        let tun = root
            .get(&value_key("tun"))
            .and_then(Value::as_mapping)
            .expect("tun");
        assert_eq!(get_bool(hmeta, "system-proxy"), Some(true));
        assert_eq!(get_bool(hmeta, "allow-bypass"), Some(true));
        assert_eq!(get_string(tun, "stack"), Some("gvisor".to_owned()));
        assert_eq!(get_bool(tun, "dns-hijack"), Some(false));

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        assert!(options.system_proxy);
        assert!(!options.dns_hijacking);
        assert!(options.allow_bypass);
        assert_eq!(options.stack, "gvisor");
    }

    #[test]
    fn updates_dns_config_in_profile_yaml() {
        let root = std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();

        store
            .set_profile_dns_config(
                &profile_id,
                vec![
                    "https://dns.alidns.com/dns-query".to_owned(),
                    "https://dns.alidns.com/dns-query".to_owned(),
                    "1.1.1.1".to_owned(),
                    " ".to_owned(),
                ],
                vec![
                    "https://dns.google/dns-query".to_owned(),
                    "https://dns.google/dns-query".to_owned(),
                ],
                BTreeMap::from([(
                    "geosite:cn".to_owned(),
                    vec![
                        "https://dns.alidns.com/dns-query".to_owned(),
                        " ".to_owned(),
                    ],
                )]),
            )
            .unwrap();

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        assert_eq!(
            options.dns_servers,
            vec![
                "https://dns.alidns.com/dns-query".to_owned(),
                "1.1.1.1".to_owned()
            ]
        );
        assert_eq!(
            options.dns_fallbacks,
            vec!["https://dns.google/dns-query".to_owned()]
        );
        assert_eq!(
            options
                .dns_nameserver_policy
                .get("geosite:cn")
                .cloned()
                .unwrap_or_default(),
            vec!["https://dns.alidns.com/dns-query".to_owned()]
        );
    }

    #[test]
    fn runtime_yaml_adds_default_nameserver_for_encrypted_dns_hostnames() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-bootstrap")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS Bootstrap",
                "local",
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                None,
            )
            .unwrap();
        let options = VpnOptions {
            dns_servers: vec!["https://dns.alidns.com/dns-query".to_owned()],
            ..VpnOptions::default()
        };

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let dns = value
            .as_mapping()
            .and_then(|root| root.get(&value_key("dns")))
            .and_then(Value::as_mapping)
            .expect("dns");

        assert_eq!(
            get_string_list(dns, "default-nameserver"),
            vec![
                "223.5.5.5".to_owned(),
                "119.29.29.29".to_owned(),
                "1.1.1.1".to_owned(),
                "8.8.8.8".to_owned()
            ]
        );
    }

    #[test]
    fn runtime_yaml_replaces_subscription_default_nameserver() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-managed")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS Managed",
                "local",
                r#"
mixed-port: 7890
dns:
  default-nameserver:
    - 127.0.0.1
  nameserver:
    - https://dns.alidns.com/dns-query
proxies: []
proxy-groups: []
rules: []
"#,
                None,
            )
            .unwrap();

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let dns = value
            .as_mapping()
            .and_then(|root| root.get(&value_key("dns")))
            .and_then(Value::as_mapping)
            .expect("dns");

        assert_eq!(
            get_string_list(dns, "default-nameserver"),
            vec![
                "223.5.5.5".to_owned(),
                "119.29.29.29".to_owned(),
                "1.1.1.1".to_owned(),
                "8.8.8.8".to_owned()
            ]
        );
    }

    #[test]
    fn runtime_yaml_disables_subscription_system_hosts_dns_lookup() {
        let root = std::env::temp_dir().join(format!(
            "hmeta-profile-test-{}",
            next_id("dns-system-hosts")
        ));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS System Hosts",
                "local",
                r#"
mixed-port: 7890
dns:
  use-hosts: true
  use-system-hosts: true
  nameserver:
    - 223.5.5.5
hosts:
  example.test: 203.0.113.10
proxies: []
proxy-groups: []
rules: []
"#,
                None,
            )
            .unwrap();

        let options = store.vpn_options_for_profile(&profile_id).unwrap();
        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let root = value.as_mapping().expect("root");
        let dns = root
            .get(&value_key("dns"))
            .and_then(Value::as_mapping)
            .expect("dns");

        assert_eq!(get_bool(dns, "use-hosts"), Some(true));
        assert_eq!(get_bool(dns, "use-system-hosts"), Some(false));
        assert!(root.get(&value_key("hosts")).is_some());
    }

    #[test]
    fn runtime_yaml_removes_subscription_fallback_and_policy_when_unset() {
        let root =
            std::env::temp_dir().join(format!("hmeta-profile-test-{}", next_id("dns-clear")));
        let mut store = ProfileStore::open(root).unwrap();
        let profile_id = store
            .import_profile_content(
                "DNS Clear",
                "local",
                r#"
mixed-port: 7890
dns:
  nameserver:
    - 9.9.9.9
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - '*.lan'
  fallback:
    - 8.8.8.8
  fallback-filter:
    geoip: true
    geoip-code: CN
  nameserver-policy:
    geosite:cn:
      - 223.5.5.5
proxies: []
proxy-groups: []
rules: []
"#,
                None,
            )
            .unwrap();
        let options = VpnOptions {
            dns_servers: vec!["1.1.1.1".to_owned()],
            dns_fallbacks: Vec::new(),
            dns_nameserver_policy: BTreeMap::new(),
            ..VpnOptions::default()
        };

        let yaml = store
            .build_runtime_yaml(&profile_id, RuntimeMode::Rule, &options)
            .unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        let dns = value
            .as_mapping()
            .and_then(|root| root.get(&value_key("dns")))
            .and_then(Value::as_mapping)
            .expect("dns");

        assert_eq!(get_string_list(dns, "nameserver"), vec!["1.1.1.1"]);
        assert!(dns.get(&value_key("fallback")).is_none());
        assert!(dns.get(&value_key("nameserver-policy")).is_none());
        assert!(dns.get(&value_key("enhanced-mode")).is_none());
        assert!(dns.get(&value_key("fake-ip-range")).is_none());
        assert!(dns.get(&value_key("fake-ip-filter")).is_none());
        assert!(dns.get(&value_key("fallback-filter")).is_none());
    }
}
