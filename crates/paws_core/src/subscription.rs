use paws_model::PawsError;
use paws_profile::ProfileStore;

/// Refresh metadata changes the editor revision even when the active engine's
/// configuration is unchanged. Only this transaction's actual impact may
/// request a VPN restart.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProfileRefreshOutcome {
    pub active_config_changed: bool,
}

#[derive(PartialEq)]
pub(super) struct ProfileRefreshConfiguration(serde_yaml::Value);

impl ProfileRefreshConfiguration {
    pub(super) fn capture(profiles: &ProfileStore, profile_id: &str) -> Result<Self, PawsError> {
        // Include persistent user overrides, but ignore comments, formatting
        // and mapping order in the downloaded subscription.
        let source = profiles.raw_yaml(profile_id)?;
        serde_yaml::from_str(&source)
            .map(Self)
            .map_err(|error| PawsError::Core(error.to_string()))
    }
}

pub(super) fn profile_name_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Subscription".to_owned())
}

pub(super) fn subscription_userinfo_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<paws_model::SubscriptionUserInfo> {
    headers
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .and_then(paws_profile::parse_subscription_userinfo)
}

pub(super) fn subscription_metadata_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<paws_model::SubscriptionMetadata> {
    let content_disposition_title = header_str(headers, "content-disposition")
        .and_then(paws_profile::parse_content_disposition_filename);
    let title = header_str(headers, "profile-title").or(content_disposition_title.as_deref());
    paws_profile::parse_subscription_metadata(
        title,
        header_str(headers, "profile-update-interval"),
        header_str(headers, "profile-web-page-url"),
        header_str(headers, "support-url"),
    )
}

pub(super) fn subscription_profile_name_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<String> {
    header_str(headers, "profile-title")
        .and_then(paws_profile::decode_subscription_header_text)
        .or_else(|| {
            header_str(headers, "content-disposition")
                .and_then(paws_profile::parse_content_disposition_filename)
        })
}

pub(super) fn header_str<'a>(
    headers: &'a reqwest::header::HeaderMap,
    name: &str,
) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}
