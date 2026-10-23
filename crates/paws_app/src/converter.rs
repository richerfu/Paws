//! Subscription conversion belongs to the application layer, independent of its UI.
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use paws_core::{
    http_client::{
        read_external_text_response, shared_external_http_client, ExternalHttpRoute,
        ExternalRequestKind, EXTERNAL_HTTP_MAX_BODY_BYTES,
    },
    CoreHandle,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, fs, path::PathBuf, sync::Arc};
use url::Url;

const DEFAULT_BACKEND: &str = "https://api.wcc.best/sub?";
const DEFAULT_SHORT_URL_API: &str = "https://suosuo.de/short";
const DEFAULT_CONFIG_UPLOAD_API: &str = "https://oss.wcc.best/upload";
const DRAFT_FILE: &str = "subscription-converter.json";
const KNOWN_PARAMS: &[&str] = &[
    "target",
    "ver",
    "url",
    "insert",
    "config",
    "exclude",
    "include",
    "filename",
    "append_type",
    "emoji",
    "list",
    "tfo",
    "scv",
    "fdn",
    "expand",
    "sort",
    "udp",
    "surge.doh",
    "clash.doh",
    "new_name",
];

#[derive(Debug, thiserror::Error)]
#[error("subscription conversion failed: {0}")]
pub(crate) struct ConverterError(String);
impl From<String> for ConverterError {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl From<std::io::Error> for ConverterError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<serde_json::Error> for ConverterError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

pub(crate) struct Converter {
    core: Arc<CoreHandle>,
    home: PathBuf,
}
impl Converter {
    pub(crate) fn new(core: Arc<CoreHandle>, home: PathBuf) -> Self {
        Self { core, home }
    }
    pub(crate) fn load_draft(&self) -> Result<SubscriptionConverterDraft, ConverterError> {
        match fs::read_to_string(self.home.join(DRAFT_FILE)) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SubscriptionConverterDraft::default())
            }
            Err(error) => Err(error.into()),
        }
    }
    pub(crate) fn save_draft(
        &self,
        draft: &SubscriptionConverterDraft,
    ) -> Result<(), ConverterError> {
        fs::create_dir_all(&self.home)?;
        let path = self.home.join(DRAFT_FILE);
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(draft)?)?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        Ok(())
    }
    pub(crate) fn build_url(
        &self,
        draft: &SubscriptionConverterDraft,
    ) -> Result<String, ConverterError> {
        Ok(build_conversion_url(draft)?)
    }
    pub(crate) async fn parse_url(
        &self,
        input: &str,
    ) -> Result<SubscriptionConverterDraft, ConverterError> {
        let input = input.trim();
        let url = service_url(input)?;
        if url.query_pairs().any(|(name, _)| name == "target") {
            return Ok(parse_conversion_url(input)?);
        }
        let (request, route) = self.request(Method::GET, url.as_str())?;
        let response = request.send().await.map_err(network_error)?;
        let text = read_external_text_response(
            response,
            "conversion URL resolve",
            route,
            EXTERNAL_HTTP_MAX_BODY_BYTES,
        )
        .await
        .map_err(|error| ConverterError(error.to_string()))?;
        Ok(parse_conversion_url(text.final_url.as_str())?)
    }
    pub(crate) async fn short_url(
        &self,
        api: &str,
        long_url: &str,
    ) -> Result<String, ConverterError> {
        let url = service_url(api)?;
        service_url(long_url)?;
        let (request, route) = self.request(Method::POST, url.as_str())?;
        let response = request
            .multipart(
                reqwest::multipart::Form::new()
                    .text("longUrl", BASE64_STANDARD.encode(long_url.as_bytes())),
            )
            .send()
            .await
            .map_err(network_error)?;
        let text = read_external_text_response(
            response,
            "short URL generation",
            route,
            EXTERNAL_HTTP_MAX_BODY_BYTES,
        )
        .await
        .map_err(|error| ConverterError(error.to_string()))?;
        let value: Value = serde_json::from_str(&text.body)?;
        let valid = value.get("Code").and_then(Value::as_i64) == Some(1)
            || value.get("Code").and_then(Value::as_str) == Some("1");
        if valid {
            if let Some(url) = value
                .get("ShortUrl")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            {
                service_url(url)?;
                return Ok(url.to_owned());
            }
        }
        Err(ConverterError(
            value
                .get("Message")
                .and_then(Value::as_str)
                .unwrap_or("Short URL service returned no URL")
                .to_owned(),
        ))
    }
    pub(crate) async fn upload_config(
        &self,
        api: &str,
        content: &str,
    ) -> Result<String, ConverterError> {
        let url = service_url(api)?;
        if content.trim().is_empty() {
            return Err(ConverterError(
                "Remote config content must not be empty".to_owned(),
            ));
        }
        let (request, route) = self.request(Method::POST, url.as_str())?;
        let response = request
            .json(&serde_json::json!({"content": content}))
            .send()
            .await
            .map_err(network_error)?;
        let text = read_external_text_response(
            response,
            "remote config upload",
            route,
            EXTERNAL_HTTP_MAX_BODY_BYTES,
        )
        .await
        .map_err(|error| ConverterError(error.to_string()))?;
        let value: Value = serde_json::from_str(&text.body)?;
        if value.get("code").and_then(Value::as_i64) == Some(0) {
            if let Some(url) = value
                .get("data")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            {
                service_url(url)?;
                return Ok(url.to_owned());
            }
        }
        Err(ConverterError(
            value
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("Config upload service returned no URL")
                .to_owned(),
        ))
    }
    pub(crate) async fn backend_version(&self, backend: &str) -> Result<String, ConverterError> {
        let mut url = service_url(&normalized_backend(backend)?)?;
        let path = url.path().trim_end_matches('/');
        let parent = path.strip_suffix("/sub").unwrap_or(path);
        url.set_path(&format!("{parent}/version"));
        url.set_query(None);
        let (request, route) = self.request(Method::GET, url.as_str())?;
        let response = request.send().await.map_err(network_error)?;
        let text = read_external_text_response(
            response,
            "backend version fetch",
            route,
            EXTERNAL_HTTP_MAX_BODY_BYTES,
        )
        .await
        .map_err(|error| ConverterError(error.to_string()))?;
        Ok(text
            .body
            .replace("backend\n", "")
            .replace("subconverter", "")
            .replace(['\r', '\n'], " ")
            .trim()
            .to_owned())
    }
    fn request(
        &self,
        method: Method,
        url: &str,
    ) -> Result<(reqwest::RequestBuilder, ExternalHttpRoute), ConverterError> {
        let (proxy_url, route) = self
            .core
            .external_http_route()
            .map_err(|error| ConverterError(error.to_string()))?;
        let client =
            shared_external_http_client().map_err(|error| ConverterError(error.to_string()))?;
        let request = client
            .request(
                method,
                url,
                proxy_url.as_deref(),
                ExternalRequestKind::Generic,
            )
            .map_err(|error| ConverterError(error.to_string()))?;
        Ok((request, route))
    }
}
fn service_url(input: &str) -> Result<Url, ConverterError> {
    let url =
        Url::parse(input.trim()).map_err(|_| ConverterError("Invalid service URL".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ConverterError(
            "Service URL must use HTTP(S) without embedded credentials".to_owned(),
        ));
    }
    Ok(url)
}
fn network_error(error: reqwest::Error) -> ConverterError {
    ConverterError(error.without_url().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SubscriptionConverterDraft {
    pub advanced: bool,
    pub source_sub_url: String,
    pub client_type: String,
    pub backend: String,
    pub remote_config: String,
    pub exclude_remarks: String,
    pub include_remarks: String,
    pub filename: String,
    pub emoji: bool,
    pub node_list: bool,
    pub sort: bool,
    pub udp: bool,
    pub need_udp: bool,
    pub tfo: bool,
    pub scv: bool,
    pub fdn: bool,
    pub expand: bool,
    pub append_type: bool,
    pub insert: bool,
    pub new_name: bool,
    pub surge_doh: bool,
    pub clash_doh: bool,
    pub custom_params: Vec<CustomParam>,
    pub short_url_api: String,
    pub config_upload_api: String,
}

impl Default for SubscriptionConverterDraft {
    fn default() -> Self {
        Self {
            advanced: false,
            source_sub_url: String::new(),
            client_type: "clash".to_owned(),
            backend: DEFAULT_BACKEND.to_owned(),
            remote_config: String::new(),
            exclude_remarks: String::new(),
            include_remarks: String::new(),
            filename: String::new(),
            emoji: true,
            node_list: false,
            sort: false,
            udp: false,
            need_udp: false,
            tfo: false,
            scv: true,
            fdn: false,
            expand: true,
            append_type: false,
            insert: false,
            new_name: true,
            surge_doh: false,
            clash_doh: false,
            custom_params: Vec::new(),
            short_url_api: DEFAULT_SHORT_URL_API.to_owned(),
            config_upload_api: DEFAULT_CONFIG_UPLOAD_API.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct CustomParam {
    pub name: String,
    pub value: String,
}

fn build_conversion_url(draft: &SubscriptionConverterDraft) -> Result<String, String> {
    let source = draft
        .source_sub_url
        .replace("\r\n", "\n")
        .replace(['\r', '\n'], "|");
    if source.trim_matches('|').trim().is_empty() {
        return Err("Subscription link must not be empty".to_owned());
    }
    if draft.client_type.trim().is_empty() {
        return Err("Select a target client".to_owned());
    }

    let mut output = normalized_backend(&draft.backend)?;
    append_raw_param(&mut output, "target", target_name(&draft.client_type));
    if let Some(version) = target_version(&draft.client_type) {
        append_raw_param(&mut output, "ver", version);
    }
    append_encoded_param(&mut output, "url", &source);
    append_raw_param(&mut output, "insert", bool_text(draft.insert));

    if !draft.advanced {
        return Ok(output);
    }

    append_optional_encoded(&mut output, "config", &draft.remote_config);
    append_optional_encoded(&mut output, "exclude", &draft.exclude_remarks);
    append_optional_encoded(&mut output, "include", &draft.include_remarks);
    append_optional_encoded(&mut output, "filename", &draft.filename);
    if draft.append_type {
        append_raw_param(&mut output, "append_type", "true");
    }
    append_raw_param(&mut output, "emoji", bool_text(draft.emoji));
    append_raw_param(&mut output, "list", bool_text(draft.node_list));
    append_raw_param(&mut output, "tfo", bool_text(draft.tfo));
    append_raw_param(&mut output, "scv", bool_text(draft.scv));
    append_raw_param(&mut output, "fdn", bool_text(draft.fdn));
    append_raw_param(&mut output, "expand", bool_text(draft.expand));
    append_raw_param(&mut output, "sort", bool_text(draft.sort));
    if draft.need_udp {
        append_raw_param(&mut output, "udp", bool_text(draft.udp));
    }
    if draft.surge_doh {
        append_raw_param(&mut output, "surge.doh", "true");
    }
    if draft.client_type == "clash" {
        if draft.clash_doh {
            append_raw_param(&mut output, "clash.doh", "true");
        }
        append_raw_param(&mut output, "new_name", bool_text(draft.new_name));
    }
    for param in &draft.custom_params {
        if !param.name.trim().is_empty() && !param.value.trim().is_empty() {
            append_encoded_pair(&mut output, param.name.trim(), param.value.trim());
        }
    }
    Ok(output)
}

fn parse_conversion_url(input: &str) -> Result<SubscriptionConverterDraft, String> {
    let url = Url::parse(input.trim()).map_err(|_| "Enter a valid conversion link".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Conversion link must use http or https".to_owned());
    }

    let pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    let param = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let target =
        param("target").ok_or_else(|| "Link is missing the target parameter".to_owned())?;
    let source = param("url").ok_or_else(|| "Link is missing the url parameter".to_owned())?;
    let mut draft = SubscriptionConverterDraft {
        source_sub_url: source.replace('|', "\n"),
        client_type: if target == "surge" {
            format!(
                "surge&ver={}",
                param("ver").unwrap_or_else(|| "4".to_owned())
            )
        } else {
            target
        },
        backend: backend_from_url(&url),
        insert: param_is_true(&param, "insert"),
        remote_config: param("config").unwrap_or_default(),
        exclude_remarks: param("exclude").unwrap_or_default(),
        include_remarks: param("include").unwrap_or_default(),
        filename: param("filename").unwrap_or_default(),
        append_type: param_is_true(&param, "append_type"),
        emoji: param_is_true(&param, "emoji"),
        node_list: param_is_true(&param, "list"),
        tfo: param_is_true(&param, "tfo"),
        scv: param_is_true(&param, "scv"),
        fdn: param_is_true(&param, "fdn"),
        sort: param_is_true(&param, "sort"),
        udp: param_is_true(&param, "udp"),
        need_udp: param("udp").is_some(),
        expand: param_is_true(&param, "expand"),
        surge_doh: param_is_true(&param, "surge.doh"),
        clash_doh: param_is_true(&param, "clash.doh"),
        new_name: param_is_true(&param, "new_name"),
        ..SubscriptionConverterDraft::default()
    };
    let known = KNOWN_PARAMS.iter().copied().collect::<HashSet<_>>();
    draft.custom_params = pairs
        .into_iter()
        .filter(|(name, _)| !known.contains(name.as_str()))
        .map(|(name, value)| CustomParam { name, value })
        .collect();
    draft.advanced = url
        .query_pairs()
        .any(|(name, _)| !matches!(name.as_ref(), "target" | "ver" | "url" | "insert"));
    Ok(draft)
}

fn normalized_backend(backend: &str) -> Result<String, String> {
    let backend = backend.trim();
    let url = Url::parse(backend).map_err(|_| "Enter a valid backend address".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Backend address must use http or https".to_owned());
    }
    if url.host_str().is_none() {
        return Err("Backend address is missing a hostname".to_owned());
    }
    if url.fragment().is_some() {
        return Err("Backend address must not contain a fragment".to_owned());
    }
    let mut value = backend.to_owned();
    if !value.ends_with(['?', '&']) {
        value.push(if url.query().is_some() { '&' } else { '?' });
    }
    Ok(value)
}

fn backend_from_url(url: &Url) -> String {
    let mut backend = url.clone();
    backend.set_query(None);
    backend.set_fragment(None);
    let mut value = backend.to_string();
    if !value.ends_with('?') {
        value.push('?');
    }
    value
}

fn target_name(client_type: &str) -> &str {
    client_type
        .split_once("&ver=")
        .map_or(client_type, |pair| pair.0)
}

fn target_version(client_type: &str) -> Option<&str> {
    client_type
        .split_once("&ver=")
        .map(|(_, version)| version)
        .filter(|version| !version.is_empty())
}

fn append_optional_encoded(output: &mut String, name: &str, value: &str) {
    if !value.trim().is_empty() {
        append_encoded_param(output, name, value.trim());
    }
}

fn append_raw_param(output: &mut String, name: &str, value: &str) {
    if !output.ends_with(['?', '&']) {
        output.push('&');
    }
    output.push_str(name);
    output.push('=');
    output.push_str(value);
}

fn append_encoded_param(output: &mut String, name: &str, value: &str) {
    if !output.ends_with(['?', '&']) {
        output.push('&');
    }
    output.push_str(name);
    output.push('=');
    output.push_str(&encode_uri_component(value));
}

fn append_encoded_pair(output: &mut String, name: &str, value: &str) {
    if !output.ends_with(['?', '&']) {
        output.push('&');
    }
    output.push_str(&encode_uri_component(name));
    output.push('=');
    output.push_str(&encode_uri_component(value));
}

fn encode_uri_component(value: &str) -> String {
    // `sub-web` uses JavaScript's encodeURIComponent, where spaces are `%20`.
    // form_urlencoded otherwise emits `+`, so normalize that one difference.
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn param_is_true<F>(param: &F, name: &str) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    param(name).as_deref() == Some("true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_preserves_encoded_sources_target_version_and_custom_parameters() {
        let draft = SubscriptionConverterDraft {
            source_sub_url: "https://a.example/sub?token=secret&x=1\nhttps://b.example/订阅".into(),
            client_type: "surge&ver=4".into(),
            backend: "https://converter.example/sub?".into(),
            advanced: true,
            custom_params: vec![CustomParam {
                name: "rename & add".into(),
                value: "hello world=ok".into(),
            }],
            ..Default::default()
        };
        let url = build_conversion_url(&draft).unwrap();
        assert!(url.contains("target=surge&ver=4"));
        assert!(!url.contains("hello world"));
        let decoded = parse_conversion_url(&url).unwrap();
        assert_eq!(decoded.source_sub_url, draft.source_sub_url);
        assert_eq!(decoded.client_type, draft.client_type);
        assert_eq!(decoded.custom_params, draft.custom_params);
    }

    #[test]
    fn simple_conversion_does_not_send_hidden_advanced_options() {
        let draft = SubscriptionConverterDraft {
            source_sub_url: "https://source.example/sub".into(),
            remote_config: "https://config.example/private.ini".into(),
            custom_params: vec![CustomParam {
                name: "secret".into(),
                value: "hidden".into(),
            }],
            ..Default::default()
        };
        let url = build_conversion_url(&draft).unwrap();
        assert!(!url.contains("config="));
        assert!(!url.contains("secret="));
        assert_eq!(
            parse_conversion_url(&url).unwrap().source_sub_url,
            draft.source_sub_url
        );
    }

    #[test]
    fn malformed_and_non_http_links_are_rejected_before_network_io() {
        for input in [
            "",
            "file:///tmp/private",
            "ftp://example.com/sub",
            "https://user:password@example.com/sub",
        ] {
            assert!(service_url(input).is_err(), "accepted {input}");
        }
        assert!(parse_conversion_url("https://example.com/sub?target=clash").is_err());
        assert!(parse_conversion_url("https://example.com/sub?url=x").is_err());
        assert!(build_conversion_url(&SubscriptionConverterDraft::default()).is_err());
    }

    #[test]
    fn corrupted_persisted_draft_is_reported_without_replacement() {
        let directory = std::env::temp_dir().join(format!(
            "paws-converter-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(DRAFT_FILE);
        fs::write(&path, "corrupt draft").unwrap();
        let converter = Converter::new(paws_core::shared_core(), directory.clone());
        assert!(converter.load_draft().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "corrupt draft");
        fs::remove_dir_all(directory).unwrap();
    }
}
