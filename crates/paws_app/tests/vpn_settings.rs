use paws_app::{AppRequest, Application};
use paws_model::{VpnEngine, VpnOptions};
use serde_json::{json, Value};

fn request(id: u64, command: &str, payload: Value) -> AppRequest {
    AppRequest {
        id,
        command: command.into(),
        payload,
    }
}

// A separate integration binary owns its process-wide Core and profile home.
#[tokio::test]
async fn settings_vpn_commits_mtu_with_engine_and_apps_through_the_real_store() {
    let root = std::env::temp_dir().join(format!("paws-app-vpn-settings-{}", std::process::id()));
    paws_core::configure_app_home(&root).unwrap();
    let app = Application::shared();
    let core = paws_core::shared_core();
    let yaml = "proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\npaws:\n  mixed-enabled: false\n  controller-enabled: false\ndns:\n  nameserver: [223.5.5.5]\n";
    for intent in [1, 2] {
        let reply = app
            .dispatch(request(
                intent,
                "profiles.importContent",
                json!({
                    "intentId":intent,"name":format!("MTU {intent}"),"content":yaml
                }),
            ))
            .await;
        assert!(reply.ok, "{:?}", reply.error);
    }
    let initial = core.command_context().unwrap();
    let profile_id = initial.active_profile.unwrap();
    let store = paws_profile::ProfileStore::open(root.clone()).unwrap();
    let inactive = store
        .summaries()
        .into_iter()
        .find(|p| p.id != profile_id)
        .unwrap()
        .id;
    let source_path = root.join(&store.profile(&profile_id).unwrap().raw_yaml_path);
    let source = std::fs::read(&source_path).unwrap();

    for (index, engine) in [VpnEngine::Meow, VpnEngine::Mihomo, VpnEngine::SingBox]
        .into_iter()
        .enumerate()
    {
        let before = core.command_context().unwrap();
        let mut options = before.vpn_options.clone();
        options.mtu = 2816;
        options.engine = engine;
        options.blocked_applications = vec!["com.example.synthetic".into()];
        let reply = app.dispatch(request(10 + index as u64, "settings.vpn", json!({
            "profileId":profile_id,"expectedRevision":before.config_revision.to_string(),"options":options
        }))).await;
        assert!(reply.ok, "{engine:?}: {:?}", reply.error);
        assert!(
            reply.effect.is_none(),
            "an inactive VPN must not be started by saving"
        );
        let after = core.command_context().unwrap();
        assert_eq!(after.config_revision, before.config_revision + 1);
        assert_eq!(after.vpn_options.mtu, 2816);
        assert_eq!(after.vpn_options.engine, engine);
        assert_eq!(
            after.vpn_options.blocked_applications,
            options.blocked_applications
        );
        let parsed =
            paws_profile::vpn_options_from_yaml(&core.profile_raw_yaml(&profile_id).unwrap())
                .unwrap();
        assert_eq!(parsed.mtu, 2816);
        assert_eq!(parsed.engine, engine);
        let reopened = paws_profile::ProfileStore::open(root.clone()).unwrap();
        assert_eq!(
            reopened.vpn_options_for_profile(&profile_id).unwrap(),
            parsed
        );
        assert_eq!(
            core.engine_tun_mtu(&serde_json::to_string(&parsed).unwrap())
                .unwrap(),
            2816
        );
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            source,
            "settings belong to the overlay, not subscription source"
        );
    }

    let before = core.command_context().unwrap();
    let index_before = std::fs::read(root.join("profiles.json")).unwrap();
    let stale = app.dispatch(request(20, "settings.vpn", json!({
        "profileId":profile_id,"expectedRevision":before.config_revision - 1,"options":before.vpn_options
    }))).await;
    assert!(!stale.ok);
    assert!(stale.error.unwrap().contains("stale"));
    let wrong_profile = app.dispatch(request(21, "settings.vpn", json!({
        "profileId":inactive,"expectedRevision":before.config_revision,"options":before.vpn_options
    }))).await;
    assert!(!wrong_profile.ok);
    assert!(wrong_profile.error.unwrap().contains("active profile"));

    for field in ["ipv6", "addresses", "routes", "dnsAddresses"] {
        let mut options = serde_json::to_value(&before.vpn_options).unwrap();
        options[field] = if field == "ipv6" {
            json!(!before.vpn_options.ipv6)
        } else {
            json!(["192.0.2.99/32"])
        };
        let reply = app.dispatch(request(22, "settings.vpn", json!({
            "profileId":profile_id,"expectedRevision":before.config_revision,"options":options
        }))).await;
        assert!(!reply.ok, "read-only field {field}");
        assert!(reply.error.unwrap().contains("profile YAML"));
        assert!(reply.effect.is_none());
    }
    for mtu in [
        json!(0),
        json!(575),
        json!(576),
        json!(1279),
        json!(65536),
        json!(-1),
        json!(2816.5),
        json!("2816"),
    ] {
        let mut options = serde_json::to_value(&before.vpn_options).unwrap();
        options["mtu"] = mtu.clone();
        let reply = app.dispatch(request(23, "settings.vpn", json!({
            "profileId":profile_id,"expectedRevision":before.config_revision,"options":options
        }))).await;
        assert!(!reply.ok, "invalid typed MTU {mtu}");
        assert!(reply.effect.is_none());
    }
    assert_eq!(
        core.command_context().unwrap().config_revision,
        before.config_revision
    );
    assert_eq!(
        std::fs::read(root.join("profiles.json")).unwrap(),
        index_before
    );
    assert_eq!(std::fs::read(&source_path).unwrap(), source);

    // Both integer boundaries and an unchanged-MTU settings save use the same
    // checked path; there is no second MTU command/reload.
    for mtu in [1280, 65535, 2816, 2816] {
        let before = core.command_context().unwrap();
        let options = VpnOptions {
            mtu,
            ..before.vpn_options
        };
        let reply = app.dispatch(request(24, "settings.vpn", json!({
            "profileId":profile_id,"expectedRevision":before.config_revision,"options":options
        }))).await;
        assert!(reply.ok, "MTU {mtu}: {:?}", reply.error);
        assert_eq!(
            core.command_context().unwrap().config_revision,
            before.config_revision + 1
        );
        assert_eq!(core.command_context().unwrap().vpn_options.mtu, mtu);
    }
    app.close();
    let _ = std::fs::remove_dir_all(root);
}
