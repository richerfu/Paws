use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableBackup {
    pub format: String,
    pub profiles: Vec<PortableProfile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableProfile {
    pub name: String,
    pub content: String,
    pub subscription_url: Option<String>,
    pub overrides: BTreeMap<String, Value>,
    pub selected_proxies: BTreeMap<String, String>,
    pub rules: Vec<String>,
}

impl ProfileStore {
    pub fn portable_backup(&self) -> Result<PortableBackup, PawsError> {
        let profiles = self
            .profiles
            .values()
            .map(|p| {
                Ok(PortableProfile {
                    name: p.name.clone(),
                    content: fs::read_to_string(self.root.join(&p.raw_yaml_path))
                        .map_err(io_error)?,
                    subscription_url: p.subscription_url.clone(),
                    overrides: p.user_overrides.clone(),
                    selected_proxies: p.selected_proxies.clone(),
                    rules: self.enabled_rule_lines(&p.id),
                })
            })
            .collect::<Result<Vec<_>, PawsError>>()?;
        Ok(PortableBackup {
            format: "TinyPaws-backup-v1".into(),
            profiles,
        })
    }

    /// Add restored copies atomically. Never overwrites active profiles or trusts archived paths/IDs.
    pub fn restore_portable_backup(&mut self, backup: PortableBackup) -> Result<usize, PawsError> {
        self.ensure_available()?;
        if backup.format != "TinyPaws-backup-v1" || backup.profiles.len() > 50 {
            return Err(PawsError::Core(
                "Unsupported backup format or too many profiles".into(),
            ));
        }
        let mut staged = self.clone();
        let count = backup.profiles.len();
        let mut writes = Vec::new();
        for entry in backup.profiles {
            if entry.name.trim().is_empty()
                || entry.name.len() > 512
                || entry.content.len() > 10 * 1024 * 1024
            {
                return Err(PawsError::Core("Invalid backup profile".into()));
            }
            if let Some(url) = &entry.subscription_url {
                let parsed = Url::parse(url).map_err(|e| PawsError::Core(e.to_string()))?;
                if !matches!(parsed.scheme(), "https" | "http") {
                    return Err(PawsError::Core("Invalid subscription scheme".into()));
                }
            }
            if entry
                .overrides
                .keys()
                .any(|k| !["dns", "tun", "paws"].contains(&k.as_str()))
            {
                return Err(PawsError::Core("Unsupported override section".into()));
            }
            let content = normalize_profile_content(&entry.content)?;
            let id = next_id("profile");
            let raw_path = format!("profiles/{id}.yaml");
            let backup_path = format!("backups/{id}.yaml");
            writes.push((self.root.join(&raw_path), content.as_bytes().to_vec()));
            writes.push((self.root.join(&backup_path), content.as_bytes().to_vec()));
            staged.profiles.insert(
                id.clone(),
                ProfileDocument {
                    id: id.clone(),
                    name: format!("{} (恢复副本)", entry.name),
                    source: entry
                        .subscription_url
                        .clone()
                        .unwrap_or_else(|| "backup".into()),
                    raw_yaml_path: raw_path,
                    yaml_backup_path: Some(backup_path),
                    user_overrides: entry.overrides,
                    subscription_url: entry.subscription_url,
                    updated_at: Some(now_string()),
                    last_refresh_at: None,
                    last_refresh_error: None,
                    selected_proxies: entry.selected_proxies,
                    upload_bytes: 0,
                    download_bytes: 0,
                    subscription_user_info: None,
                    subscription_metadata: None,
                },
            );
            for (order, line) in entry.rules.into_iter().enumerate() {
                let rule_id = next_id("rule");
                staged.rules.insert(
                    rule_id.clone(),
                    RuleDocument {
                        id: rule_id,
                        profile_id: id.clone(),
                        line,
                        enabled: true,
                        order: order as u32,
                        source: "backup".into(),
                    },
                );
            }
        }
        self.commit_staged_with_files(staged, writes)?;
        Ok(count)
    }
}
