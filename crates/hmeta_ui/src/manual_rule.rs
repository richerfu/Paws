use hmeta_model::{ManualRuleMatchKind, RuleSummary};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualRuleConflict {
    pub target: String,
    pub source: String,
    pub same_target: bool,
}

pub(crate) fn manual_rule_preview(
    match_kind: ManualRuleMatchKind,
    value: &str,
    target: &str,
) -> String {
    let value = preview_value(match_kind, value);
    let target = if target.trim().eq_ignore_ascii_case("DIRECT") {
        "DIRECT"
    } else {
        target.trim()
    };
    format!(
        "{},{},{}",
        preview_rule_type(match_kind, &value),
        value,
        target
    )
}

pub(crate) fn find_manual_rule_conflict(
    rules: &[RuleSummary],
    match_kind: ManualRuleMatchKind,
    value: &str,
    target: &str,
) -> Option<ManualRuleConflict> {
    let expected = selector(match_kind, value);
    rules.iter().find_map(|rule| {
        let (existing_kind, existing_value, existing_target) = parse_rule(&rule.line)?;
        (selector(existing_kind, &existing_value) == expected).then(|| ManualRuleConflict {
            target: existing_target.clone(),
            source: rule.source.clone(),
            same_target: existing_target.eq_ignore_ascii_case(target.trim()),
        })
    })
}

fn parse_rule(line: &str) -> Option<(ManualRuleMatchKind, String, String)> {
    let mut fields = line.split(',').map(str::trim);
    let match_kind = match fields.next()?.to_ascii_uppercase().as_str() {
        "DOMAIN" => ManualRuleMatchKind::Domain,
        "DOMAIN-SUFFIX" => ManualRuleMatchKind::DomainSuffix,
        "IP-CIDR" | "IP-CIDR6" => ManualRuleMatchKind::IpCidr,
        _ => return None,
    };
    let value = fields.next()?.to_owned();
    let target = fields.next()?.to_owned();
    Some((match_kind, value, target))
}

fn selector(match_kind: ManualRuleMatchKind, value: &str) -> (ManualRuleMatchKind, String) {
    (match_kind, preview_value(match_kind, value))
}

fn preview_value(match_kind: ManualRuleMatchKind, value: &str) -> String {
    match match_kind {
        ManualRuleMatchKind::Domain => value.trim().trim_end_matches('.').to_ascii_lowercase(),
        ManualRuleMatchKind::DomainSuffix => value.trim().trim_matches('.').to_ascii_lowercase(),
        ManualRuleMatchKind::IpCidr => {
            let value = value.trim();
            if value.contains('/') {
                value.to_ascii_lowercase()
            } else if let Ok(ip) = value.parse::<IpAddr>() {
                format!("{ip}/{}", if ip.is_ipv4() { 32 } else { 128 })
            } else {
                value.to_ascii_lowercase()
            }
        }
    }
}

fn preview_rule_type(match_kind: ManualRuleMatchKind, value: &str) -> &'static str {
    match_kind.rule_type(match_kind == ManualRuleMatchKind::IpCidr && value.contains(':'))
}
