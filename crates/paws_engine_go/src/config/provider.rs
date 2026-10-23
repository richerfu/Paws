//! The single pure converter used by initial and refreshed native providers.
//! This module never fetches, reads paths, calls Core/JS, or takes session locks.
use paws_engine::EngineError;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum ProviderKind {
    Proxy,
    Rule,
}
#[derive(Clone, Copy)]
pub(crate) enum RuleBehavior {
    Domain,
    IpCidr,
    Classical,
}
#[derive(Clone, Copy)]
pub(crate) enum ProviderFormat {
    Yaml,
    Text,
    Mrs,
}

fn error(reason: &'static str) -> EngineError {
    EngineError::InvalidConfig(format!("sing-box provider conversion: {reason}"))
}
fn document(bytes: &[u8]) -> Result<serde_yaml::Value, EngineError> {
    let text = std::str::from_utf8(bytes).map_err(|_| error("payload is not UTF-8"))?;
    let mut depth = 0_u16;
    for byte in text.bytes() {
        match byte {
            b'[' | b'{' => {
                depth = depth
                    .checked_add(1)
                    .filter(|depth| *depth <= 128)
                    .ok_or_else(|| error("YAML nesting exceeds 128"))?;
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if text
        .lines()
        .any(|line| line.len() - line.trim_start_matches(' ').len() > 256)
    {
        return Err(error("YAML indentation exceeds 256"));
    }
    let mut parsed: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|_| error("invalid YAML payload"))?;
    parsed
        .apply_merge()
        .map_err(|_| error("invalid YAML merge"))?;
    Ok(parsed)
}

pub(crate) fn compile(
    kind: ProviderKind,
    behavior: RuleBehavior,
    format: ProviderFormat,
    bytes: &[u8],
) -> Result<Value, EngineError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(error("payload exceeds byte budget"));
    }
    match kind {
        ProviderKind::Proxy => {
            if !matches!(format, ProviderFormat::Yaml) {
                return Err(error("proxy provider requires YAML"));
            }
            let payload = document(bytes)?;
            let proxies = payload
                .as_sequence()
                .or_else(|| {
                    payload
                        .get("proxies")
                        .and_then(serde_yaml::Value::as_sequence)
                })
                .ok_or_else(|| error("proxy payload has no proxies sequence"))?;
            let mut names = BTreeSet::new();
            let mut outbounds = Vec::with_capacity(proxies.len());
            for proxy in proxies {
                let outbound = super::sing_box_proxy_outbound(proxy)
                    .ok_or_else(|| error("proxy protocol or transport cannot be represented"))?;
                let name = outbound["tag"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| error("proxy has no name"))?;
                if !names.insert(name.to_owned()) {
                    return Err(error("duplicate proxy name"));
                }
                outbounds.push(outbound);
            }
            Ok(json!({"outbounds":outbounds}))
        }
        ProviderKind::Rule => {
            let entries: Vec<String> = match format {
                ProviderFormat::Yaml => document(bytes)?
                    .get("payload")
                    .and_then(serde_yaml::Value::as_sequence)
                    .ok_or_else(|| error("rule payload has no payload sequence"))?
                    .iter()
                    .map(|entry| {
                        entry
                            .as_str()
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| error("rule payload entry is not a string"))
                    })
                    .collect::<Result<_, _>>()?,
                ProviderFormat::Text => std::str::from_utf8(bytes)
                    .map_err(|_| error("payload is not UTF-8"))?
                    .lines()
                    .map(str::trim)
                    .filter(|line| {
                        !line.is_empty() && !line.starts_with('#') && !line.starts_with("//")
                    })
                    .map(ToOwned::to_owned)
                    .collect(),
                ProviderFormat::Mrs => {
                    let reader = meow_rules::mrs_parser::UpstreamRuleSetReader::open_bounded(
                        bytes,
                        MAX_OUTPUT_BYTES as u64,
                    )
                    .map_err(|_| {
                        error("invalid MRS binary payload or decompression budget exceeded")
                    })?;
                    let expected = match behavior {
                        RuleBehavior::Domain => 0,
                        RuleBehavior::IpCidr => 1,
                        RuleBehavior::Classical => {
                            return Err(error("MRS has no classical rule-set encoding"));
                        }
                    };
                    if reader.behavior() != expected || reader.count() > 250_000 {
                        return Err(error("MRS behavior or count exceeds bounds"));
                    }
                    let mut entries = Vec::new();
                    if expected == 0 {
                        reader
                            .for_each_domain_bounded(
                                |domain| entries.push(domain.to_owned()),
                                250_000,
                                MAX_OUTPUT_BYTES / 2,
                            )
                            .map_err(|_| error("MRS domain output exceeds bounds or is invalid"))?;
                    } else {
                        reader
                            .for_each_net_bounded(|net| entries.push(net.to_string()), 250_000)
                            .map_err(|_| error("MRS CIDR output exceeds bounds or is invalid"))?;
                    }
                    entries
                }
            };
            let rules = entries
                .iter()
                .map(|entry| match behavior {
                    RuleBehavior::Classical => super::rule::matcher(entry),
                    RuleBehavior::IpCidr => super::rule::matcher(&format!("IP-CIDR,{entry}")),
                    RuleBehavior::Domain => {
                        if let Some(suffix) =
                            entry.strip_prefix("+.").or_else(|| entry.strip_prefix('.'))
                        {
                            Ok(json!({"domain_suffix":[suffix]}))
                        } else if entry.contains('*') {
                            Err(error(
                                "wildcard domain needs an explicit equivalent matcher",
                            ))
                        } else {
                            Ok(json!({"domain":[entry]}))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"rules":rules,"ruleCount":entries.len()}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_and_refresh_conversion_share_strict_protocol_rules() {
        let first = compile(
            ProviderKind::Proxy,
            RuleBehavior::Domain,
            ProviderFormat::Yaml,
            b"proxies: [{name: a, type: direct}]",
        )
        .unwrap();
        let refreshed = compile(
            ProviderKind::Proxy,
            RuleBehavior::Domain,
            ProviderFormat::Yaml,
            b"proxies: [{name: b, type: direct}]",
        )
        .unwrap();
        assert_eq!(first["outbounds"][0]["tag"], "a");
        assert_eq!(refreshed["outbounds"][0]["tag"], "b");
        assert!(compile(
            ProviderKind::Proxy,
            RuleBehavior::Domain,
            ProviderFormat::Yaml,
            b"proxies: [{name: b, type: unknown}]"
        )
        .is_err());
    }
    #[test]
    fn malformed_unicode_or_rule_provider_is_not_an_empty_success() {
        assert!(compile(
            ProviderKind::Rule,
            RuleBehavior::Domain,
            ProviderFormat::Text,
            &[0xff]
        )
        .is_err());
        assert!(compile(
            ProviderKind::Rule,
            RuleBehavior::Classical,
            ProviderFormat::Text,
            b"UNKNOWN,secret.invalid"
        )
        .is_err());
        assert!(compile(
            ProviderKind::Rule,
            RuleBehavior::Domain,
            ProviderFormat::Yaml,
            b"other: []"
        )
        .is_err());
        let value = compile(
            ProviderKind::Rule,
            RuleBehavior::Domain,
            ProviderFormat::Yaml,
            b"payload: [example.com, '+.example.org']",
        )
        .unwrap();
        assert_eq!(value["ruleCount"], 2);
    }
}
