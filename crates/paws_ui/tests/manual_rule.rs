#[path = "../src/manual_rule.rs"]
mod manual_rule;

use manual_rule::{find_manual_rule_conflict, manual_rule_preview};
use paws_model::{ManualRuleMatchKind, RuleSummary};

fn rule(line: &str, source: &str) -> RuleSummary {
    RuleSummary {
        id: line.to_owned(),
        profile_id: "profile".to_owned(),
        line: line.to_owned(),
        enabled: true,
        order: 0,
        source: source.to_owned(),
    }
}

#[test]
fn previews_normalize_domains_and_host_prefixes() {
    assert_eq!(
        manual_rule_preview(ManualRuleMatchKind::Domain, "API.Example.COM.", "direct"),
        "DOMAIN,api.example.com,DIRECT"
    );
    assert_eq!(
        manual_rule_preview(ManualRuleMatchKind::IpCidr, "2001:db8::1", "Proxy"),
        "IP-CIDR6,2001:db8::1/128,Proxy"
    );
}

#[test]
fn conflicts_distinguish_same_rules_from_profile_overrides() {
    let rules = vec![rule("DOMAIN-SUFFIX,example.com,Proxy", "profile-yaml")];
    let conflict = find_manual_rule_conflict(
        &rules,
        ManualRuleMatchKind::DomainSuffix,
        ".EXAMPLE.COM.",
        "DIRECT",
    )
    .unwrap();
    assert_eq!(conflict.target, "Proxy");
    assert_eq!(conflict.source, "profile-yaml");
    assert!(!conflict.same_target);

    assert!(find_manual_rule_conflict(
        &rules,
        ManualRuleMatchKind::Domain,
        "example.com",
        "DIRECT"
    )
    .is_none());
}
