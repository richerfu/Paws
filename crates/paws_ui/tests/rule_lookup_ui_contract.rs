const VIEW: &str = include_str!("../src/view/pages/resources.rs");
const UI: &str = concat!(
    include_str!("../src/ui.rs"),
    include_str!("../src/ui/tasks.rs"),
    include_str!("../src/ui/operations.rs"),
    include_str!("../src/ui_store.rs")
);
const CORE: &str = include_str!("../../paws_core/src/lib.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("section start");
    let tail = &source[start..];
    let end = tail.find(end).expect("section end");
    &tail[..end]
}

#[test]
fn resources_header_opens_a_domain_and_ip_rule_lookup() {
    let page = VIEW;

    assert!(page.contains("lookup_services.open_rule_lookup(lookup_editors.clone())"));
    assert!(page.contains("RuleLookupDialog { local: local_editors }"));
    assert!(page.contains("fn RuleLookupDialog("));
    assert!(page.contains("fn RuleLookupDialogContent"));
    assert!(page.contains("\"example.com / 203.0.113.1\""));
    assert!(page.contains("query_services.set_rule_lookup_query(query_editors.clone(), value)"));
    assert!(page.contains("services.lookup_rule(lookup_editors.clone())"));
    assert!(page.contains("add_services.add_rule_from_lookup(add_editors.clone())"));
    assert!(page.contains("tr::page_tr_207()"));
    assert!(page.contains("result.rule_line"));
    assert!(page.contains("result.resolved_ip"));
    assert!(page.contains("proxies.mode != RuntimeMode::Rule"));
}

#[test]
fn lookup_state_tracks_async_results_without_reopening_a_closed_dialog() {
    assert!(UI.contains("lookup: Option<RuleLookupState>"));
    assert!(UI.contains("query_task"));
    assert!(UI.contains("local.replace_query(task.abort_handle())"));
    assert!(UI.contains(".filter(|lookup| lookup.id == lookup_id)"));
    assert!(UI.contains("spawn(lookup_rule(lookup.query))"));
    assert!(UI.contains("pub(crate) fn add_rule_from_lookup"));
    assert!(UI.contains("self.open_manual_rule_editor(local, None, domain, destination_ip)"));
    assert!(UI.contains("if !local.is_alive()"));
    assert!(!UI.contains("Duration::from_millis(40)"));
    assert!(UI.contains("task.abort()"));
    assert!(UI.contains("editors.lookup = None"));
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
