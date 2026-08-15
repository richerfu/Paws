use base64::Engine;
use ipnet::IpNet;
use paws_model::{
    ControllerAccessConfig, GeodataFileSummary, ManualRuleMatchKind, ManualRuleMutation,
    ManualRuleMutationKind, ManualRuleSpec, NetworkPortConfig, PawsError, ProfileSummary,
    ProviderSummary, RuleSummary, RuntimeMode, SubscriptionMetadata, SubscriptionUserInfo,
    VpnOptions, VpnStack, DEFAULT_CHINA_DNS_SERVERS, DEFAULT_GLOBAL_DNS_FALLBACKS,
    DEFAULT_MIXED_PORT,
};
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

mod rules;
mod runtime_config;
mod subscription;

use rules::*;
pub use runtime_config::*;
pub use subscription::*;

const STORE_VERSION: u32 = 1;
const MEOW_V4_CLIENT: &str = "172.19.0.1/30";
const MEOW_V4_ROUTER: &str = "172.19.0.2";
const MEOW_V6_CLIENT: &str = "fdfe:dcba:9876::1/126";
pub const APP_MIXED_PROXY_PORT: u16 = DEFAULT_MIXED_PORT;
const DEFAULT_PROXY_SUBSCRIPTION_RULES: [&str; 3] =
    ["GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT", "MATCH,Proxy"];
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
    pub fn open_default() -> Result<Self, PawsError> {
        let root = std::env::var("PAWS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_store_root());
        Self::open(root)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PawsError> {
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
            .map_err(|err| PawsError::InvalidJson(err.to_string()))?;
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
    ) -> Result<String, PawsError> {
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
    ) -> Result<String, PawsError> {
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
    ) -> Result<String, PawsError> {
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
    ) -> Result<(), PawsError> {
        self.replace_profile_content_with_subscription_info(profile_id, raw_yaml, None)
    }

    pub fn replace_profile_content_with_subscription_info(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
        subscription_user_info: Option<SubscriptionUserInfo>,
    ) -> Result<(), PawsError> {
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
    ) -> Result<(), PawsError> {
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
                .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
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
    ) -> Result<(), PawsError> {
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
        profile.last_refresh_at = Some(now_string());
        profile.last_refresh_error = Some(error.into());
        self.save()
    }

    pub fn update_profile_content(
        &mut self,
        profile_id: &str,
        raw_yaml: impl Into<String>,
    ) -> Result<(), PawsError> {
        let raw_yaml = normalize_profile_content(&raw_yaml.into())?;
        let raw_path = {
            let profile = self
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
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
    ) -> Result<(), PawsError> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(PawsError::Core("profile name cannot be empty".to_owned()));
        }
        let subscription_url = subscription_url.into().trim().to_owned();
        let parsed = Url::parse(&subscription_url).map_err(subscription_error)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(PawsError::Core(
                "subscription URL must use http or https".to_owned(),
            ));
        }
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
        profile.name = name;
        profile.source = subscription_url.clone();
        profile.subscription_url = Some(subscription_url);
        self.save()
    }

    pub fn restore_profile_backup(&mut self, profile_id: &str) -> Result<(), PawsError> {
        let (raw_path, backup_path) = {
            let profile = self
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
            let backup_path = profile
                .yaml_backup_path
                .clone()
                .ok_or_else(|| PawsError::Core(format!("profile {profile_id} has no backup")))?;
            profile.updated_at = Some(now_string());
            (profile.raw_yaml_path.clone(), backup_path)
        };
        let backup = fs::read_to_string(self.root.join(backup_path)).map_err(io_error)?;
        fs::write(self.root.join(raw_path), backup).map_err(io_error)?;
        self.save()
    }

    pub fn delete_profile(&mut self, profile_id: &str) -> Result<(), PawsError> {
        let profile = self
            .profiles
            .remove(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
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
    ) -> Result<(), PawsError> {
        if upload_delta == 0 && download_delta == 0 {
            return Ok(());
        }
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
        profile.upload_bytes = profile.upload_bytes.saturating_add(upload_delta);
        profile.download_bytes = profile.download_bytes.saturating_add(download_delta);
        self.save()
    }

    pub fn set_profile_dns_servers(
        &mut self,
        profile_id: &str,
        dns_servers: Vec<String>,
    ) -> Result<(), PawsError> {
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
    ) -> Result<(), PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(PawsError::Core(
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
            serde_yaml::to_string(&value).map_err(|err| PawsError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)
    }

    pub fn set_profile_vpn_config(
        &mut self,
        profile_id: &str,
        system_proxy: bool,
        dns_hijacking: bool,
        allow_bypass: bool,
        stack: String,
    ) -> Result<(), PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(PawsError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let paws_key = value_key("paws");
        let mut paws = root
            .remove(&paws_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        put_bool(&mut paws, "system-proxy", system_proxy);
        put_bool(&mut paws, "allow-bypass", allow_bypass);
        root.insert(paws_key, Value::Mapping(paws));

        let tun_key = value_key("tun");
        let mut tun = root
            .remove(&tun_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        let stack = VpnStack::try_from(stack.as_str())?.as_str().to_owned();
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
            serde_yaml::to_string(&value).map_err(|err| PawsError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)
    }

    pub fn set_profile_network_config(
        &mut self,
        profile_id: &str,
        network_ports: NetworkPortConfig,
        allow_lan: bool,
    ) -> Result<(NetworkPortConfig, ControllerAccessConfig), PawsError> {
        let network_ports = network_ports.validate()?;
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(PawsError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let paws_key = value_key("paws");
        let mut paws = root
            .remove(&paws_key)
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        let mut secret = get_string(&paws, "controller-secret")
            .or_else(|| get_string(&paws, "controllerSecret"))
            .filter(|secret| controller_secret_is_valid(secret));
        if allow_lan && secret.is_none() {
            secret = Some(ControllerSecretGenerator::generate()?);
        }
        put_i64(&mut paws, "mixed-port", i64::from(network_ports.mixed_port));
        put_i64(
            &mut paws,
            "controller-port",
            i64::from(network_ports.controller_port),
        );
        put_bool(&mut paws, "controller-allow-lan", allow_lan);
        if let Some(secret) = secret.as_deref() {
            put_string(&mut paws, "controller-secret", secret);
        }
        root.insert(paws_key, Value::Mapping(paws));

        let raw_yaml =
            serde_yaml::to_string(&value).map_err(|err| PawsError::Core(err.to_string()))?;
        self.update_profile_content(profile_id, raw_yaml)?;
        Ok((network_ports, ControllerAccessConfig { allow_lan, secret }))
    }

    pub fn set_active(&mut self, id: &str) -> Result<(), PawsError> {
        if !self.profiles.contains_key(id) {
            return Err(PawsError::ProfileNotFound(id.to_owned()));
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

    pub fn profile(&self, profile_id: &str) -> Result<&ProfileDocument, PawsError> {
        self.profiles
            .get(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))
    }

    pub fn selected_proxies(
        &self,
        profile_id: &str,
    ) -> Result<BTreeMap<String, String>, PawsError> {
        Ok(self.profile(profile_id)?.selected_proxies.clone())
    }

    pub fn set_selected_proxy(
        &mut self,
        profile_id: &str,
        group_name: impl Into<String>,
        proxy_name: impl Into<String>,
    ) -> Result<(), PawsError> {
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| PawsError::ProfileNotFound(profile_id.to_owned()))?;
        profile
            .selected_proxies
            .insert(group_name.into(), proxy_name.into());
        self.save()
    }

    pub fn raw_yaml(&self, profile_id: &str) -> Result<String, PawsError> {
        let profile = self.profile(profile_id)?;
        fs::read_to_string(self.root.join(&profile.raw_yaml_path)).map_err(io_error)
    }

    pub fn active_raw_yaml(&self) -> Result<String, PawsError> {
        let id = self
            .active_profile
            .as_deref()
            .ok_or_else(|| PawsError::ProfileNotFound("<active>".to_owned()))?;
        self.raw_yaml(id)
    }

    pub fn runtime_yaml_path(&self, profile_id: &str) -> PathBuf {
        self.root.join("runtime").join(format!("{profile_id}.yaml"))
    }

    pub fn vpn_options_for_profile(&self, profile_id: &str) -> Result<VpnOptions, PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        vpn_options_from_yaml(&raw_yaml)
    }

    pub fn controller_access_for_profile(
        &self,
        profile_id: &str,
    ) -> Result<ControllerAccessConfig, PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        controller_access_from_yaml(&raw_yaml)
    }

    pub fn network_ports_for_profile(
        &self,
        profile_id: &str,
    ) -> Result<NetworkPortConfig, PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        network_ports_from_yaml(&raw_yaml)
    }

    pub fn active_vpn_options(&self) -> Result<VpnOptions, PawsError> {
        let id = self
            .active_profile
            .as_deref()
            .ok_or_else(|| PawsError::ProfileNotFound("<active>".to_owned()))?;
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
    ) -> Result<Vec<String>, PawsError> {
        self.profile(profile_id)?;
        let source = source.into();
        let imported_rules = parse_imported_rule_lines(rules_text)?;
        let mut next_order = self
            .rules
            .values()
            .filter(|rule| rule.profile_id == profile_id)
            .map(|rule| rule.order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut ids = Vec::new();
        for line in imported_rules {
            let id = next_id("rule");
            self.rules.insert(
                id.clone(),
                RuleDocument {
                    id: id.clone(),
                    profile_id: profile_id.to_owned(),
                    line,
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
    ) -> Result<ManualRuleMutation, PawsError> {
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
                .ok_or_else(|| PawsError::RuleNotFound(existing.id.clone()))?;
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
    ) -> Result<(), PawsError> {
        let rule = self
            .rules
            .get_mut(rule_id)
            .ok_or_else(|| PawsError::RuleNotFound(rule_id.to_owned()))?;
        if rule.profile_id != profile_id {
            return Err(PawsError::RuleNotFound(rule_id.to_owned()));
        }
        rule.enabled = enabled;
        self.save()
    }

    pub fn reorder_rules(
        &mut self,
        profile_id: &str,
        ordered_rule_ids: &[String],
    ) -> Result<(), PawsError> {
        for (index, rule_id) in ordered_rule_ids.iter().enumerate() {
            let rule = self
                .rules
                .get_mut(rule_id)
                .ok_or_else(|| PawsError::RuleNotFound(rule_id.clone()))?;
            if rule.profile_id != profile_id {
                return Err(PawsError::RuleNotFound(rule_id.clone()));
            }
            rule.order = index as u32;
        }
        self.save()
    }

    pub fn delete_rule(&mut self, rule_id: &str) -> Result<(), PawsError> {
        self.rules
            .remove(rule_id)
            .ok_or_else(|| PawsError::RuleNotFound(rule_id.to_owned()))?;
        self.save()
    }

    pub fn build_runtime_yaml(
        &self,
        profile_id: &str,
        mode: RuntimeMode,
        vpn_options: &VpnOptions,
    ) -> Result<String, PawsError> {
        let yaml = self.render_runtime_yaml(profile_id, mode, vpn_options)?;
        self.write_runtime_yaml(profile_id, &yaml)?;
        Ok(yaml)
    }

    pub fn render_runtime_yaml(
        &self,
        profile_id: &str,
        mode: RuntimeMode,
        vpn_options: &VpnOptions,
    ) -> Result<String, PawsError> {
        let raw_yaml = self.raw_yaml(profile_id)?;
        let mut value: Value =
            serde_yaml::from_str(&raw_yaml).map_err(|err| PawsError::Core(err.to_string()))?;
        let Some(root) = value.as_mapping_mut() else {
            return Err(PawsError::Core(
                "profile root must be a YAML map".to_owned(),
            ));
        };

        let controller_access = controller_access_from_mapping(root);
        let network_ports = network_ports_from_mapping(root);
        sanitize_app_managed_config(root);
        put_string(root, "mode", mode.as_str());
        put_bool(root, "ipv6", vpn_options.ipv6);
        put_string(root, "log-level", "info");
        put_i64(root, "mixed-port", i64::from(network_ports.mixed_port));
        patch_controller_access(root, &controller_access, network_ports.controller_port)?;
        patch_geox_url(root);
        patch_geodata_paths(root, &self.root)?;
        upgrade_legacy_generated_subscription_rules(root);
        prune_unavailable_default_subscription_rules(root, &self.root);
        patch_dns(root, vpn_options);
        patch_tun(root, vpn_options);
        rewrite_provider_paths(root, &self.root, profile_id)?;
        merge_rules(root, self.enabled_rule_lines(profile_id));

        serde_yaml::to_string(&value).map_err(|err| PawsError::Core(err.to_string()))
    }

    pub fn write_runtime_yaml(&self, profile_id: &str, yaml: &str) -> Result<(), PawsError> {
        fs::write(
            self.root.join("runtime").join(format!("{profile_id}.yaml")),
            yaml,
        )
        .map_err(io_error)
    }

    pub fn persist(&self) -> Result<(), PawsError> {
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

    fn save(&self) -> Result<(), PawsError> {
        let index = StoreIndex {
            version: STORE_VERSION,
            active_profile: self.active_profile.clone(),
            profiles: self.profiles.values().cloned().collect(),
            rules: self.rules.values().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&index)
            .map_err(|err| PawsError::InvalidJson(err.to_string()))?;
        fs::write(self.root.join("profiles.json"), json).map_err(io_error)
    }

    #[cfg(test)]
    fn seed_default(&mut self) -> Result<(), PawsError> {
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

fn default_store_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(".paws")
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

fn io_error(error: std::io::Error) -> PawsError {
    PawsError::Io(error.to_string())
}

#[cfg(test)]
mod tests;
