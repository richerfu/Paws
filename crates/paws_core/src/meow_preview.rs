//! Read-only configuration projection using upstream construction APIs.
//! Remote providers are represented by private file snapshots for this build;
//! the original raw document is retained and runtime starts always reload it.
use meow_config::{raw::RawConfig, Config};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub(super) enum PreviewError {
    #[error("{0}")]
    Construction(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Path(#[from] std::path::StripPrefixError),
}
fn construction(error: impl std::fmt::Display) -> PreviewError {
    PreviewError::Construction(error.to_string())
}

pub(super) struct MeowPreviewLoader<'a> {
    raw: RawConfig,
    cache: Option<&'a Path>,
}
impl<'a> MeowPreviewLoader<'a> {
    pub fn new(raw: RawConfig, cache: Option<&'a Path>) -> Self {
        Self { raw, cache }
    }

    pub async fn build(self) -> Result<Config, PreviewError> {
        let strict = self.raw.strict.unwrap_or(false);
        meow_config::ech_dns::check_ech_defects(self.raw.proxies.as_deref().unwrap_or(&[]), strict)
            .map_err(construction)?;
        self.validate_dns_urls()?;
        let files = match self.cache {
            Some(cache) => {
                std::fs::create_dir_all(cache)?;
                tempfile::Builder::new()
                    .prefix(".paws-preview-")
                    .tempdir_in(cache)?
            }
            None => tempfile::Builder::new().prefix("paws-preview-").tempdir()?,
        };
        let cache = self.cache.unwrap_or(files.path());
        let mut projected = self.raw.clone();
        let mut index = 0;
        if let Some(providers) = projected.proxy_providers.as_mut() {
            for provider in providers.values_mut() {
                if provider.provider_type != "http" {
                    continue;
                }
                let bytes = self
                    .cached(provider.path.as_deref())?
                    .unwrap_or_else(|| b"proxies: []\n".to_vec());
                let path = files.path().join(format!("proxy-{index}.yaml"));
                index += 1;
                std::fs::write(&path, bytes)?;
                provider.provider_type = "file".into();
                provider.path = Some(Self::snapshot_path(&path, cache)?);
                provider.url = None;
                provider.interval = None;
            }
        }
        if let Some(providers) = projected.rule_providers.as_mut() {
            for provider in providers.values_mut() {
                if provider.provider_type != "http" {
                    continue;
                }
                if let Some(bytes) = self.cached(provider.path.as_deref())? {
                    let path = files.path().join(format!("rule-{index}"));
                    index += 1;
                    std::fs::write(&path, bytes)?;
                    provider.provider_type = "file".into();
                    provider.path = Some(Self::snapshot_path(&path, cache)?);
                } else {
                    provider.provider_type = "inline".into();
                    provider.payload = Some(Vec::new());
                    provider.path = None;
                    provider.format = None;
                }
                provider.url = None;
                provider.interval = None;
                provider.proxy = None;
            }
        }
        // This upstream rebuild does not download geodata. File snapshots
        // remove provider HTTP acquisition; ECH parsing uses its deferred path.
        let rebuilt = meow_config::rebuild_from_raw_with_cache_dir(&projected, Some(cache), None)
            .map_err(construction)?;
        let mut config = meow_config::build_config(RawConfig::default(), None)
            .await
            .map_err(construction)?;
        config.general.mode = self
            .raw
            .mode
            .as_deref()
            .unwrap_or("rule")
            .parse()
            .map_err(construction)?;
        config.general.ipv6 = meow_config::effective_ipv6(self.raw.ipv6);
        config.sniffer = meow_config::sniffer_config_from_raw(&self.raw).map_err(construction)?;
        config.listeners.named =
            meow_config::resolve_named_listeners(&self.raw).map_err(construction)?;
        config.tun = meow_config::parse_tun_config(
            self.raw.tun.as_ref(),
            self.raw.max_connections,
            self.raw
                .dns
                .as_ref()
                .and_then(|dns| dns.fake_ip_range.as_deref()),
        )
        .map_err(construction)?;
        config.proxies = rebuilt.proxies;
        config.rules = rebuilt.rules;
        config.proxy_providers = rebuilt.proxy_providers;
        config.rule_providers = rebuilt.rule_providers;
        config.dialer_registry = rebuilt.dialer_registry;
        config.raw = self.raw;
        Ok(config)
    }

    fn snapshot_path(path: &Path, cache: &Path) -> Result<String, PreviewError> {
        path.strip_prefix(cache)?
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| construction("preview snapshot path is not UTF-8"))
    }

    fn cached(&self, path: Option<&str>) -> Result<Option<Vec<u8>>, PreviewError> {
        let Some(path) = path else {
            return Ok(None);
        };
        let cache = self
            .cache
            .ok_or_else(|| construction("provider cache path needs a configuration directory"))?
            .canonicalize()?;
        let mut resolved = PathBuf::new();
        for component in cache.join(path).components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    resolved.pop();
                }
                other => resolved.push(other),
            }
        }
        if !resolved.starts_with(&cache) {
            return Err(construction(
                "provider cache path escapes its configuration directory",
            ));
        }
        let path = match resolved.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !path.starts_with(&cache) {
            return Err(construction(
                "provider cache symlink escapes its configuration directory",
            ));
        }
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(construction("provider preview cache exceeds 16 MiB"));
        }
        Ok(Some(bytes))
    }

    fn validate_dns_urls(&self) -> Result<(), PreviewError> {
        let Some(dns) = &self.raw.dns else {
            return Ok(());
        };
        for value in dns
            .nameserver
            .iter()
            .flatten()
            .chain(dns.fallback.iter().flatten())
            .chain(dns.default_nameserver.iter().flatten())
            .chain(dns.proxy_server_nameserver.iter().flatten())
            .chain(dns.direct_nameserver.iter().flatten())
        {
            meow_dns::upstream::NameServerEntry::parse(value).map_err(construction)?;
        }
        for value in dns
            .nameserver_policy
            .iter()
            .chain(dns.proxy_server_nameserver_policy.iter())
            .flat_map(|map| map.values())
        {
            for url in value.as_urls() {
                meow_dns::upstream::NameServerEntry::parse(url).map_err(construction)?;
            }
        }
        Ok(())
    }
}
