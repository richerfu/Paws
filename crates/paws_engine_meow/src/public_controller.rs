//! User-facing Clash API. The private credentialed management endpoint is not
//! exposed here. Public mutations call the actual typed owner directly, so
//! cancellation cannot leave a detached private HTTP handler running.
use crate::management::MeowManagement;
use axum::{
    body::{to_bytes, Body},
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        FromRequestParts, Request, State,
    },
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use futures::{stream, StreamExt};
use paws_engine::{EngineError, EngineManagement, EngineNetworkServices, EngineProviderKind};
use paws_model::VpnEngine;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{watch, Semaphore},
    task::{JoinHandle, JoinSet},
};

const REQUEST_LIMIT: usize = 64 * 1024;
const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

struct PublicState {
    management: Arc<MeowManagement>,
    network: EngineNetworkServices,
    active: AtomicBool,
    cancelled: watch::Sender<bool>,
    sockets: Mutex<JoinSet<()>>,
    socket_slots: Arc<Semaphore>,
    http_traffic: Mutex<TrafficSampler>,
}

pub(super) struct PublicController {
    pub address: SocketAddr,
    state: Arc<PublicState>,
    task: Option<JoinHandle<()>>,
}

impl PublicController {
    pub async fn start(
        management: Arc<MeowManagement>,
        network: EngineNetworkServices,
    ) -> Result<Self, EngineError> {
        network.validate()?;
        if !network.ports.controller_enabled {
            return Err(error("public controller is disabled"));
        }
        let requested = SocketAddr::from((
            if network.access.allow_lan {
                Ipv4Addr::UNSPECIFIED
            } else {
                Ipv4Addr::LOCALHOST
            },
            network.ports.controller_port,
        ));
        let listener = tokio::net::TcpListener::bind(requested)
            .await
            .map_err(|_| error("could not bind public controller port"))?;
        let address = listener.local_addr()?;
        let (cancelled, mut cancellation) = watch::channel(false);
        let state = Arc::new(PublicState {
            management,
            network,
            active: AtomicBool::new(true),
            cancelled,
            sockets: Mutex::new(JoinSet::new()),
            socket_slots: Arc::new(Semaphore::new(32)),
            http_traffic: Mutex::new(TrafficSampler::default()),
        });
        let router = Router::new().fallback(dispatch).with_state(state.clone());
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let shutdown = async move {
                PublicState::wait_cancelled(&mut cancellation).await;
            };
            if axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .is_err()
            {
                tracing::warn!("public meow controller stopped with a transport error");
            }
            server_state.active.store(false, Ordering::Release);
            server_state.cancelled.send_replace(true);
        });
        Ok(Self {
            address,
            state,
            task: Some(task),
        })
    }

    pub fn revoke(&self) {
        self.state.active.store(false, Ordering::Release);
        self.state.cancelled.send_replace(true);
    }

    pub fn is_running(&self) -> bool {
        self.state.available() && self.task.as_ref().is_some_and(|task| !task.is_finished())
    }

    pub async fn stop(mut self) {
        self.revoke();
        // Upgraded sockets are outside axum's HTTP graceful-drain future.
        let mut sockets = {
            let mut sockets = self
                .state
                .sockets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *sockets)
        };
        sockets.abort_all();
        while sockets.join_next().await.is_some() {}
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for PublicController {
    fn drop(&mut self) {
        self.revoke();
        if let Some(task) = &self.task {
            task.abort();
        }
        if let Ok(mut sockets) = self.state.sockets.lock() {
            sockets.abort_all();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Version,
    Config,
    Proxies,
    Proxy,
    Connections,
    Rules,
    Memory,
    Traffic,
    Logs,
    Providers(EngineProviderKind),
    Provider(EngineProviderKind),
    Forward,
}

struct PublicRequest {
    operation: Operation,
    parameters: Vec<String>,
    query: BTreeMap<String, String>,
    websocket: bool,
}

impl PublicRequest {
    fn parse(request: &Request) -> Result<Self, StatusCode> {
        let path = request.uri().path();
        if path.len() > 4096 || !path.starts_with('/') {
            return Err(StatusCode::NOT_FOUND);
        }
        let raw: Vec<_> = path[1..].split('/').collect();
        let mut decoded = Vec::with_capacity(raw.len());
        for part in &raw {
            let value = percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            if value.is_empty()
                || matches!(value.as_ref(), "." | "..")
                || value.contains(['\\', '%'])
                || value.chars().any(char::is_control)
            {
                return Err(StatusCode::NOT_FOUND);
            }
            decoded.push(value.into_owned());
        }
        // Static path segments must be literal, so encoded aliases cannot
        // acquire an internal route through an extra decoding layer.
        let root = raw[0];
        if root != decoded[0] || root == "paws" {
            return Err(StatusCode::NOT_FOUND);
        }
        let method = request.method();
        let get = method == Method::GET || method == Method::OPTIONS;
        let operation = match (root, raw.len()) {
            ("version", 1) if get => Operation::Version,
            ("configs", 1) if get => Operation::Config,
            ("proxies", 1) if get => Operation::Proxies,
            ("proxies", 2) if get => Operation::Proxy,
            ("proxies", 2) if matches!(*method, Method::PUT | Method::DELETE) => Operation::Forward,
            ("proxies", 3) if raw[2] == "delay" && get => Operation::Forward,
            ("connections", 1) if get => Operation::Connections,
            ("connections", 1 | 2) if method == Method::DELETE || method == Method::OPTIONS => {
                Operation::Forward
            }
            ("rules", 1) if get => Operation::Rules,
            ("memory", 1) if get => Operation::Memory,
            ("traffic", 1) if get => Operation::Traffic,
            ("logs", 1) if get => Operation::Logs,
            ("providers", 2 | 3) if matches!(raw[1], "proxies" | "rules") && get => {
                let kind = if raw[1] == "proxies" {
                    EngineProviderKind::Proxy
                } else {
                    EngineProviderKind::Rule
                };
                if raw.len() == 2 {
                    Operation::Providers(kind)
                } else {
                    Operation::Provider(kind)
                }
            }
            ("providers", 3) if matches!(raw[1], "proxies" | "rules") && method == Method::PUT => {
                Operation::Forward
            }
            ("providers", 4 | 5)
                if raw[1] == "proxies" && raw.last() == Some(&"healthcheck") && get =>
            {
                Operation::Forward
            }
            ("cache", 3)
                if matches!(raw[1], "dns" | "fakeip")
                    && raw[2] == "flush"
                    && (method == Method::POST || method == Method::OPTIONS) =>
            {
                Operation::Forward
            }
            _ => return Err(StatusCode::NOT_FOUND),
        };
        let websocket = request
            .headers()
            .get(header::UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
        if websocket
            && (!matches!(
                operation,
                Operation::Traffic | Operation::Logs | Operation::Memory | Operation::Connections
            ) || method != Method::GET)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mut query = BTreeMap::new();
        let encoded = request.uri().query().unwrap_or_default();
        if encoded.len() > 8192 {
            return Err(StatusCode::BAD_REQUEST);
        }
        for (key, value) in url::form_urlencoded::parse(encoded.as_bytes()) {
            if query.insert(key.into_owned(), value.into_owned()).is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        for key in query.keys() {
            let allowed = match key.as_str() {
                "token" => websocket,
                "interval" => matches!(
                    operation,
                    Operation::Connections | Operation::Traffic | Operation::Memory
                ),
                "level" => operation == Operation::Logs,
                "url" | "timeout" | "expected" => {
                    operation == Operation::Forward
                        && matches!(raw.last(), Some(&"delay" | &"healthcheck"))
                }
                _ => false,
            };
            if !allowed {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        Ok(Self {
            operation,
            parameters: decoded,
            query,
            websocket,
        })
    }

    fn authorized(&self, request: &Request, secret: Option<&str>) -> bool {
        let Some(secret) = secret.filter(|value| !value.is_empty()) else {
            return true;
        };
        let mut headers = request.headers().get_all(header::AUTHORIZATION).iter();
        let first = headers.next();
        if headers.next().is_some() {
            return false;
        }
        let supplied = if let Some(value) = first {
            value
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("Bearer "))
        } else if self.websocket {
            self.query.get("token").map(String::as_str)
        } else {
            None
        };
        supplied.is_some_and(|value| secret_equal(value.as_bytes(), secret.as_bytes()))
    }
}

fn secret_equal(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    left.ct_eq(right).into()
}

async fn dispatch(State(state): State<Arc<PublicState>>, mut request: Request) -> Response {
    if !state.available() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let public = match PublicRequest::parse(&request) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static("*"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            header::HeaderValue::from_static("authorization,content-type"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            header::HeaderValue::from_static("GET,PUT,DELETE,POST,OPTIONS"),
        );
        return response;
    }
    if !public.authorized(&request, state.network.access.secret.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if public.websocket {
        let permit = match state.socket_slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        // Cut the independent stream before HTTP 101 reaches the caller.
        // An event emitted after a successful handshake must not disappear in
        // an asynchronously delayed first journal read.
        let initial_cursor = if public.operation == Operation::Logs {
            match state
                .execute(async { state.management.runtime_logs(0, 1).await.map_err(status) })
                .await
            {
                Ok(page) => page.high_watermark,
                Err(status) => return status.into_response(),
            }
        } else {
            0
        };
        let (mut parts, _) = request.into_parts();
        let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(upgrade) => upgrade,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        return upgrade
            .max_message_size(64 * 1024)
            .on_upgrade(move |socket| async move {
                if state.available() {
                    if let Ok(mut tasks) = state.sockets.lock() {
                        while tasks.try_join_next().is_some() {}
                        if state.available() {
                            let state = state.clone();
                            tasks.spawn(async move {
                                let _permit = permit;
                                state.serve_socket(socket, public, initial_cursor).await;
                            });
                        }
                    }
                }
            })
            .into_response();
    }
    let future = async {
        if public.operation == Operation::Forward {
            state.forward(&public, &mut request).await
        } else {
            state.snapshot(&public).await.and_then(json_response)
        }
    };
    let result = state.execute(future).await;
    let mut response = if state.available() {
        result.unwrap_or_else(IntoResponse::into_response)
    } else {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    };
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    response
}

impl PublicState {
    async fn wait_cancelled(receiver: &mut watch::Receiver<bool>) {
        // Return unit, not watch::Ref: an outer select must release its borrow
        // before a selected branch starts another cancellation-aware send.
        let _ = receiver.wait_for(|value| *value).await;
    }

    async fn execute<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, StatusCode>>,
    ) -> Result<T, StatusCode> {
        let mut cancelled = self.cancelled.subscribe();
        if !self.available() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        tokio::select! {
            biased;
            _ = Self::wait_cancelled(&mut cancelled) => Err(StatusCode::SERVICE_UNAVAILABLE),
            value = tokio::time::timeout(REQUEST_TIMEOUT, operation) => value.unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT)),
        }
    }

    fn available(&self) -> bool {
        self.active.load(Ordering::Acquire) && !self.management.retiring.load(Ordering::Acquire)
    }

    async fn forward(
        &self,
        public: &PublicRequest,
        request: &mut Request,
    ) -> Result<Response, StatusCode> {
        let timeout = public
            .query
            .get("timeout")
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .unwrap_or(5000)
            .clamp(1, 30000);
        let timeout = Duration::from_millis(timeout);
        let url = public.query.get("url").map(String::as_str);
        let expected = public.query.get("expected").map(String::as_str);
        let values = public
            .parameters
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        match values.as_slice() {
            ["proxies", group] if request.method() == Method::PUT => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Selection {
                    name: String,
                }
                let bytes = to_bytes(
                    std::mem::replace(request.body_mut(), Body::empty()),
                    REQUEST_LIMIT,
                )
                .await
                .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
                let selection: Selection =
                    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
                self.management
                    .select_proxy(group, Some(&selection.name))
                    .await
                    .map_err(status)?;
            }
            ["proxies", group] if request.method() == Method::DELETE => {
                self.management
                    .select_proxy(group, None)
                    .await
                    .map_err(status)?;
            }
            ["proxies", proxy, "delay"] => {
                let delay = self
                    .management
                    .proxy_delay(proxy, url.ok_or(StatusCode::BAD_REQUEST)?, timeout)
                    .await
                    .map_err(status)?;
                return Ok(axum::Json(json!({"delay":delay})).into_response());
            }
            ["connections"] => self
                .management
                .close_connections(None)
                .await
                .map_err(status)?,
            ["connections", id] => self
                .management
                .close_connections(Some(id))
                .await
                .map_err(status)?,
            ["providers", kind, name] => self
                .management
                .refresh_provider(kind, name)
                .await
                .map_err(status)?,
            ["providers", "proxies", name, "healthcheck"] => {
                self.management
                    .provider_health(name, None, url, timeout, expected)
                    .await
                    .map_err(status)?;
            }
            ["providers", "proxies", name, proxy, "healthcheck"] => {
                let delay = self
                    .management
                    .provider_health(name, Some(proxy), url, timeout, expected)
                    .await
                    .map_err(status)?
                    .ok_or(StatusCode::BAD_GATEWAY)?;
                return Ok(axum::Json(json!({"delay":delay})).into_response());
            }
            ["cache", cache, "flush"] => self
                .management
                .flush_dns(*cache == "fakeip")
                .await
                .map_err(status)?,
            _ => return Err(StatusCode::NOT_FOUND),
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    }

    async fn snapshot(&self, request: &PublicRequest) -> Result<Value, StatusCode> {
        match request.operation {
            Operation::Version => Ok(json!({"meta": false, "version": "meow-0.22.0"})),
            Operation::Config => Ok(json!({
                "mode": self.management.tunnel.mode().to_string(),
                "mixed-port": if self.network.ports.mixed_enabled { self.network.ports.mixed_port } else { 0 },
                "allow-lan": self.network.access.allow_lan,
                "external-controller": SocketAddr::from((if self.network.access.allow_lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST }, self.network.ports.controller_port)).to_string(),
            })),
            Operation::Proxy => {
                let route = self.management.tunnel.route_snapshot();
                let name = request.parameters[1].as_str();
                if let Some(proxy) = route.proxies.get(name) {
                    return Ok(proxy_value(proxy.as_ref()));
                }
                // Provider-only nodes need one lookup, never JSON materialization
                // of every unrelated group and its members.
                for provider in self.management.proxy_providers.values() {
                    if let Some(proxy) = provider
                        .proxies()
                        .into_iter()
                        .find(|proxy| proxy.name() == name)
                    {
                        return Ok(proxy_value(proxy.as_ref()));
                    }
                }
                Err(StatusCode::NOT_FOUND)
            }
            Operation::Proxies => {
                let route = self.management.tunnel.route_snapshot();
                let mut proxies = serde_json::Map::new();
                for proxy in route.proxies.values() {
                    proxies.insert(proxy.name().to_owned(), proxy_value(proxy.as_ref()));
                    for member in proxy.member_proxies().unwrap_or_default() {
                        proxies
                            .entry(member.name().to_owned())
                            .or_insert_with(|| proxy_value(member.as_ref()));
                    }
                }
                Ok(json!({"proxies": proxies}))
            }
            Operation::Connections => {
                let connections = self.management.connections().await.map_err(status)?;
                let (upload, download) = self.management.tunnel.statistics().snapshot();
                let values: Vec<_> = connections.into_iter().map(|connection| json!({
                    "id": connection.id,
                    "metadata": {"host":connection.domain,"destinationIP":connection.destination_ip,"destinationPort":connection.destination_port,"network":connection.network},
                    "upload":connection.upload_bytes,"download":connection.download_bytes,"chains":connection.chains,
                    "start":connection.started_at,"rule":connection.rule,"rulePayload":connection.rule_payload,
                })).collect();
                Ok(json!({"connections":values,"uploadTotal":upload,"downloadTotal":download}))
            }
            Operation::Rules => {
                let rules: Vec<_> = self.management.runtime_rules().await.map_err(status)?.into_iter()
                    .map(|rule| json!({"type":rule.rule_type,"payload":rule.payload,"proxy":rule.target})).collect();
                Ok(json!({"rules":rules}))
            }
            Operation::Memory => {
                let memory = self.management.memory().await.map_err(status)?;
                Ok(json!({"inuse": memory.in_use_bytes, "oslimit":memory.limit_bytes}))
            }
            Operation::Traffic => {
                let (upload, download) = self.management.tunnel.statistics().snapshot();
                let mut sampler = self
                    .http_traffic
                    .lock()
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                let (up, down) = sampler.sample(Instant::now(), totals(upload, download));
                Ok(json!({"up":up,"down":down,"uploadTotal":upload,"downloadTotal":download}))
            }
            Operation::Logs => Err(StatusCode::UPGRADE_REQUIRED),
            Operation::Providers(kind) => {
                let mut names: Vec<_> = match kind {
                    EngineProviderKind::Proxy => {
                        self.management.proxy_providers.keys().cloned().collect()
                    }
                    EngineProviderKind::Rule => {
                        self.management.rule_providers.keys().cloned().collect()
                    }
                };
                names.sort();
                let values: Vec<_> = stream::iter(names)
                    .map(|name| async move {
                        let provider = self
                            .management
                            .provider_state(kind, &name)
                            .await
                            .map_err(status)?;
                        Ok::<_, StatusCode>((name, provider_value(provider)))
                    })
                    .buffered(4)
                    .collect()
                    .await;
                let providers = values
                    .into_iter()
                    .collect::<Result<serde_json::Map<_, _>, _>>()?;
                Ok(json!({"providers":providers}))
            }
            Operation::Provider(kind) => Ok(provider_value(
                self.management
                    .provider_state(kind, &request.parameters[2])
                    .await
                    .map_err(status)?,
            )),
            Operation::Forward => Err(StatusCode::NOT_FOUND),
        }
    }

    async fn serve_socket(
        self: Arc<Self>,
        mut socket: WebSocket,
        request: PublicRequest,
        mut cursor: u64,
    ) {
        let mut cancelled = self.cancelled.subscribe();
        if !self.available() {
            return;
        }
        let interval = request
            .query
            .get("interval")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1000)
            .clamp(250, 10000);
        let mut timer = tokio::time::interval(Duration::from_millis(interval));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut traffic = TrafficSampler::default();
        loop {
            tokio::select! {
                biased;
                _ = Self::wait_cancelled(&mut cancelled) => break,
                incoming = socket.recv() => {
                    match incoming { Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break, _ => {} }
                },
                _ = timer.tick() => {
                    if !self.available() { break; }
                    let observations = async {
                        if request.operation == Operation::Logs {
                            let page = self.management.runtime_logs(cursor, 128).await.map_err(status)?;
                            cursor = page.next_cursor;
                            let minimum = request.query.get("level").map_or(0, |value| log_level(value));
                            let mut values: Vec<_> = page.entries.into_iter().filter(|entry| log_level(&entry.level) >= minimum)
                                .map(|entry| json!({"type":entry.level,"payload":entry.message})).collect();
                            if let Some(first) = page.dropped_before {
                                values.insert(0, json!({"type":"warning","payload":format!("log stream retention gap before sequence {first}")}));
                            }
                            Ok(values)
                        } else if request.operation == Operation::Traffic {
                            let totals = self.management.tunnel.statistics().snapshot();
                            let (up, down) = traffic.sample(Instant::now(), self::totals(totals.0, totals.1));
                            Ok(vec![json!({"up":up,"down":down})])
                        } else { self.snapshot(&request).await.map(|value| vec![value]) }
                    };
                    let values = match self.execute(observations).await { Ok(values) => values, Err(_) => break };
                    for value in values {
                        if !self.available() { return; }
                        let message = match encode_response(&value).and_then(|bytes| String::from_utf8(bytes).map_err(|_| StatusCode::BAD_GATEWAY)) {
                            Ok(message) => message, Err(_) => return,
                        };
                        let sent = tokio::select! {
                            biased;
                            _ = Self::wait_cancelled(&mut cancelled) => return,
                            value = tokio::time::timeout(Duration::from_secs(2), socket.send(Message::Text(message.into()))) => value,
                        };
                        if !matches!(sent, Ok(Ok(()))) { return; }
                    }
                }
            }
        }
    }
}

fn proxy_value(proxy: &dyn meow_common::Proxy) -> Value {
    let delay = proxy.last_delay();
    let mut value = json!({"name":proxy.name(),"type":proxy.adapter_type().to_string(),
        "history":if delay > 0 { vec![json!({"delay":delay})] } else { Vec::new() }});
    if let Some(members) = proxy.members() {
        value["all"] = json!(members);
        value["now"] = json!(proxy.current());
        value["fixed"] = json!(proxy.selection().and_then(|selection| selection.fixed()));
    }
    value
}

#[derive(Default)]
struct TrafficSampler {
    previous: Option<(Instant, (u64, u64))>,
}
impl TrafficSampler {
    fn sample(&mut self, now: Instant, totals: (u64, u64)) -> (u64, u64) {
        let rates = self.previous.map_or((0, 0), |(previous, counters)| {
            let elapsed = now.duration_since(previous).as_secs_f64();
            if elapsed <= 0.0 {
                return (0, 0);
            }
            (
                (totals.0.saturating_sub(counters.0) as f64 / elapsed) as u64,
                (totals.1.saturating_sub(counters.1) as f64 / elapsed) as u64,
            )
        });
        self.previous = Some((now, totals));
        rates
    }
}
fn totals(upload: meow_common::atomic::Int, download: meow_common::atomic::Int) -> (u64, u64) {
    (
        u64::try_from(upload).unwrap_or(0),
        u64::try_from(download).unwrap_or(0),
    )
}

struct ResponseBuffer(Vec<u8>);
impl std::io::Write for ResponseBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > RESPONSE_LIMIT {
            return Err(std::io::Error::other("public response exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_response(value: &Value) -> Result<Vec<u8>, StatusCode> {
    let mut buffer = ResponseBuffer(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(|_| StatusCode::BAD_GATEWAY)?;
    Ok(buffer.0)
}
fn json_response(value: Value) -> Result<Response, StatusCode> {
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        encode_response(&value)?,
    )
        .into_response())
}

fn provider_value(provider: paws_engine::EngineProviderState) -> Value {
    let mut value = json!({"name":provider.name,"vehicleType":provider.vehicle,"updatedAt":provider.updated_at});
    if let Some(members) = provider.members {
        value["proxies"] = json!(members
            .into_iter()
            .map(
                |member| json!({"name":member.name,"type":member.proxy_type,"alive":member.alive,
            "history":member.delay_ms.map(|delay|vec![json!({"delay":delay})]).unwrap_or_default()})
            )
            .collect::<Vec<_>>());
    }
    if let Some(count) = provider.rule_count {
        value["ruleCount"] = json!(count);
    }
    value
}

fn log_level(level: &str) -> u8 {
    match level.to_ascii_lowercase().as_str() {
        "trace" => 0,
        "debug" => 1,
        "info" => 2,
        "warn" | "warning" => 3,
        "error" | "fatal" => 4,
        "silent" => 5,
        _ => 0,
    }
}
fn status(error: EngineError) -> StatusCode {
    match error {
        EngineError::NotRunning => StatusCode::SERVICE_UNAVAILABLE,
        EngineError::InvalidConfig(_) => StatusCode::BAD_REQUEST,
        EngineError::Unsupported { .. } | EngineError::ObservationUnavailable { .. } => {
            StatusCode::NOT_IMPLEMENTED
        }
        _ => StatusCode::BAD_GATEWAY,
    }
}
fn error(message: &str) -> EngineError {
    EngineError::Backend {
        engine: VpnEngine::Meow,
        operation: "public controller",
        message: message.to_owned(),
    }
}

#[cfg(test)]
#[path = "public_controller_tests.rs"]
mod tests;
