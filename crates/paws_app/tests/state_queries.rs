use paws_app::{AppRequest, Application};
use serde_json::{json, Value};

#[tokio::test]
async fn domain_queries_require_current_subscription_and_new_owners_start_empty() {
    let root = std::env::temp_dir().join(format!("paws-state-queries-{}", std::process::id()));
    paws_core::configure_app_home(&root).unwrap();
    let app = Application::shared();
    let bootstrap = app
        .dispatch(AppRequest {
            id: 1,
            command: "bootstrap".into(),
            payload: json!({}),
        })
        .await;
    assert!(bootstrap.ok);
    assert_eq!(
        bootstrap.data,
        Some(Value::Null),
        "initialization cannot smuggle an aggregate snapshot into the wire"
    );
    assert!(
        !app.dispatch(AppRequest {
            id: 2,
            command: "snapshot".into(),
            payload: json!({})
        })
        .await
        .ok,
        "obsolete aggregate query must be unavailable"
    );
    let query = json!({"generation":1,"domain":"config"}).to_string();
    assert!(
        app.query_state_json(&query).await.is_err(),
        "queries must follow subscription"
    );
    let subscribe = json!({"generation":1,"domains":[{"domain":"rules"}]}).to_string();
    let changed: Value =
        serde_json::from_str(&app.subscribe_state_json(&subscribe).unwrap()).unwrap();
    assert_eq!(changed["generation"], 1);
    assert_eq!(changed["changes"].as_array().unwrap().len(), 3);
    let config: Value = serde_json::from_str(&app.query_state_json(&query).await.unwrap()).unwrap();
    assert!(config["data"]["vpnOptions"].is_object());
    assert!(config["data"].get("rules").is_none());
    assert!(config["data"].get("about").is_none());
    let replacement = Application::shared();
    assert!(replacement.query_state_json(&query).await.is_err());
    app.subscribe_state_json(&json!({"generation":2,"domains":[]}).to_string())
        .unwrap();
    assert!(
        app.query_state_json(&query).await.is_err(),
        "old route generations are revoked"
    );
    assert!(
        app.query_state_json(&json!({"generation":2,"domain":"rules"}).to_string())
            .await
            .is_err(),
        "unsubscribed domains cannot load"
    );
    assert!(
        app.subscribe_state_json(&subscribe).is_err(),
        "old subscription cannot replace new page"
    );
    app.close();
    assert!(app
        .query_state_json(&json!({"generation":2,"domain":"config"}).to_string())
        .await
        .is_err());
    let _ = std::fs::remove_dir_all(root);
}
