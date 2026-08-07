use futures::{stream::FuturesUnordered, StreamExt};
use hmeta_model::{ExitIpServiceSummary, ExitLocationSnapshot, HMetaError};
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;

const EXIT_LOCATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_LOCATION_OVERALL_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_EXIT_LOCATION_RESPONSE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct ExitLocationProvider {
    name: &'static str,
    url: &'static str,
    documentation_url: &'static str,
    parser: fn(&[u8]) -> Result<ExitLocationSnapshot, String>,
}

const EXIT_LOCATION_PROVIDERS: &[ExitLocationProvider] = &[
    ExitLocationProvider {
        name: "IPWho.is",
        url: "https://ipwho.is/?fields=success,message,ip,country,country_code",
        documentation_url: "https://ipwhois.io/documentation",
        parser: parse_ipwho,
    },
    ExitLocationProvider {
        name: "MyIP.com",
        url: "https://api.myip.com/",
        documentation_url: "https://www.myip.com/api-docs/",
        parser: parse_my_ip,
    },
    ExitLocationProvider {
        name: "ipapi.co",
        url: "https://ipapi.co/json/",
        documentation_url: "https://ipapi.co/api/",
        parser: parse_ipapi_co,
    },
    ExitLocationProvider {
        name: "ident.me",
        url: "https://ident.me/json",
        documentation_url: "https://api.ident.me/",
        parser: parse_ident_me,
    },
    ExitLocationProvider {
        name: "IP.SB",
        url: "https://api.ip.sb/geoip",
        documentation_url: "https://ip.sb/api/",
        parser: parse_ip_sb,
    },
    ExitLocationProvider {
        name: "IPinfo",
        url: "https://ipinfo.io/json",
        documentation_url: "https://ipinfo.io/developers",
        parser: parse_ipinfo,
    },
];

struct ExitLocationProbe<'a> {
    client: reqwest::Client,
    route_label: &'a str,
    providers: &'a [ExitLocationProvider],
}

impl<'a> ExitLocationProbe<'a> {
    fn through_proxy(
        proxy_url: &'a str,
        providers: &'a [ExitLocationProvider],
    ) -> Result<Self, HMetaError> {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|error| {
            HMetaError::Core(format!("invalid exit location proxy URL: {error}"))
        })?;
        let client = reqwest::Client::builder()
            .proxy(proxy)
            .user_agent(concat!("Paws/", env!("CARGO_PKG_VERSION")))
            .timeout(EXIT_LOCATION_REQUEST_TIMEOUT)
            .build()
            .map_err(|error| {
                HMetaError::Core(format!("exit location client setup failed: {error}"))
            })?;
        Ok(Self {
            client,
            route_label: proxy_url,
            providers,
        })
    }

    fn through_system_vpn(providers: &'a [ExitLocationProvider]) -> Result<Self, HMetaError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .user_agent(concat!("Paws/", env!("CARGO_PKG_VERSION")))
            .timeout(EXIT_LOCATION_REQUEST_TIMEOUT)
            .build()
            .map_err(|error| {
                HMetaError::Core(format!(
                    "exit location fallback client setup failed: {error}"
                ))
            })?;
        Ok(Self {
            client,
            route_label: "the system VPN",
            providers,
        })
    }

    async fn run(&self) -> Result<ExitLocationSnapshot, HMetaError> {
        tokio::time::timeout(EXIT_LOCATION_OVERALL_TIMEOUT, async {
            let mut probes = FuturesUnordered::new();
            for provider in self.providers.iter().copied() {
                probes.push(self.probe_provider(provider));
            }

            let mut failures = Vec::with_capacity(self.providers.len());
            while let Some(result) = probes.next().await {
                match result {
                    Ok(location) => return Ok(location),
                    Err(error) => failures.push(error),
                }
            }
            failures.sort_unstable();
            Err(HMetaError::Core(format!(
                "all exit location providers failed through {}: {}",
                self.route_label,
                failures.join(" | ")
            )))
        })
        .await
        .map_err(|_| {
            HMetaError::Core(format!(
                "exit location providers timed out through {}",
                self.route_label
            ))
        })?
    }

    async fn probe_provider(
        &self,
        provider: ExitLocationProvider,
    ) -> Result<ExitLocationSnapshot, String> {
        let response = self
            .client
            .get(provider.url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| format!("{} request failed: {error}", provider.name))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{} returned HTTP {status}", provider.name));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_EXIT_LOCATION_RESPONSE_BYTES)
        {
            return Err(format!("{} response is too large", provider.name));
        }
        let mut body = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk
                .map_err(|error| format!("{} response read failed: {error}", provider.name))?;
            if body.len().saturating_add(chunk.len()) as u64 > MAX_EXIT_LOCATION_RESPONSE_BYTES {
                return Err(format!("{} response is too large", provider.name));
            }
            body.extend_from_slice(&chunk);
        }
        let mut location = (provider.parser)(&body)
            .map_err(|error| format!("{} response is invalid: {error}", provider.name))?;
        location.provider = Some(provider.name.to_owned());
        Ok(location)
    }
}

pub(super) fn exit_ip_service_summaries() -> Vec<ExitIpServiceSummary> {
    EXIT_LOCATION_PROVIDERS
        .iter()
        .map(|provider| ExitIpServiceSummary {
            name: provider.name.to_owned(),
            documentation_url: provider.documentation_url.to_owned(),
        })
        .collect()
}

pub(super) async fn probe_exit_location(
    mixed_port: u16,
) -> Result<ExitLocationSnapshot, HMetaError> {
    let proxy_url = format!("http://127.0.0.1:{mixed_port}");
    probe_exit_location_through(&proxy_url, EXIT_LOCATION_PROVIDERS).await
}

async fn probe_exit_location_through(
    proxy_url: &str,
    providers: &[ExitLocationProvider],
) -> Result<ExitLocationSnapshot, HMetaError> {
    let proxy_error = match ExitLocationProbe::through_proxy(proxy_url, providers)?
        .run()
        .await
    {
        Ok(location) => return Ok(location),
        Err(error) => error,
    };
    ExitLocationProbe::through_system_vpn(providers)?
        .run()
        .await
        .map_err(|fallback_error| {
            HMetaError::Core(format!(
                "mixed proxy path failed ({proxy_error}); system VPN fallback failed ({fallback_error})"
            ))
        })
}

fn parse_ipwho(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let reason = optional_string(&value, "message").unwrap_or("request rejected");
        return Err(reason.to_owned());
    }
    parse_location_fields(&value, "ip", Some("country"), "country_code")
}

fn parse_my_ip(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    parse_location_fields(&value, "ip", Some("country"), "cc")
}

fn parse_ipapi_co(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    if value.get("error").and_then(Value::as_bool) == Some(true) {
        let reason = optional_string(&value, "reason").unwrap_or("request rejected");
        return Err(reason.to_owned());
    }
    parse_location_fields(&value, "ip", Some("country_name"), "country_code")
}

fn parse_ident_me(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    parse_location_fields(&value, "ip", Some("country"), "cc")
}

fn parse_ip_sb(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    parse_location_fields(&value, "ip", Some("country"), "country_code")
}

fn parse_ipinfo(body: &[u8]) -> Result<ExitLocationSnapshot, String> {
    let value = parse_json(body)?;
    parse_location_fields(&value, "ip", None, "country")
}

fn parse_json(body: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    if !value.is_object() {
        return Err("JSON root is not an object".to_owned());
    }
    Ok(value)
}

fn parse_location_fields(
    value: &Value,
    ip_field: &str,
    country_field: Option<&str>,
    country_code_field: &str,
) -> Result<ExitLocationSnapshot, String> {
    let ip = required_string(value, ip_field)?
        .parse::<IpAddr>()
        .map_err(|error| format!("invalid IP address: {error}"))?
        .to_string();
    let country_code = required_string(value, country_code_field)?.to_ascii_uppercase();
    if country_code.len() != 2 || !country_code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err("missing or invalid country code".to_owned());
    }
    let country = country_field
        .and_then(|field| optional_string(value, field))
        .unwrap_or_default()
        .to_owned();

    Ok(ExitLocationSnapshot {
        ip,
        country,
        country_code,
        updated_at: Some(super::unix_timestamp_string()),
        error: None,
        provider: None,
    })
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    optional_string(value, field).ok_or_else(|| format!("missing {field}"))
}

fn optional_string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn parses_supported_provider_responses() {
        let cases = [
            (
                parse_ipwho as fn(&[u8]) -> Result<ExitLocationSnapshot, String>,
                br#"{"success":true,"ip":"203.0.113.9","country":"Japan","country_code":"jp"}"#
                    .as_slice(),
            ),
            (
                parse_my_ip,
                br#"{"ip":"203.0.113.9","country":"Japan","cc":"JP"}"#.as_slice(),
            ),
            (
                parse_ipapi_co,
                br#"{"ip":"203.0.113.9","country_name":"Japan","country_code":"JP"}"#.as_slice(),
            ),
            (
                parse_ident_me,
                br#"{"ip":"203.0.113.9","country":"Japan","cc":"JP"}"#.as_slice(),
            ),
            (
                parse_ip_sb,
                br#"{"ip":"203.0.113.9","country":"Japan","country_code":"JP"}"#.as_slice(),
            ),
            (
                parse_ipinfo,
                br#"{"ip":"203.0.113.9","country":"JP"}"#.as_slice(),
            ),
        ];

        for (parse, body) in cases {
            let location = parse(body).expect("valid provider response");
            assert_eq!(location.ip, "203.0.113.9");
            assert_eq!(location.country_code, "JP");
            assert!(location.updated_at.is_some());
            assert!(location.error.is_none());
        }
    }

    #[test]
    fn rejects_provider_errors_and_invalid_locations() {
        let rejected = parse_ipwho(br#"{"success":false,"message":"rate limited"}"#)
            .expect_err("service rejection");
        assert!(rejected.contains("rate limited"));

        let invalid_ip = parse_ipwho(
            br#"{"success":true,"ip":"10.0.0.1/30","country":"Japan","country_code":"JP"}"#,
        )
        .expect_err("TUN address is not a public IP response");
        assert!(invalid_ip.contains("invalid IP"));
    }

    #[tokio::test]
    async fn queries_all_providers_through_the_explicit_proxy_and_uses_a_fallback() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fake HTTP proxy");
        let proxy_addr = listener.local_addr().expect("fake proxy address");
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept proxy request");
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 2048];
                    let read = stream.read(&mut buffer).await.expect("read proxy request");
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let response = if request.contains("fallback.test") {
                        let body = r#"{"ip":"198.51.100.7","country":"Japan","cc":"JP"}"#;
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                    };
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write proxy response");
                });
            }
        });

        let providers = [
            ExitLocationProvider {
                name: "failed",
                url: "http://failed.test/location",
                documentation_url: "https://failed.test/docs",
                parser: parse_my_ip,
            },
            ExitLocationProvider {
                name: "fallback",
                url: "http://fallback.test/location",
                documentation_url: "https://fallback.test/docs",
                parser: parse_my_ip,
            },
        ];
        let proxy_url = format!("http://{proxy_addr}");
        let location = ExitLocationProbe::through_proxy(&proxy_url, &providers)
            .expect("construct exit location probe")
            .run()
            .await
            .expect("fallback provider succeeds through the proxy");

        assert_eq!(location.ip, "198.51.100.7");
        assert_eq!(location.country_code, "JP");
        assert_eq!(location.provider.as_deref(), Some("fallback"));
    }

    #[tokio::test]
    async fn falls_back_to_the_system_vpn_when_the_local_proxy_is_unavailable() {
        let unavailable = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("reserve unavailable proxy address")
            .local_addr()
            .expect("unavailable proxy address");

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind direct provider");
        let provider_addr = listener.local_addr().expect("direct provider address");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept direct request");
            let mut buffer = [0_u8; 2048];
            let bytes_read = stream.read(&mut buffer).await.expect("read direct request");
            assert!(bytes_read > 0, "direct request must not be empty");
            let body = r#"{"ip":"198.51.100.8","country":"Japan","cc":"JP"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write direct response");
        });

        let provider_url = Box::leak(format!("http://{provider_addr}/location").into_boxed_str());
        let providers = [ExitLocationProvider {
            name: "direct fallback",
            url: provider_url,
            documentation_url: "https://fallback.test/docs",
            parser: parse_my_ip,
        }];
        let location = probe_exit_location_through(&format!("http://{unavailable}"), &providers)
            .await
            .expect("system VPN fallback succeeds");

        assert_eq!(location.ip, "198.51.100.8");
        assert_eq!(location.provider.as_deref(), Some("direct fallback"));
    }
}
