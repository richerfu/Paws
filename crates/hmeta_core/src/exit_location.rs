use hmeta_model::{ExitLocationSnapshot, HMetaError};
use serde::Deserialize;
use std::net::IpAddr;
use std::time::Duration;

const EXIT_LOCATION_URL: &str = "https://ipwho.is/?fields=success,message,ip,country,country_code";
const EXIT_LOCATION_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Deserialize)]
struct ExitLocationResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    country_code: String,
    #[serde(default)]
    message: String,
}

pub(super) async fn probe_exit_location() -> Result<ExitLocationSnapshot, HMetaError> {
    // The UI and VPN extension are separate processes. Only the extension is
    // protected from its own TUN, so an ordinary request from the UI process
    // naturally traverses the active system VPN and observes its real exit.
    // Dialling a meow adapter here would make the adapter socket enter the TUN
    // again and create a nested proxy loop.
    tokio::time::timeout(EXIT_LOCATION_TIMEOUT, async move {
        let response = reqwest::Client::new()
            .get(EXIT_LOCATION_URL)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| HMetaError::Core(format!("exit location request failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(HMetaError::Core(format!(
                "exit location request failed with HTTP {status}"
            )));
        }
        let body = response.bytes().await.map_err(|error| {
            HMetaError::Core(format!("exit location response read failed: {error}"))
        })?;
        parse_exit_location_response(body.as_ref())
    })
    .await
    .map_err(|_| HMetaError::Core("exit location request timed out".to_owned()))?
}

fn parse_exit_location_response(body: &[u8]) -> Result<ExitLocationSnapshot, HMetaError> {
    let response: ExitLocationResponse = serde_json::from_slice(body)
        .map_err(|error| HMetaError::Core(format!("invalid exit location response: {error}")))?;
    if !response.success {
        let reason = response.message.trim();
        return Err(HMetaError::Core(if reason.is_empty() {
            "exit location service rejected the request".to_owned()
        } else {
            format!("exit location service rejected the request: {reason}")
        }));
    }

    let ip = response
        .ip
        .trim()
        .parse::<IpAddr>()
        .map_err(|error| HMetaError::Core(format!("invalid exit IP address: {error}")))?
        .to_string();
    let country_code = response.country_code.trim().to_ascii_uppercase();
    if country_code.len() != 2 || !country_code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(HMetaError::Core(
            "exit location response has no valid country code".to_owned(),
        ));
    }
    let country = response.country.trim();
    if country.is_empty() {
        return Err(HMetaError::Core(
            "exit location response has no country".to_owned(),
        ));
    }

    Ok(ExitLocationSnapshot {
        ip,
        country: country.to_owned(),
        country_code,
        updated_at: Some(super::unix_timestamp_string()),
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_exit_location() {
        let location = parse_exit_location_response(
            br#"{"success":true,"ip":"203.0.113.9","country":"Japan","country_code":"jp"}"#,
        )
        .expect("valid exit location");

        assert_eq!(location.ip, "203.0.113.9");
        assert_eq!(location.country, "Japan");
        assert_eq!(location.country_code, "JP");
        assert!(location.updated_at.is_some());
        assert!(location.error.is_none());
    }

    #[test]
    fn rejects_unsuccessful_or_invalid_responses() {
        let rejected =
            parse_exit_location_response(br#"{"success":false,"message":"rate limited"}"#)
                .expect_err("service rejection");
        assert!(rejected.to_string().contains("rate limited"));

        let invalid_ip = parse_exit_location_response(
            br#"{"success":true,"ip":"10.0.0.1/30","country":"Japan","country_code":"JP"}"#,
        )
        .expect_err("TUN address is not a public IP response");
        assert!(invalid_ip.to_string().contains("invalid exit IP"));
    }
}
