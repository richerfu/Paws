use crate::i18n::{tr, translate_ui};
use crate::locale::UiLocale;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use url::Url;

pub(crate) const DEFAULT_BACKEND: &str = "https://api.wcc.best/sub?";
pub(crate) const DEFAULT_SHORT_URL_API: &str = "https://suosuo.de/short";
pub(crate) const DEFAULT_CONFIG_UPLOAD_API: &str = "https://oss.wcc.best/upload";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientType {
    pub label: &'static str,
    pub value: &'static str,
}

pub(crate) const CLIENT_TYPES: &[ClientType] = &[
    ClientType {
        label: "Clash",
        value: "clash",
    },
    ClientType {
        label: "Surge",
        value: "surge&ver=4",
    },
    ClientType {
        label: "Quantumult",
        value: "quan",
    },
    ClientType {
        label: "QuantumultX",
        value: "quanx",
    },
    ClientType {
        label: "Mellow",
        value: "mellow",
    },
    ClientType {
        label: "Surfboard",
        value: "surfboard",
    },
    ClientType {
        label: "Loon",
        value: "loon",
    },
    ClientType {
        label: "sing-box",
        value: "singbox",
    },
    ClientType {
        label: "SS",
        value: "ss",
    },
    ClientType {
        label: "SSD",
        value: "ssd",
    },
    ClientType {
        label: "SSSub",
        value: "sssub",
    },
    ClientType {
        label: "SSR",
        value: "ssr",
    },
    ClientType {
        label: "ClashR",
        value: "clashr",
    },
    ClientType {
        label: "V2Ray",
        value: "v2ray",
    },
    ClientType {
        label: "Trojan",
        value: "trojan",
    },
    ClientType {
        label: "Surge 3",
        value: "surge&ver=3",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RemoteConfig {
    pub label: &'static str,
    pub value: &'static str,
}

pub(crate) const REMOTE_CONFIGS: &[RemoteConfig] = &[
    RemoteConfig {
        label: "",
        value: "",
    },
    RemoteConfig {
        label: "Universal · No-Urltest",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/universal/no-urltest.ini",
    },
    RemoteConfig {
        label: "Universal · Urltest",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/universal/urltest.ini",
    },
    RemoteConfig {
        label: "Customized · Maying",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/maying.ini",
    },
    RemoteConfig {
        label: "Customized · Ytoo",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/ytoo.ini",
    },
    RemoteConfig {
        label: "Customized · FlowerCloud",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/flowercloud.ini",
    },
    RemoteConfig {
        label: "Customized · Nexitally",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/nexitally.ini",
    },
    RemoteConfig {
        label: "Customized · SoCloud",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/socloud.ini",
    },
    RemoteConfig {
        label: "Customized · ARK",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/ark.ini",
    },
    RemoteConfig {
        label: "Customized · ssrCloud",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/customized/ssrcloud.ini",
    },
    RemoteConfig {
        label: "Special · NeteaseUnblock",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/special/netease.ini",
    },
    RemoteConfig {
        label: "Special · Basic",
        value: "https://cdn.jsdelivr.net/gh/SleepyHeeead/subconverter-config@master/remote-config/special/basic.ini",
    },
];

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
            advanced: true,
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

pub(crate) fn client_label(_locale: UiLocale, value: &str) -> String {
    CLIENT_TYPES
        .iter()
        .find(|item| item.value == value)
        .map(|item| item.label.to_owned())
        .unwrap_or_else(|| "Clash".to_owned())
}

pub(crate) fn client_value(label: &str) -> &'static str {
    CLIENT_TYPES
        .iter()
        .find(|item| item.label == label)
        .map(|item| item.value)
        .unwrap_or("clash")
}

pub(crate) fn remote_config_label(locale: UiLocale, value: &str) -> String {
    if value.is_empty() {
        return translate_ui(locale, tr::conv_001());
    }
    REMOTE_CONFIGS
        .iter()
        .find(|item| item.value == value)
        .map(|item| item.label.to_owned())
        .unwrap_or_else(|| translate_ui(locale, tr::conv_002()))
}

pub(crate) fn parse_custom_params(text: &str) -> Vec<CustomParam> {
    text.lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(CustomParam {
                name: name.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

pub(crate) fn format_custom_params(params: &[CustomParam]) -> String {
    params
        .iter()
        .filter(|item| !item.name.is_empty() && !item.value.is_empty())
        .map(|item| format!("{}={}", item.name, item.value))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn build_conversion_url(
    locale: UiLocale,
    draft: &SubscriptionConverterDraft,
) -> Result<String, String> {
    let source = draft
        .source_sub_url
        .replace("\r\n", "\n")
        .replace(['\r', '\n'], "|");
    if source.trim_matches('|').trim().is_empty() {
        return Err(translate_ui(locale, tr::conv_003()));
    }
    if draft.client_type.trim().is_empty() {
        return Err(translate_ui(locale, tr::conv_004()));
    }

    let mut output = normalized_backend(locale, &draft.backend)?;
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

pub(crate) fn parse_conversion_url(
    locale: UiLocale,
    input: &str,
) -> Result<SubscriptionConverterDraft, String> {
    let url =
        Url::parse(input.trim()).map_err(|_| translate_ui(locale, tr::conv_005()).to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(translate_ui(locale, tr::conv_006()));
    }

    let pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    let param = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let target = param("target").ok_or_else(|| translate_ui(locale, tr::conv_007()).to_owned())?;
    let source = param("url").ok_or_else(|| translate_ui(locale, tr::conv_008()).to_owned())?;
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

pub(crate) async fn resolve_and_parse_conversion_url(
    locale: UiLocale,
    input: &str,
) -> Result<SubscriptionConverterDraft, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(translate_ui(locale, tr::conv_009()));
    }
    if input.contains("target") {
        return parse_conversion_url(locale, input);
    }
    let client = reqwest::Client::builder()
        .redirect(Policy::limited(10))
        .build()
        .map_err(|err| translate_ui(locale, tr::conv_010(err.to_string())))?;
    let response = client
        .get(input)
        .send()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_011(err.to_string())))?;
    parse_conversion_url(locale, response.url().as_str())
}

pub(crate) async fn generate_short_url(
    locale: UiLocale,
    api: &str,
    long_url: &str,
) -> Result<String, String> {
    let api = validate_service_url(locale, api, &translate_ui(locale, tr::conv_012()))?;
    let form = reqwest::multipart::Form::new()
        .text("longUrl", BASE64_STANDARD.encode(long_url.as_bytes()));
    let response = reqwest::Client::new()
        .post(api)
        .multipart(form)
        .send()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_013(err.to_string())))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_014(err.to_string())))?;
    if !status.is_success() {
        return Err(translate_ui(locale, tr::conv_015(status.to_string())));
    }
    let code_ok = value.get("Code").and_then(Value::as_i64) == Some(1)
        || value.get("Code").and_then(Value::as_str) == Some("1");
    let short_url = value
        .get("ShortUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code_ok && !short_url.is_empty() {
        Ok(short_url.to_owned())
    } else {
        Err(value
            .get("Message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| translate_ui(locale, tr::conv_016())))
    }
}

pub(crate) async fn upload_remote_config(
    locale: UiLocale,
    api: &str,
    content: &str,
) -> Result<String, String> {
    let api = validate_service_url(locale, api, &translate_ui(locale, tr::conv_017()))?;
    if content.trim().is_empty() {
        return Err(translate_ui(locale, tr::conv_018()));
    }
    let response = reqwest::Client::new()
        .post(api)
        .json(&serde_json::json!({ "content": content }))
        .send()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_019(err.to_string())))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_020(err.to_string())))?;
    if !status.is_success() {
        return Err(translate_ui(locale, tr::conv_021(status.to_string())));
    }
    let uploaded = value
        .get("data")
        .and_then(|data| data.get("url"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if value.get("code").and_then(Value::as_i64) == Some(0) && !uploaded.is_empty() {
        Ok(uploaded.to_owned())
    } else {
        Err(value
            .get("msg")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| translate_ui(locale, tr::conv_022())))
    }
}

pub(crate) async fn fetch_backend_version(
    locale: UiLocale,
    backend: &str,
) -> Result<String, String> {
    let mut url = Url::parse(&normalized_backend(locale, backend)?)
        .map_err(|_| translate_ui(locale, tr::conv_023()).to_owned())?;
    let path = url.path().trim_end_matches('/');
    let parent = path.strip_suffix("/sub").unwrap_or(path);
    url.set_path(&format!("{parent}/version"));
    url.set_query(None);
    let response = reqwest::get(url)
        .await
        .map_err(|err| translate_ui(locale, tr::conv_024(err.to_string())))?
        .error_for_status()
        .map_err(|err| translate_ui(locale, tr::conv_024(err.to_string())))?;
    let text = response
        .text()
        .await
        .map_err(|err| translate_ui(locale, tr::conv_025(err.to_string())))?;
    Ok(text
        .replace("backend\n", "")
        .replace("subconverter", "")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_owned())
}

pub(crate) fn load_draft() -> SubscriptionConverterDraft {
    let Some(path) = draft_path() else {
        return SubscriptionConverterDraft::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn save_draft(
    locale: UiLocale,
    draft: &SubscriptionConverterDraft,
) -> Result<(), String> {
    let path = draft_path().ok_or_else(|| translate_ui(locale, tr::conv_026()).to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| translate_ui(locale, tr::conv_027()).to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|err| translate_ui(locale, tr::conv_028(err.to_string())))?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(draft)
        .map_err(|err| translate_ui(locale, tr::conv_029(err.to_string())))?;
    fs::write(&temp, bytes).map_err(|err| translate_ui(locale, tr::conv_030(err.to_string())))?;
    fs::rename(&temp, &path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        translate_ui(locale, tr::conv_030(err.to_string()))
    })
}

pub(crate) fn clash_install_url(
    locale: UiLocale,
    long_url: &str,
    short_url: &str,
) -> Result<String, String> {
    let source = if short_url.trim().is_empty() {
        long_url.trim()
    } else {
        short_url.trim()
    };
    if source.is_empty() {
        return Err(translate_ui(locale, tr::conv_031()));
    }
    let encoded = encode_uri_component(source);
    Ok(format!("clash://install-config?url={encoded}"))
}

fn draft_path() -> Option<PathBuf> {
    std::env::var("HMETA_HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(DRAFT_FILE))
}

fn normalized_backend(locale: UiLocale, backend: &str) -> Result<String, String> {
    let backend = backend.trim();
    let url = Url::parse(backend).map_err(|_| translate_ui(locale, tr::conv_032()).to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(translate_ui(locale, tr::conv_033()));
    }
    if url.host_str().is_none() {
        return Err(translate_ui(locale, tr::conv_034()));
    }
    if url.fragment().is_some() {
        return Err(translate_ui(locale, tr::conv_035()));
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

fn validate_service_url(locale: UiLocale, input: &str, label: &str) -> Result<Url, String> {
    let url = Url::parse(input.trim()).map_err(|_| translate_ui(locale, tr::conv_036(label)))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(translate_ui(locale, tr::conv_037(label)));
    }
    Ok(url)
}
