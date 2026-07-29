#[path = "../src/subscription_converter.rs"]
mod subscription_converter;

use subscription_converter::{
    build_conversion_url, clash_install_url, format_custom_params, parse_conversion_url,
    parse_custom_params, CustomParam, SubscriptionConverterDraft, CLIENT_TYPES, REMOTE_CONFIGS,
};

#[test]
fn exposes_every_sub_web_client_and_remote_config() {
    assert_eq!(CLIENT_TYPES.len(), 16);
    assert!(CLIENT_TYPES
        .iter()
        .any(|client| client.value == "surge&ver=4"));
    assert!(CLIENT_TYPES.iter().any(|client| client.value == "singbox"));
    assert_eq!(REMOTE_CONFIGS.len(), 12);
    assert!(REMOTE_CONFIGS
        .iter()
        .any(|config| config.label.contains("NeteaseUnblock")));
}

#[test]
fn builds_sub_web_compatible_advanced_url() {
    let draft = SubscriptionConverterDraft {
        source_sub_url: "https://one.example/sub\nvmess://node".to_owned(),
        client_type: "surge&ver=4".to_owned(),
        remote_config: "https://config.example/rules.ini".to_owned(),
        include_remarks: "香港|HK".to_owned(),
        filename: "Paws Config".to_owned(),
        append_type: true,
        udp: true,
        need_udp: true,
        surge_doh: true,
        custom_params: vec![CustomParam {
            name: "x-test".to_owned(),
            value: "a b".to_owned(),
        }],
        ..SubscriptionConverterDraft::default()
    };

    let url = build_conversion_url(&draft).unwrap();
    assert!(url.starts_with(
        "https://api.wcc.best/sub?target=surge&ver=4&url=https%3A%2F%2Fone.example%2Fsub%7Cvmess%3A%2F%2Fnode&insert=false"
    ));
    assert!(url.contains("&config=https%3A%2F%2Fconfig.example%2Frules.ini"));
    assert!(url.contains("&include=%E9%A6%99%E6%B8%AF%7CHK"));
    assert!(url.contains("&filename=Paws%20Config"));
    assert!(url.contains("&append_type=true"));
    assert!(
        url.contains("&emoji=true&list=false&tfo=false&scv=true&fdn=false&expand=true&sort=false")
    );
    assert!(url.contains("&udp=true&surge.doh=true&x-test=a%20b"));
}

#[test]
fn basic_mode_only_emits_the_base_parameters() {
    let draft = SubscriptionConverterDraft {
        advanced: false,
        source_sub_url: "https://example.com/sub".to_owned(),
        insert: true,
        ..SubscriptionConverterDraft::default()
    };

    assert_eq!(
        build_conversion_url(&draft).unwrap(),
        "https://api.wcc.best/sub?target=clash&url=https%3A%2F%2Fexample.com%2Fsub&insert=true"
    );
}

#[test]
fn custom_backend_keeps_existing_query_parameters() {
    let draft = SubscriptionConverterDraft {
        advanced: false,
        source_sub_url: "https://example.com/sub".to_owned(),
        backend: "https://converter.example/sub?token=private".to_owned(),
        ..SubscriptionConverterDraft::default()
    };

    assert_eq!(
        build_conversion_url(&draft).unwrap(),
        "https://converter.example/sub?token=private&target=clash&url=https%3A%2F%2Fexample.com%2Fsub&insert=false"
    );
}

#[test]
fn parses_long_url_and_preserves_unknown_parameters() {
    let parsed = parse_conversion_url(
        "https://converter.example/sub?target=surge&ver=3&url=https%3A%2F%2Fa.example%2Fsub%7Css%3A%2F%2Fnode&insert=true&emoji=true&list=false&tfo=true&scv=true&fdn=true&expand=false&sort=true&udp=false&surge.doh=true&foo=bar",
    )
    .unwrap();

    assert_eq!(parsed.backend, "https://converter.example/sub?");
    assert_eq!(parsed.client_type, "surge&ver=3");
    assert_eq!(parsed.source_sub_url, "https://a.example/sub\nss://node");
    assert!(parsed.insert);
    assert!(parsed.emoji);
    assert!(parsed.tfo);
    assert!(parsed.need_udp);
    assert!(!parsed.udp);
    assert_eq!(
        parsed.custom_params,
        vec![CustomParam {
            name: "foo".to_owned(),
            value: "bar".to_owned()
        }]
    );
}

#[test]
fn custom_parameter_editor_round_trips_valid_lines() {
    let parsed = parse_custom_params("foo=bar\nempty=\n a = two words \ninvalid");
    assert_eq!(
        parsed,
        vec![
            CustomParam {
                name: "foo".to_owned(),
                value: "bar".to_owned(),
            },
            CustomParam {
                name: "a".to_owned(),
                value: "two words".to_owned(),
            }
        ]
    );
    assert_eq!(format_custom_params(&parsed), "foo=bar\na=two words");
}

#[test]
fn clash_install_prefers_short_url() {
    assert_eq!(
        clash_install_url(
            "https://converter.example/long?a=1",
            "https://short.example/a"
        )
        .unwrap(),
        "clash://install-config?url=https%3A%2F%2Fshort.example%2Fa"
    );
}
