use paws_profile::normalize_profile_content;
use std::fs;
use std::path::PathBuf;

#[test]
fn every_local_protocol_profile_is_accepted_by_the_app_yaml_parser() {
    let profiles =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local-protocol-tests/profiles");
    let entries = fs::read_dir(&profiles).expect("local protocol profile templates");

    for entry in entries {
        let path = entry.expect("profile template entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("in") {
            continue;
        }
        let template = fs::read_to_string(&path).expect("read profile template");
        let rendered = template
            .replace("{{HOST}}", "192.0.2.7")
            .replace("{{ECHO_PORT}}", "18080")
            .replace("{{PROXY_PORT}}", "18081");
        normalize_profile_content(&rendered)
            .unwrap_or_else(|error| panic!("{} was rejected: {error}", path.display()));
    }
}
