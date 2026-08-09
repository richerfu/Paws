use futures::StreamExt;
use once_cell::sync::Lazy;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, USER_AGENT};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const EXTERNAL_HTTP_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const EXTERNAL_HTTP_ERROR_BODY_BYTES: usize = 64 * 1024;
const APP_USER_AGENT: &str = concat!("Paws/", env!("CARGO_PKG_VERSION"));
const SUBSCRIPTION_USER_AGENT: &str = concat!("clash.meta/0.19.0 Paws/", env!("CARGO_PKG_VERSION"));
const SUBSCRIPTION_ACCEPT: &str =
    "application/yaml, text/yaml, text/plain, application/octet-stream, */*";

static SHARED_EXTERNAL_HTTP: Lazy<Result<ExternalHttpClientPool, ExternalHttpError>> =
    Lazy::new(ExternalHttpClientPool::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalHttpRoute {
    Direct,
    CurrentProxy,
}

impl fmt::Display for ExternalHttpRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Direct => "direct",
            Self::CurrentProxy => "current proxy",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRequestKind {
    Generic,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalHttpError {
    message: String,
}

impl ExternalHttpError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExternalHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExternalHttpError {}

#[derive(Debug)]
pub struct ExternalTextResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub final_url: Url,
    pub body: String,
}

#[derive(Clone)]
pub struct ExternalHttpClientPool {
    direct: Client,
    cookie_jar: Arc<Jar>,
    proxied: Arc<Mutex<HashMap<String, Client>>>,
}

impl ExternalHttpClientPool {
    fn new() -> Result<Self, ExternalHttpError> {
        let cookie_jar = Arc::new(Jar::default());
        let direct = build_client(cookie_jar.clone(), None)?;
        Ok(Self {
            direct,
            cookie_jar,
            proxied: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn request(
        &self,
        method: Method,
        url: &str,
        proxy_url: Option<&str>,
        kind: ExternalRequestKind,
    ) -> Result<RequestBuilder, ExternalHttpError> {
        let client = self.client(proxy_url)?;
        let request = client.request(method, url);
        Ok(match kind {
            ExternalRequestKind::Generic => request,
            ExternalRequestKind::Subscription => request
                .header(USER_AGENT, SUBSCRIPTION_USER_AGENT)
                .header(ACCEPT, SUBSCRIPTION_ACCEPT)
                .header(CACHE_CONTROL, "no-cache"),
        })
    }

    fn client(&self, proxy_url: Option<&str>) -> Result<Client, ExternalHttpError> {
        let Some(proxy_url) = proxy_url else {
            return Ok(self.direct.clone());
        };
        let mut clients = self
            .proxied
            .lock()
            .map_err(|_| ExternalHttpError::new("external HTTP proxy cache lock poisoned"))?;
        if let Some(client) = clients.get(proxy_url) {
            return Ok(client.clone());
        }
        let client = build_client(self.cookie_jar.clone(), Some(proxy_url))?;
        clients.insert(proxy_url.to_owned(), client.clone());
        Ok(client)
    }
}

pub fn shared_external_http_client() -> Result<&'static ExternalHttpClientPool, ExternalHttpError> {
    match &*SHARED_EXTERNAL_HTTP {
        Ok(client) => Ok(client),
        Err(error) => Err(error.clone()),
    }
}

pub async fn read_external_text_response(
    response: Response,
    context: &str,
    route: ExternalHttpRoute,
    max_body_bytes: usize,
) -> Result<ExternalTextResponse, ExternalHttpError> {
    let status = response.status();
    let headers = response.headers().clone();
    let final_url = response.url().clone();
    let limit = if status.is_success() {
        max_body_bytes
    } else {
        EXTERNAL_HTTP_ERROR_BODY_BYTES
    };
    let body = read_limited_body(response, limit, context).await?;
    if !status.is_success() {
        return Err(http_status_error(context, route, status, &headers, &body));
    }
    Ok(ExternalTextResponse {
        status,
        headers,
        final_url,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn build_client(
    cookie_jar: Arc<Jar>,
    proxy_url: Option<&str>,
) -> Result<Client, ExternalHttpError> {
    let mut default_headers = HeaderMap::new();
    default_headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    default_headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN, zh;q=0.9, en;q=0.6"),
    );
    let mut builder = Client::builder()
        .user_agent(APP_USER_AGENT)
        .default_headers(default_headers)
        .cookie_provider(cookie_jar)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .tcp_keepalive(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10));
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|error| ExternalHttpError::new(format!("invalid local proxy URL: {error}")))?;
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|error| {
        ExternalHttpError::new(format!("external HTTP client init failed: {error}"))
    })
}

async fn read_limited_body(
    response: Response,
    max_body_bytes: usize,
    context: &str,
) -> Result<Vec<u8>, ExternalHttpError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_body_bytes as u64)
    {
        return Err(ExternalHttpError::new(format!(
            "{context} response is larger than {max_body_bytes} bytes"
        )));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            ExternalHttpError::new(format!(
                "{context} response body read failed: {}",
                error.without_url()
            ))
        })?;
        if body.len().saturating_add(chunk.len()) > max_body_bytes {
            return Err(ExternalHttpError::new(format!(
                "{context} response is larger than {max_body_bytes} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn http_status_error(
    context: &str,
    route: ExternalHttpRoute,
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> ExternalHttpError {
    let server = safe_header(headers, "server");
    let cf_ray = safe_header(headers, "cf-ray");
    let cf_mitigated = safe_header(headers, "cf-mitigated");
    let lower_body = String::from_utf8_lossy(body).to_ascii_lowercase();
    let cloudflare = server
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("cloudflare"))
        || cf_ray.is_some()
        || lower_body.contains("cloudflare");
    let challenge = cf_mitigated
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("challenge"))
        || lower_body.contains("cf-chl-")
        || lower_body.contains("just a moment...");

    let mut details = vec![format!("route={route}")];
    if cloudflare {
        details.push(if challenge {
            "provider=Cloudflare challenge".to_owned()
        } else {
            "provider=Cloudflare".to_owned()
        });
    }
    if let Some(ray) = cf_ray {
        details.push(format!("cf-ray={ray}"));
    }
    if let Some(mitigated) = cf_mitigated {
        details.push(format!("cf-mitigated={mitigated}"));
    }
    if !cloudflare {
        if let Some(server) = server {
            details.push(format!("server={server}"));
        }
    }
    ExternalHttpError::new(format!(
        "{context} failed with HTTP {status} ({})",
        details.join(", ")
    ))
}

fn safe_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::http::header::{COOKIE, SET_COOKIE};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    #[test]
    fn cloudflare_error_diagnostics_do_not_echo_body() {
        let mut headers = HeaderMap::new();
        headers.insert("server", HeaderValue::from_static("cloudflare"));
        headers.insert("cf-ray", HeaderValue::from_static("abc123-SIN"));
        headers.insert("cf-mitigated", HeaderValue::from_static("challenge"));
        let error = http_status_error(
            "profile download",
            ExternalHttpRoute::Direct,
            StatusCode::FORBIDDEN,
            &headers,
            b"secret-token cf-chl-page",
        );
        let message = error.to_string();
        assert!(message.contains("Cloudflare challenge"));
        assert!(message.contains("cf-ray=abc123-SIN"));
        assert!(!message.contains("secret-token"));
    }

    #[tokio::test]
    async fn subscription_requests_use_compatible_ua_and_retain_cookies() {
        async fn handler(request: Request) -> impl IntoResponse {
            let user_agent = request
                .headers()
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let has_cookie = request
                .headers()
                .get(COOKIE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("paws_session=ready"));
            let mut headers = HeaderMap::new();
            if !has_cookie {
                headers.insert(
                    SET_COOKIE,
                    HeaderValue::from_static("paws_session=ready; Path=/; HttpOnly"),
                );
            }
            (headers, format!("ua={user_agent}; cookie={has_cookie}"))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/", get(handler)))
                .await
                .unwrap();
        });
        let pool = ExternalHttpClientPool::new().unwrap();
        let url = format!("http://{address}/");

        for (index, expected_cookie) in [false, true].into_iter().enumerate() {
            let response = pool
                .request(Method::GET, &url, None, ExternalRequestKind::Subscription)
                .unwrap()
                .send()
                .await
                .unwrap();
            let response = read_external_text_response(
                response,
                "test subscription",
                ExternalHttpRoute::Direct,
                1024,
            )
            .await
            .unwrap();
            assert!(response.body.contains("ua=clash.meta/0.19.0 Paws/"));
            assert_eq!(
                response.body.contains("cookie=true"),
                expected_cookie,
                "request {index}"
            );
        }
        task.abort();
    }
}
