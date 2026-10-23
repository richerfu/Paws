use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use paws_engine::{EngineControllerEndpoint, EngineError, EngineManagement};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
struct ControllerState {
    management: Arc<dyn EngineManagement>,
    secret: String,
}
pub(crate) struct PrivateController {
    pub endpoint: EngineControllerEndpoint,
    task: tokio::task::JoinHandle<()>,
}
impl PrivateController {
    pub async fn start(management: Arc<dyn EngineManagement>) -> Result<Self, EngineError> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| EngineError::InvalidConfig(error.to_string()))?;
        use std::fmt::Write;
        let mut secret = String::with_capacity(64);
        for byte in bytes {
            let _ = write!(secret, "{byte:02x}");
        }
        let state = ControllerState {
            management,
            secret: secret.clone(),
        };
        let router = Router::new()
            .route("/proxies", get(proxy_groups))
            .route("/paws/observations/revisions", get(runtime_revisions))
            .route("/paws/observations/groups", get(group_summaries))
            .route("/paws/observations/proxy-groups/{name}", get(proxy_group))
            .route(
                "/paws/observations/connection-events",
                get(connection_events),
            )
            .route(
                "/paws/observations/connection-summary",
                get(connection_summary),
            )
            .route("/paws/observations/rules", get(runtime_rules))
            .route("/paws/observations/dns", get(dns_statistics))
            .route("/paws/observations/dns-config", get(dns_config))
            .route("/paws/observations/memory", get(memory))
            .route("/paws/observations/logs", get(runtime_logs))
            .route(
                "/paws/observations/providers/{kind}/{name}",
                get(provider_state),
            )
            .route("/connections", get(connections).delete(close_all))
            .route("/connections/{id}", delete(close_one))
            .route("/cache/dns/flush", post(flush_dns))
            .route("/cache/fakeip/flush", post(flush_fake_ip))
            .route("/proxies/{name}/delay", get(proxy_delay))
            .route(
                "/proxies/{name}",
                get(proxy_selection).put(select_proxy).delete(unfix_proxy),
            )
            .route(
                "/providers/{kind}/{name}",
                get(provider_members).put(refresh_provider),
            )
            .route(
                "/providers/proxies/{name}/healthcheck",
                get(provider_health),
            )
            .route(
                "/providers/proxies/{name}/{proxy}/healthcheck",
                get(provider_proxy_health),
            )
            .route("/proxies/{name}/echo", post(proxy_echo))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, authenticate));
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::warn!(%error,"private meow controller stopped");
            }
        });
        Ok(Self {
            endpoint: EngineControllerEndpoint {
                address,
                secret: Some(secret),
            },
            task,
        })
    }
    pub async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for PrivateController {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn runtime_revisions(State(state): State<ControllerState>) -> Response {
    match state.management.runtime_revisions().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn authenticate(
    State(state): State<ControllerState>,
    request: Request,
    next: Next,
) -> Response {
    let expected = format!("Bearer {}", state.secret);
    if request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}
fn failure(error: EngineError) -> Response {
    let status = if matches!(
        error,
        EngineError::Unsupported { .. } | EngineError::ObservationUnavailable { .. }
    ) {
        StatusCode::NOT_IMPLEMENTED
    } else {
        StatusCode::BAD_GATEWAY
    };
    (
        status,
        Json(serde_json::json!({"message":error.to_string()})),
    )
        .into_response()
}
async fn group_summaries(State(state): State<ControllerState>) -> Response {
    match state.management.group_summaries().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn proxy_group(State(state): State<ControllerState>, Path(name): Path<String>) -> Response {
    match state.management.proxy_group(&name).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn connection_events(
    State(state): State<ControllerState>,
    Query(query): Query<PageQuery>,
) -> Response {
    match state
        .management
        .connection_events(query.after, query.limit)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn connection_summary(State(state): State<ControllerState>) -> Response {
    match state.management.connection_summary().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn runtime_rules(State(state): State<ControllerState>) -> Response {
    match state.management.runtime_rules().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn dns_statistics(State(state): State<ControllerState>) -> Response {
    match state.management.dns_statistics().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn dns_config(State(state): State<ControllerState>) -> Response {
    match state.management.dns_config().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn memory(State(state): State<ControllerState>) -> Response {
    match state.management.memory().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
#[derive(Deserialize)]
struct PageQuery {
    after: u64,
    limit: u16,
}
async fn runtime_logs(
    State(state): State<ControllerState>,
    Query(query): Query<PageQuery>,
) -> Response {
    match state
        .management
        .runtime_logs(query.after, query.limit)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn provider_state(
    State(state): State<ControllerState>,
    Path((kind, name)): Path<(String, String)>,
) -> Response {
    let kind = match kind.as_str() {
        "proxies" => paws_engine::EngineProviderKind::Proxy,
        "rules" => paws_engine::EngineProviderKind::Rule,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.management.provider_state(kind, &name).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn connections(State(state): State<ControllerState>) -> Response {
    match state.management.connections().await {
        Ok(connections) => {
            let connections=connections.into_iter().map(|connection|serde_json::json!({"id":connection.id,"metadata":{"host":connection.domain,"destinationIP":connection.destination_ip,"destinationPort":connection.destination_port,"network":connection.network},"upload":connection.upload_bytes,"download":connection.download_bytes,"chains":connection.chains,"start":connection.started_at,"rule":connection.rule,"rulePayload":connection.rule_payload})).collect::<Vec<_>>();
            Json(serde_json::json!({"connections":connections})).into_response()
        }
        Err(error) => failure(error),
    }
}
async fn proxy_groups(State(state): State<ControllerState>) -> Response {
    match state.management.proxy_groups().await {
        Ok(groups) => {
            let mut proxies = serde_json::Map::new();
            for group in groups {
                for proxy in &group.proxies {
                    proxies.entry(proxy.name.clone()).or_insert_with(|| serde_json::json!({"type":proxy.proxy_type,"history":proxy.delay_ms.map(|delay|vec![serde_json::json!({"delay":delay})]).unwrap_or_default()}));
                }
                proxies.insert(group.name, serde_json::json!({"type":group.group_type,"now":group.selected.unwrap_or_default(),"fixed":group.fixed,"all":group.proxies.into_iter().map(|proxy|proxy.name).collect::<Vec<_>>()}));
            }
            Json(serde_json::json!({"proxies":proxies})).into_response()
        }
        Err(error) => failure(error),
    }
}
async fn close_all(State(state): State<ControllerState>) -> Response {
    match state.management.close_connections(None).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn close_one(State(state): State<ControllerState>, Path(id): Path<String>) -> Response {
    match state.management.close_connections(Some(&id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
#[derive(Deserialize)]
struct Selection {
    name: String,
}
async fn proxy_selection(
    State(state): State<ControllerState>,
    Path(group): Path<String>,
) -> Response {
    match state.management.proxy_selection(&group).await {
        Ok(selection) => Json(serde_json::json!({"now":selection.selected.unwrap_or_default(),"fixed":selection.fixed})).into_response(),
        Err(error) => failure(error),
    }
}
async fn select_proxy(
    State(state): State<ControllerState>,
    Path(group): Path<String>,
    Json(selection): Json<Selection>,
) -> Response {
    match state
        .management
        .select_proxy(&group, Some(&selection.name))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn unfix_proxy(State(state): State<ControllerState>, Path(group): Path<String>) -> Response {
    match state.management.select_proxy(&group, None).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn flush_dns(State(state): State<ControllerState>) -> Response {
    match state.management.flush_dns(false).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn flush_fake_ip(State(state): State<ControllerState>) -> Response {
    match state.management.flush_dns(true).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
#[derive(Deserialize)]
struct DelayQuery {
    url: String,
    timeout: Option<u64>,
}
#[derive(Deserialize)]
struct HealthQuery {
    url: Option<String>,
    timeout: Option<u64>,
    expected: Option<String>,
}
async fn refresh_provider(
    State(state): State<ControllerState>,
    Path((kind, name)): Path<(String, String)>,
) -> Response {
    match state.management.refresh_provider(&kind, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn provider_health(
    State(state): State<ControllerState>,
    Path(name): Path<String>,
    Query(query): Query<HealthQuery>,
) -> Response {
    match state
        .management
        .provider_health(
            &name,
            None,
            query.url.as_deref(),
            Duration::from_millis(query.timeout.unwrap_or(30000).min(30000)),
            query.expected.as_deref(),
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}
async fn provider_proxy_health(
    State(state): State<ControllerState>,
    Path((name, proxy)): Path<(String, String)>,
    Query(query): Query<HealthQuery>,
) -> Response {
    match state
        .management
        .provider_health(
            &name,
            Some(&proxy),
            query.url.as_deref(),
            Duration::from_millis(query.timeout.unwrap_or(5000).min(30000)),
            query.expected.as_deref(),
        )
        .await
    {
        Ok(Some(delay)) => Json(serde_json::json!({"delay":delay})).into_response(),
        Ok(None) => failure(EngineError::InvalidConfig(
            "provider response missing delay".into(),
        )),
        Err(error) => failure(error),
    }
}
async fn proxy_delay(
    State(state): State<ControllerState>,
    Path(name): Path<String>,
    Query(query): Query<DelayQuery>,
) -> Response {
    match state
        .management
        .proxy_delay(
            &name,
            &query.url,
            Duration::from_millis(query.timeout.unwrap_or(5000).min(30000)),
        )
        .await
    {
        Ok(delay) => Json(serde_json::json!({"delay":delay})).into_response(),
        Err(error) => failure(error),
    }
}

#[derive(Deserialize)]
struct EchoRequest {
    url: String,
    payload: String,
    timeout: u64,
}
async fn proxy_echo(
    State(state): State<ControllerState>,
    Path(name): Path<String>,
    Json(request): Json<EchoRequest>,
) -> Response {
    match state
        .management
        .proxy_echo(
            &name,
            &request.url,
            &request.payload,
            Duration::from_millis(request.timeout.min(30000)),
        )
        .await
    {
        Ok(payload) => Json(serde_json::json!({"payload":payload})).into_response(),
        Err(error) => failure(error),
    }
}

async fn provider_members(
    State(state): State<ControllerState>,
    Path((_kind, name)): Path<(String, String)>,
) -> Response {
    match state.management.provider_members(&name).await {
        Ok(members) => {
            let proxies=members.into_iter().map(|proxy|serde_json::json!({"name":proxy.name,"type":proxy.proxy_type,"alive":proxy.alive,"history":[{"delay":proxy.delay_ms.unwrap_or(0)}]})).collect::<Vec<_>>();
            Json(serde_json::json!({"proxies":proxies})).into_response()
        }
        Err(error) => failure(error),
    }
}
