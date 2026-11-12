//! Backend-neutral configuration declarations. No proxy, resolver, health
//! worker or routable tunnel is constructed to display an offline profile.
use super::*;
use paws_model::{ProviderProxySummary, ProxyGroup, ProxyItem};
use regex::Regex;
use std::io::Read;

const MAX_PROVIDER_BYTES: u64 = 8 * 1024 * 1024;

pub struct ProfileSourceMetadata {
    pub proxy_groups: Vec<ProxyGroup>,
    pub providers: Vec<ProviderSummary>,
    pub rule_lines: Vec<String>,
    /// Missing/corrupt cache is not a successful live provider observation.
    pub unverified_providers: Vec<String>,
}

struct SourceDefinitions {
    root: Mapping,
    leaves: Vec<(String, String)>,
    groups: Vec<Mapping>,
    rules: Vec<String>,
}
#[derive(Default)]
struct SourceFilter {
    include: Option<Regex>,
    exclude: Option<Regex>,
    types: Vec<String>,
}
impl SourceFilter {
    fn parse(map: &Mapping) -> Result<Self, PawsError> {
        let pattern = |key: &str| -> Result<Option<Regex>, PawsError> {
            match map.get(value_key(key)).filter(|value| !value.is_null()) {
                None => Ok(None),
                Some(Value::String(value)) if value.is_empty() => Ok(None),
                Some(Value::String(value)) => Regex::new(value)
                    .map(Some)
                    .map_err(|_| invalid("filter expression requires runtime validation")),
                _ => Err(invalid("filter field")),
            }
        };
        let types = match map.get(value_key("exclude-type")) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(value)) => vec![value.clone()],
            Some(value) => value
                .as_sequence()
                .ok_or_else(|| invalid("exclude-type"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| invalid("exclude-type"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(Self {
            include: pattern("filter")?,
            exclude: pattern("exclude-filter")?,
            types: types
                .iter()
                .flat_map(|value| value.split('|'))
                .map(canonical_kind)
                .collect(),
        })
    }
    fn keep(&self, name: &str, kind: &str) -> bool {
        self.include
            .as_ref()
            .is_none_or(|filter| filter.is_match(name))
            && self
                .exclude
                .as_ref()
                .is_none_or(|filter| !filter.is_match(name))
            && !self.types.contains(&canonical_kind(kind))
    }
}
fn canonical_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "ss" => "shadowsocks".into(),
        "socks" => "socks5".into(),
        other => other.into(),
    }
}
fn invalid(field: &str) -> PawsError {
    PawsError::Core(format!("invalid profile definition shape: {field}"))
}
fn required_text(map: &Mapping, field: &str) -> Result<String, PawsError> {
    map.get(value_key(field))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(field))
}
fn names(map: &Mapping, field: &str) -> Result<Vec<String>, PawsError> {
    match map.get(value_key(field)) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => value
            .as_sequence()
            .ok_or_else(|| invalid(field))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| invalid(field))
            })
            .collect(),
    }
}
impl SourceDefinitions {
    fn parse(source: &str) -> Result<Self, PawsError> {
        let mut value: Value = serde_yaml::from_str(source).map_err(|_| invalid("YAML"))?;
        value.apply_merge().map_err(|_| invalid("YAML merge"))?;
        let root = value
            .as_mapping()
            .ok_or_else(|| invalid("root must be a map"))?
            .clone();
        let mut leaves = Vec::new();
        let mut groups = Vec::new();
        let mut declared = HashSet::new();
        for key in ["proxies", "proxy-groups"] {
            let Some(value) = root.get(value_key(key)).filter(|value| !value.is_null()) else {
                continue;
            };
            for value in value.as_sequence().ok_or_else(|| invalid(key))? {
                let map = value.as_mapping().ok_or_else(|| invalid(key))?;
                let name = required_text(map, "name")?;
                let kind = required_text(map, "type")?;
                if !declared.insert(name.clone()) {
                    return Err(invalid("duplicate proxy/group name"));
                }
                if key == "proxies" {
                    leaves.push((name, kind));
                } else {
                    names(map, "proxies")?;
                    names(map, "use")?;
                    groups.push(map.clone());
                }
            }
        }
        for key in ["proxy-providers", "rule-providers"] {
            let Some(value) = root.get(value_key(key)).filter(|value| !value.is_null()) else {
                continue;
            };
            for (name, definition) in value.as_mapping().ok_or_else(|| invalid(key))? {
                if name.as_str().is_none_or(str::is_empty) || !definition.is_mapping() {
                    return Err(invalid(key));
                }
                required_text(definition.as_mapping().unwrap(), "type")?;
            }
        }
        let rules = names(&root, "rules")?;
        Ok(Self {
            root,
            leaves,
            groups,
            rules,
        })
    }
}

/// Common shape validation does not imply that a selected backend supports
/// any protocol. That check belongs to the actual backend compiler/runtime.
pub fn validate_profile_source_shape(source: &str) -> Result<(), PawsError> {
    SourceDefinitions::parse(source).map(|_| ())
}

impl ProfileStore {
    pub fn source_metadata(
        &self,
        prepared_yaml: &str,
        selections: &BTreeMap<String, String>,
    ) -> Result<ProfileSourceMetadata, PawsError> {
        let definitions = SourceDefinitions::parse(prepared_yaml)?;
        let mut providers = self.providers_from_yaml(prepared_yaml);
        let mut unverified = Vec::new();
        for provider in providers
            .iter_mut()
            .filter(|provider| provider.provider_type == "proxy")
        {
            let source = definitions
                .root
                .get(value_key("proxy-providers"))
                .and_then(Value::as_mapping)
                .and_then(|providers| providers.get(value_key(&provider.name)))
                .and_then(Value::as_mapping);
            let inline = source
                .filter(|map| map.get(value_key("type")).and_then(Value::as_str) == Some("inline"))
                .and_then(|map| map.get(value_key("payload")))
                .cloned();
            let payload =
                inline.or_else(|| self.cached_proxy_definitions(provider.path.as_deref()));
            let members = payload
                .as_ref()
                .and_then(|payload| {
                    payload
                        .as_sequence()
                        .or_else(|| payload.get("proxies").and_then(Value::as_sequence))
                })
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            let map = item
                                .as_mapping()
                                .ok_or_else(|| invalid("provider member"))?;
                            Ok(ProviderProxySummary {
                                name: required_text(map, "name")?,
                                proxy_type: required_text(map, "type")?,
                                alive: None,
                                delay_ms: None,
                            })
                        })
                        .collect::<Result<Vec<_>, PawsError>>()
                });
            let filter = source.map(SourceFilter::parse).transpose();
            let unsupported_override = source.is_some_and(|map| {
                map.get(value_key("override"))
                    .is_some_and(|value| !value.is_null())
            });
            match (members, filter, unsupported_override) {
                (Some(Ok(mut members)), Ok(filter), false) => {
                    if let Some(filter) = filter {
                        members.retain(|member| filter.keep(&member.name, &member.proxy_type));
                    }
                    provider.members = members;
                }
                _ => unverified.push(provider.name.clone()),
            }
        }
        let mut kinds: BTreeMap<String, String> = definitions.leaves.iter().cloned().collect();
        for (name, kind) in [
            ("DIRECT", "Direct"),
            ("REJECT", "Reject"),
            ("REJECT-DROP", "RejectDrop"),
            ("COMPATIBLE", "Compatible"),
        ] {
            kinds.entry(name.into()).or_insert_with(|| kind.into());
        }
        for group in &definitions.groups {
            kinds.insert(required_text(group, "name")?, required_text(group, "type")?);
        }
        for provider in &providers {
            for member in &provider.members {
                kinds
                    .entry(member.name.clone())
                    .or_insert_with(|| member.proxy_type.clone());
            }
        }
        let mut groups = Vec::new();
        for definition in &definitions.groups {
            let name = required_text(definition, "name")?;
            let kind = required_text(definition, "type")?;
            let mut members = names(definition, "proxies")?;
            let all = definition
                .get(value_key("include-all"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if all
                || definition
                    .get(value_key("include-all-proxies"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                members.extend(definitions.leaves.iter().map(|(name, _)| name.clone()));
            }
            let use_names = names(definition, "use")?;
            let all_providers = all
                || definition
                    .get(value_key("include-all-providers"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            let filter = SourceFilter::parse(definition).ok();
            let mut provider_order = use_names;
            if all_providers {
                let mut extra = providers
                    .iter()
                    .filter(|provider| {
                        provider.provider_type == "proxy"
                            && !provider_order.contains(&provider.name)
                    })
                    .map(|provider| provider.name.clone())
                    .collect::<Vec<_>>();
                extra.sort();
                provider_order.extend(extra);
            }
            for provider_name in provider_order {
                if let Some(provider) = providers.iter().find(|provider| {
                    provider.provider_type == "proxy" && provider.name == provider_name
                }) {
                    if let Some(filter) = &filter {
                        members.extend(
                            provider
                                .members
                                .iter()
                                .filter(|member| filter.keep(&member.name, &member.proxy_type))
                                .map(|member| member.name.clone()),
                        );
                    }
                }
            }
            let mut seen = HashSet::new();
            members.retain(|member| member != &name && seen.insert(member.clone()));
            let automatic = matches!(kind.as_str(), "url-test" | "fallback" | "load-balance");
            let saved = selections.get(&name).cloned().or_else(|| {
                definition
                    .get(value_key("default-selected"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
            let selected = saved
                .clone()
                .filter(|name| members.contains(name))
                .or_else(|| (!automatic).then(|| members.first().cloned()).flatten());
            let fixed = automatic.then(|| {
                saved
                    .filter(|name| members.contains(name))
                    .unwrap_or_default()
            });
            groups.push(group(&name, &kind, members, selected, fixed, &kinds));
        }
        if !groups.iter().any(|group| group.name == "GLOBAL") {
            let mut members = vec!["DIRECT".to_owned()];
            members.extend(definitions.leaves.iter().map(|(name, _)| name.clone()));
            members.extend(groups.iter().map(|group| group.name.clone()));
            for provider in &providers {
                members.extend(provider.members.iter().map(|member| member.name.clone()));
            }
            let mut seen = HashSet::new();
            members.retain(|name| seen.insert(name.clone()));
            let selected = selections
                .get("GLOBAL")
                .filter(|name| members.contains(name))
                .cloned()
                .unwrap_or_else(|| "DIRECT".into());
            groups.push(group(
                "GLOBAL",
                "select",
                members,
                Some(selected),
                None,
                &kinds,
            ));
        }
        Ok(ProfileSourceMetadata {
            proxy_groups: groups,
            providers,
            rule_lines: definitions.rules,
            unverified_providers: unverified,
        })
    }

    fn cached_proxy_definitions(&self, path: Option<&str>) -> Option<Value> {
        let path = Path::new(path?).canonicalize().ok()?;
        let allowed = self.root.join("runtime").canonicalize().ok()?;
        if !path.starts_with(allowed) {
            return None;
        }
        let file = fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_PROVIDER_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_PROVIDER_BYTES {
            return None;
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        let mut depth = 0usize;
        for character in text.chars() {
            match character {
                '[' | '{' => {
                    depth += 1;
                    if depth > 128 {
                        return None;
                    }
                }
                ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if text
            .lines()
            .any(|line| line.len() - line.trim_start_matches(' ').len() > 256)
        {
            return None;
        }
        let mut value: Value = serde_yaml::from_slice(&bytes).ok()?;
        value.apply_merge().ok()?;
        Some(value)
    }
}

fn group(
    name: &str,
    kind: &str,
    members: Vec<String>,
    selected: Option<String>,
    fixed: Option<String>,
    kinds: &BTreeMap<String, String>,
) -> ProxyGroup {
    ProxyGroup {
        name: name.into(),
        group_type: kind.into(),
        proxies: members
            .into_iter()
            .map(|name| ProxyItem {
                proxy_type: kinds
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                delay_ms: None,
                selected: selected.as_deref() == Some(&name),
                name,
            })
            .collect(),
        selected,
        fixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declarations_preserve_non_meow_protocols_order_and_preselection_without_health_claims() {
        let root = std::env::temp_dir().join(format!(
            "paws-source-metadata-{}-{}",
            std::process::id(),
            crate::now_nanos()
        ));
        let store = ProfileStore::open(&root).unwrap();
        let yaml="proxies:\n  - {name: grpc, type: trojan, server: example.test, port: 443, password: secret, network: grpc}\n  - {name: any, type: backend-specific}\nproxy-providers:\n  remote: {type: http, url: 'http://127.0.0.1:1/no-network'}\nproxy-groups:\n  - {name: Manual, type: select, proxies: [grpc, any]}\n  - {name: Auto, type: url-test, use: [remote]}\nrules: ['MATCH,Manual']\n";
        let result = store
            .source_metadata(yaml, &BTreeMap::from([("Manual".into(), "any".into())]))
            .unwrap();
        assert_eq!(result.proxy_groups[0].name, "Manual");
        assert_eq!(
            result.proxy_groups[0]
                .proxies
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["grpc", "any"]
        );
        assert_eq!(result.proxy_groups[0].selected.as_deref(), Some("any"));
        assert_eq!(result.proxy_groups[0].proxies[0].proxy_type, "trojan");
        assert!(result
            .proxy_groups
            .iter()
            .flat_map(|group| &group.proxies)
            .all(|item| item.delay_ms.is_none()));
        assert_eq!(result.unverified_providers, ["remote"]);
        assert_eq!(result.proxy_groups[1].selected, None);
        assert_eq!(result.proxy_groups[1].fixed.as_deref(), Some(""));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn shared_shape_accepts_backend_fields_but_rejects_broken_declarations() {
        validate_profile_source_shape("proxies: [{name: other, type: other-backend, transport: {custom: true}}]\nrules: ['MATCH,DIRECT']").unwrap();
        for yaml in [
            "proxies: false",
            "proxy-groups: [{name: Group, type: select, proxies: false}]",
            "proxy-providers: {remote: false}",
            "rules: [false]",
        ] {
            assert!(validate_profile_source_shape(yaml).is_err());
        }
    }

    #[test]
    fn cached_members_use_declared_order_filters_and_never_select_a_removed_name() {
        let root = std::env::temp_dir().join(format!(
            "paws-source-filter-{}-{}",
            std::process::id(),
            crate::now_nanos()
        ));
        let store = ProfileStore::open(&root).unwrap();
        let cache = root.join("runtime/providers");
        fs::create_dir_all(&cache).unwrap();
        let z = cache.join("z.yaml");
        let a = cache.join("a.yaml");
        fs::write(&z,"proxies: [{name: us-z, type: socks5}, {name: us-expat, type: socks5}, {name: us-ss, type: ss}]").unwrap();
        fs::write(&a, "proxies: [{name: us-a, type: trojan}]").unwrap();
        let yaml=format!("proxies: [{{name: Static, type: any-backend}}]\nproxy-providers:\n  A: {{type: file, path: {a:?}}}\n  Z: {{type: file, path: {z:?}}}\nproxy-groups:\n  - {{name: Manual, type: select, proxies: [Static], use: [Z,A], filter: '^us', exclude-filter: 'expat', exclude-type: [ss]}}\n  - {{name: Auto, type: url-test, proxies: [Static], default-selected: absent}}\n",a=a.to_str().unwrap(),z=z.to_str().unwrap());
        let metadata = store
            .source_metadata(
                &yaml,
                &BTreeMap::from([
                    ("Manual".into(), "removed".into()),
                    ("GLOBAL".into(), "removed".into()),
                ]),
            )
            .unwrap();
        assert_eq!(
            metadata.proxy_groups[0]
                .proxies
                .iter()
                .map(|proxy| proxy.name.as_str())
                .collect::<Vec<_>>(),
            ["Static", "us-z", "us-a"]
        );
        assert_eq!(metadata.proxy_groups[0].selected.as_deref(), Some("Static"));
        assert_eq!(metadata.proxy_groups[1].selected, None);
        assert_eq!(metadata.proxy_groups[1].fixed.as_deref(), Some(""));
        assert!(!metadata
            .proxy_groups
            .iter()
            .any(|group| group.selected.as_deref() == Some("removed")));
        assert!(metadata
            .providers
            .iter()
            .flat_map(|provider| &provider.members)
            .all(|member| member.alive.is_none() && member.delay_ms.is_none()));
        let _ = fs::remove_dir_all(root);
    }
}
