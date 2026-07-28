const VIEW: &str = include_str!("../src/view.rs");
const UI: &str = include_str!("../src/ui.rs");
const CORE: &str = include_str!("../../hmeta_core/src/lib.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn resources_header_opens_a_domain_and_ip_rule_lookup() {
    let page = section(VIEW, "fn resources_page", "fn provider_detail_dialog");

    assert!(page.contains("icon_action(\"route\", Action::OpenRuleLookup"));
    assert!(page.contains("rule_lookup_dialog(state, &current)"));
    assert!(page.contains("fn RuleLookupDialogContent"));
    assert!(page.contains("\"example.com / 203.0.113.1\""));
    assert!(page.contains("Action::SetRuleLookupQuery"));
    assert!(page.contains("Action::LookupRule"));
    assert!(page.contains("Action::AddRuleFromLookup"));
    assert!(page.contains("新增当前输入规则"));
    assert!(page.contains("result.rule_line"));
    assert!(page.contains("result.resolved_ip"));
    assert!(page.contains("current.snapshot.mode != RuntimeMode::Rule"));
}

#[test]
fn lookup_state_tracks_async_results_without_reopening_a_closed_dialog() {
    assert!(UI.contains("rule_lookup: Option<RuleLookupState>"));
    assert!(UI.contains("Action::RuleLookedUp"));
    assert!(UI.contains(".filter(|lookup| lookup.id == lookup_id)"));
    assert!(UI.contains("lookup_rule(lookup.query.clone())"));
    assert!(UI.contains("Action::AddRuleFromLookup"));
    assert!(UI.contains("Action::OpenManualRuleEditor"));
    assert!(UI.contains("Duration::from_millis(40)"));
    assert!(UI.contains("state.rule_lookup = None"));
}

#[test]
fn core_lookup_is_read_only_and_uses_the_compiled_rule_engine() {
    let lookup = section(
        CORE,
        "pub async fn lookup_rule",
        "pub fn active_vpn_options_json",
    );

    assert!(lookup.contains(".match_rules_lazy("));
    assert!(lookup.contains(".match_rules("));
    assert!(lookup.contains("resolve_ip_real"));
    assert!(!lookup.contains("resolve_proxy"));
    assert!(!lookup.contains("track_connection"));
}
