use serde_json::Value;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedSubscription {
    pub(crate) url: String,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScannedSubscriptionError {
    Empty,
    Unsupported,
}

pub(crate) fn parse_scanned_subscription(
    payload: &str,
) -> Result<ScannedSubscription, ScannedSubscriptionError> {
    let payload = payload.trim_start_matches('\u{feff}').trim();
    if payload.is_empty() {
        return Err(ScannedSubscriptionError::Empty);
    }

    if let Ok(subscription) = parse_url_payload(payload) {
        return Ok(subscription);
    }

    let json = serde_json::from_str::<Value>(payload)
        .map_err(|_| ScannedSubscriptionError::Unsupported)?;
    let Value::Object(object) = json else {
        return Err(ScannedSubscriptionError::Unsupported);
    };
    let url = ["url", "subscriptionUrl", "subscription_url"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .ok_or(ScannedSubscriptionError::Unsupported)?;
    let name = ["name", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .and_then(non_empty);
    direct_subscription(url, name)
}

fn parse_url_payload(payload: &str) -> Result<ScannedSubscription, ScannedSubscriptionError> {
    let parsed = Url::parse(payload).map_err(|_| ScannedSubscriptionError::Unsupported)?;
    if matches!(parsed.scheme(), "http" | "https") {
        return direct_subscription(payload, None);
    }

    if !matches!(parsed.scheme(), "clash" | "mihomo") || !is_install_config_url(&parsed) {
        return Err(ScannedSubscriptionError::Unsupported);
    }
    let mut subscription_url = None;
    let mut name = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "url" => subscription_url = non_empty(value.as_ref()),
            "name" | "title" => name = non_empty(value.as_ref()),
            _ => {}
        }
    }
    direct_subscription(
        subscription_url
            .as_deref()
            .ok_or(ScannedSubscriptionError::Unsupported)?,
        name,
    )
}

fn is_install_config_url(url: &Url) -> bool {
    matches!(
        url.host_str().unwrap_or_default(),
        "install-config" | "import-config"
    ) || matches!(
        url.path().trim_matches('/'),
        "install-config" | "import-config"
    )
}

fn direct_subscription(
    value: &str,
    name: Option<String>,
) -> Result<ScannedSubscription, ScannedSubscriptionError> {
    let value = value.trim();
    let parsed = Url::parse(value).map_err(|_| ScannedSubscriptionError::Unsupported)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ScannedSubscriptionError::Unsupported);
    }
    Ok(ScannedSubscription {
        url: value.to_owned(),
        name,
    })
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
