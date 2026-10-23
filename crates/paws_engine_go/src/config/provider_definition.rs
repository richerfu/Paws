//! Pure provider-source validation shared by qualification and native startup.
//! It performs no path reads, downloads, or provider health checks.
use paws_engine::EngineError;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(untagged)]
enum HeaderValue {
    One(String),
    Many(Vec<String>),
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct Health {
    enable: bool,
    url: String,
    interval: u64,
    lazy: bool,
    timeout: u64,
    expected_status: String,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct Definition {
    #[serde(rename = "type")]
    source: String,
    url: String,
    path: String,
    interval: u64,
    proxy: String,
    header: BTreeMap<String, HeaderValue>,
    payload: Option<Vec<serde_yaml::Value>>,
    behavior: String,
    format: String,
    health_check: Health,
    filter: String,
    exclude_filter: String,
    #[serde(rename = "override")]
    overrides: Option<serde_yaml::Value>,
    size_limit: u64,
}
fn invalid(reason: &'static str) -> EngineError {
    EngineError::InvalidConfig(format!("sing-box provider definition: {reason}"))
}
impl Definition {
    fn validate(&self, proxy: bool) -> Result<(), EngineError> {
        match self.source.as_str() {
            "http" => {
                let url = reqwest::Url::parse(&self.url)
                    .map_err(|_| invalid("invalid HTTP source URL"))?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err(invalid("source URL must use HTTP or HTTPS"));
                }
            }
            "file" if !self.path.is_empty() => {}
            "inline" if self.payload.is_some() => {}
            _ => {
                return Err(invalid(
                    "source requires a supported type and location or payload",
                ));
            }
        }
        if self.interval > 365 * 86400
            || self.health_check.interval > 365 * 86400
            || self.health_check.timeout > 120_000
        {
            return Err(invalid("provider timing exceeds runtime bounds"));
        }
        if self.size_limit > 8 * 1024 * 1024 {
            return Err(invalid("size-limit exceeds compiler input budget"));
        }
        if proxy {
            if !matches!(self.format.as_str(), "" | "yaml") || !self.behavior.is_empty() {
                return Err(invalid("proxy provider requires YAML proxy data"));
            }
        } else if !matches!(self.behavior.as_str(), "domain" | "ipcidr" | "classical")
            || !matches!(self.format.as_str(), "" | "yaml" | "text" | "mrs")
            || (self.format == "mrs" && self.behavior == "classical")
        {
            return Err(invalid("unsupported rule provider behavior or format"));
        }
        if !self.filter.is_empty() || !self.exclude_filter.is_empty() || self.overrides.is_some() {
            return Err(invalid(
                "provider filter or override conversion is not implemented",
            ));
        }
        for value in self.header.values() {
            let values = match value {
                HeaderValue::One(value) => std::slice::from_ref(value),
                HeaderValue::Many(values) => values.as_slice(),
            };
            if values.iter().any(|value| value.contains(['\r', '\n'])) {
                return Err(invalid("provider header contains a line break"));
            }
        }
        if self.health_check.enable {
            let url = reqwest::Url::parse(&self.health_check.url)
                .map_err(|_| invalid("invalid health-check URL"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(invalid("health-check URL must use HTTP or HTTPS"));
            }
        }
        // These fields are executed by the native owner rather than qualification.
        let _ = (
            &self.proxy,
            self.health_check.lazy,
            &self.health_check.expected_status,
        );
        Ok(())
    }
}
/// Validates declarations through the same pure schema used by native provider
/// startup. Payload conversion remains the single initial/refresh compiler.
pub fn validate_sing_box_provider_definitions(
    root: &serde_yaml::Mapping,
) -> Result<(), EngineError> {
    for (key, proxy) in [("proxy-providers", true), ("rule-providers", false)] {
        let Some(value) = root.get(serde_yaml::Value::String(key.into())) else {
            continue;
        };
        let declarations = value
            .as_mapping()
            .ok_or_else(|| invalid("providers must be a map"))?;
        for (name, value) in declarations {
            if name.as_str().is_none_or(str::is_empty) {
                return Err(invalid("provider name is empty or not a string"));
            }
            let definition: Definition = serde_yaml::from_value(value.clone())
                .map_err(|_| invalid("unknown or invalid provider declaration fields"))?;
            definition.validate(proxy)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn validate(text: &str) -> Result<(), EngineError> {
        validate_sing_box_provider_definitions(
            serde_yaml::from_str::<serde_yaml::Value>(text)
                .unwrap()
                .as_mapping()
                .unwrap(),
        )
    }
    #[test]
    fn native_and_qualification_validate_the_same_sources_without_io() {
        validate("proxy-providers: {a: {type: http, url: 'http://127.0.0.1:1/unreachable', path: cache, health-check: {enable: true, url: 'https://example.test/'}}}\nrule-providers: {b: {type: inline, behavior: domain, payload: ['+.example.test']}}").unwrap();
        for text in [
            "proxy-providers: {a: {type: http, url: 'ftp://secret.example/data'}}",
            "proxy-providers: {a: {type: inline, payload: [], filter: a}}",
            "rule-providers: {a: {type: inline, payload: [], behavior: classical, format: mrs}}",
            "proxy-providers: {a: {type: inline, payload: [], unknown: true}}",
        ] {
            assert!(validate(text).is_err());
        }
    }
}
