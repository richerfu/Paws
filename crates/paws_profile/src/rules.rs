use super::*;

pub(super) fn parse_imported_rule_lines(rules_text: &str) -> Result<Vec<String>, PawsError> {
    let content = rules_text.trim_start_matches('\u{feff}').trim();
    if content.is_empty() {
        return Err(PawsError::Core(
            "the selected file does not contain custom rules".to_owned(),
        ));
    }

    let rules = match serde_yaml::from_str::<Value>(content) {
        Ok(Value::Mapping(root)) => {
            let value = root.get(&value_key("rules")).ok_or_else(|| {
                PawsError::Core(
                    "the selected YAML file does not contain a top-level rules list".to_owned(),
                )
            })?;
            imported_rule_yaml_sequence(value)?
        }
        Ok(Value::Sequence(values)) => imported_rule_values(&values)?,
        _ => imported_rule_text_lines(content),
    };

    if rules.is_empty() {
        return Err(PawsError::Core(
            "the selected file does not contain custom rules".to_owned(),
        ));
    }
    for (index, rule) in rules.iter().enumerate() {
        validate_imported_rule_line(rule, index)?;
    }
    Ok(rules)
}

pub(super) fn imported_rule_yaml_sequence(value: &Value) -> Result<Vec<String>, PawsError> {
    match value {
        Value::Sequence(values) => imported_rule_values(values),
        _ => Err(PawsError::Core(
            "the top-level rules field must be a YAML list".to_owned(),
        )),
    }
}

pub(super) fn imported_rule_values(values: &[Value]) -> Result<Vec<String>, PawsError> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    PawsError::Core(format!(
                        "custom rule item {} must be a non-empty string",
                        index + 1
                    ))
                })
        })
        .collect()
}

pub(super) fn imported_rule_text_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            line.strip_prefix("- ")
                .map(str::trim)
                .unwrap_or(line)
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

pub(super) fn validate_imported_rule_line(rule: &str, index: usize) -> Result<(), PawsError> {
    let mut fields = rule.split(',').map(str::trim);
    let rule_type = fields.next().unwrap_or_default();
    let first_argument = fields.next().unwrap_or_default();
    if rule_type.is_empty()
        || first_argument.is_empty()
        || rule_type.contains([':', '\n', '\r'])
        || rule.starts_with('-')
    {
        return Err(PawsError::Core(format!(
            "invalid custom rule at item {}: {rule}",
            index + 1
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedManualRule {
    match_kind: ManualRuleMatchKind,
    value: String,
    target: String,
    ipv6: bool,
}

impl NormalizedManualRule {
    pub(super) fn selector(&self) -> (ManualRuleMatchKind, String) {
        (self.match_kind, self.value.clone())
    }

    pub(super) fn line(&self) -> String {
        format!(
            "{},{},{}",
            self.match_kind.rule_type(self.ipv6),
            self.value,
            self.target
        )
    }
}

pub(super) fn normalize_manual_rule_spec(
    spec: &ManualRuleSpec,
) -> Result<NormalizedManualRule, PawsError> {
    let target = spec.target.trim();
    if target.is_empty() || target.contains(',') || target.contains('\n') || target.contains('\r') {
        return Err(PawsError::Core(
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

pub(super) fn normalize_manual_rule_value(
    match_kind: ManualRuleMatchKind,
    value: &str,
) -> Result<(String, bool), PawsError> {
    match match_kind {
        ManualRuleMatchKind::Domain | ManualRuleMatchKind::DomainSuffix => {
            let mut value = value.trim().trim_end_matches('.');
            if match_kind == ManualRuleMatchKind::DomainSuffix {
                value = value.trim_start_matches('.');
            }
            if value.is_empty() || value.contains(['/', ',', '\n', '\r']) {
                return Err(PawsError::Core("invalid manual rule domain".to_owned()));
            }
            let domain = match Host::parse(value)
                .map_err(|_| PawsError::Core(format!("invalid manual rule domain: {value}")))?
            {
                Host::Domain(domain) => domain,
                Host::Ipv4(_) | Host::Ipv6(_) => {
                    return Err(PawsError::Core(
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
                    .map_err(|_| PawsError::Core(format!("invalid IP/CIDR: {}", value.trim())))?
            } else {
                let ip = value.trim().parse::<IpAddr>().map_err(|_| {
                    PawsError::Core(format!("invalid IP address: {}", value.trim()))
                })?;
                IpNet::new(ip, if ip.is_ipv4() { 32 } else { 128 })
                    .map_err(|error| PawsError::Core(format!("invalid IP address: {error}")))?
            }
            .trunc();
            Ok((network.to_string(), network.addr().is_ipv6()))
        }
    }
}

pub(super) fn manual_rule_selector(line: &str) -> Option<(ManualRuleMatchKind, String)> {
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
