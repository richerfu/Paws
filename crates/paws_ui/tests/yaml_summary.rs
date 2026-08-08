#[path = "../src/yaml_summary.rs"]
mod yaml_summary;

use yaml_summary::summarize_yaml_edit;

#[test]
fn yaml_summary_counts_common_clash_sections() {
    let yaml = r#"proxies:
  - name: DIRECT
    type: direct
rules:
  - MATCH,DIRECT
proxy-providers:
  remote:
    type: http
rule-providers:
  geosite:
    type: http
"#;

    let summary = summarize_yaml_edit(yaml, yaml);

    assert_eq!(summary.lines, 11);
    assert_eq!(summary.proxies, 1);
    assert_eq!(summary.rules, 1);
    assert_eq!(summary.providers, 2);
    assert!(!summary.changed);
    assert!(summary.parseable);
}

#[test]
fn yaml_summary_marks_changed_and_unparseable_content() {
    let summary = summarize_yaml_edit("proxies: [", "proxies: []\n");

    assert!(summary.changed);
    assert!(!summary.parseable);
    assert_eq!(summary.proxies, 0);
}
