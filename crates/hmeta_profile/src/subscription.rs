use super::*;

pub fn normalize_profile_content(raw_profile: &str) -> Result<String, HMetaError> {
    if let Ok(yaml) = normalize_yaml(raw_profile) {
        return Ok(yaml);
    }

    for candidate in subscription_candidates(raw_profile) {
        if let Some(yaml) = subscription_links_to_yaml(&candidate)? {
            return Ok(yaml);
        }
    }

    normalize_yaml(raw_profile)
}

pub fn parse_subscription_userinfo(header: &str) -> Option<SubscriptionUserInfo> {
    let mut upload_bytes = None;
    let mut download_bytes = None;
    let mut total_bytes = None;
    let mut expire_at = None;

    for part in header.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "upload" => upload_bytes = value.parse::<u64>().ok(),
            "download" => download_bytes = value.parse::<u64>().ok(),
            "total" => total_bytes = value.parse::<u64>().ok(),
            "expire" => {
                expire_at = value
                    .parse::<u64>()
                    .ok()
                    .filter(|expire| *expire > 0)
                    .map(|expire| expire.to_string())
            }
            _ => {}
        }
    }

    if upload_bytes.is_none()
        && download_bytes.is_none()
        && total_bytes.is_none()
        && expire_at.is_none()
    {
        return None;
    }
    Some(SubscriptionUserInfo {
        upload_bytes: upload_bytes.unwrap_or(0),
        download_bytes: download_bytes.unwrap_or(0),
        total_bytes,
        expire_at,
    })
}

pub fn parse_subscription_userinfo_comment(raw_profile: &str) -> Option<SubscriptionUserInfo> {
    leading_subscription_comments(raw_profile).find_map(|comment| {
        let payload = comment
            .split_once(':')
            .and_then(|(key, value)| {
                key.trim()
                    .eq_ignore_ascii_case("subscription-userinfo")
                    .then_some(value.trim())
            })
            .unwrap_or(comment);
        parse_subscription_userinfo(payload)
    })
}

pub fn parse_subscription_metadata(
    title: Option<&str>,
    update_interval: Option<&str>,
    web_page_url: Option<&str>,
    support_url: Option<&str>,
) -> Option<SubscriptionMetadata> {
    let title = title.and_then(decode_subscription_header_text);
    let update_interval_hours = update_interval.and_then(|value| value.trim().parse::<u64>().ok());
    let web_page_url = web_page_url.and_then(clean_subscription_url);
    let support_url = support_url.and_then(clean_subscription_url);
    if title.is_none()
        && update_interval_hours.is_none()
        && web_page_url.is_none()
        && support_url.is_none()
    {
        return None;
    }
    Some(SubscriptionMetadata {
        title,
        update_interval_hours,
        web_page_url,
        support_url,
    })
}

pub fn parse_subscription_metadata_comment(raw_profile: &str) -> Option<SubscriptionMetadata> {
    let mut title = None;
    let mut update_interval = None;
    let mut web_page_url = None;
    let mut support_url = None;
    for comment in leading_subscription_comments(raw_profile) {
        let payload = comment
            .split_once(':')
            .and_then(|(key, value)| {
                key.trim()
                    .eq_ignore_ascii_case("subscription-metadata")
                    .then_some(value.trim())
            })
            .unwrap_or(comment);
        for part in payload.split(';') {
            let Some((key, value)) = split_subscription_metadata_part(part) else {
                continue;
            };
            match key.as_str() {
                "profile-title" if title.is_none() => title = Some(value),
                "profile-update-interval" | "update-interval" if update_interval.is_none() => {
                    update_interval = Some(value)
                }
                "profile-web-page-url" | "web-page-url" if web_page_url.is_none() => {
                    web_page_url = Some(value)
                }
                "support-url" if support_url.is_none() => support_url = Some(value),
                _ => {}
            }
        }
    }
    parse_subscription_metadata(title, update_interval, web_page_url, support_url)
}

pub(super) fn leading_subscription_comments(raw_profile: &str) -> impl Iterator<Item = &str> {
    raw_profile
        .lines()
        .map(str::trim)
        .scan(false, |seen_content, line| {
            if *seen_content {
                return None;
            }
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() {
                return Some(None);
            }
            if let Some(comment) = line.strip_prefix('#') {
                return Some(Some(comment.trim()));
            }
            *seen_content = true;
            None
        })
        .flatten()
}

pub(super) fn split_subscription_metadata_part(part: &str) -> Option<(String, &str)> {
    let (key, value) = part.split_once('=').or_else(|| part.split_once(':'))?;
    let key = key.trim().replace('_', "-").to_ascii_lowercase();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

pub fn merge_subscription_metadata(
    primary: Option<SubscriptionMetadata>,
    fallback: Option<SubscriptionMetadata>,
) -> Option<SubscriptionMetadata> {
    match (primary, fallback) {
        (Some(mut primary), Some(fallback)) => {
            if primary.title.is_none() {
                primary.title = fallback.title;
            }
            if primary.update_interval_hours.is_none() {
                primary.update_interval_hours = fallback.update_interval_hours;
            }
            if primary.web_page_url.is_none() {
                primary.web_page_url = fallback.web_page_url;
            }
            if primary.support_url.is_none() {
                primary.support_url = fallback.support_url;
            }
            Some(primary)
        }
        (Some(primary), None) => Some(primary),
        (None, fallback) => fallback,
    }
}

pub fn parse_content_disposition_filename(header: &str) -> Option<String> {
    let mut fallback = None;
    for part in header.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "filename*" {
            let value = value.strip_prefix("UTF-8''").unwrap_or(value);
            return decode_subscription_header_text(value);
        }
        if key == "filename" {
            fallback = decode_subscription_header_text(value);
        }
    }
    fallback
}

pub fn decode_subscription_header_text(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    let text = decoded.trim().trim_matches('"').trim().to_owned();
    (!text.is_empty()).then_some(text)
}

pub(super) fn clean_subscription_url(value: &str) -> Option<String> {
    let value = decode_subscription_header_text(value)?;
    Url::parse(&value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|_| value)
}

pub fn normalize_yaml(raw_yaml: &str) -> Result<String, HMetaError> {
    let value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    if !matches!(value, Value::Mapping(_)) {
        return Err(HMetaError::Core(
            "profile root must be a YAML map or supported proxy subscription".to_owned(),
        ));
    }
    serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
}

pub fn sanitize_profile_for_meow_validation(raw_yaml: &str) -> Result<String, HMetaError> {
    let mut value: Value =
        serde_yaml::from_str(raw_yaml).map_err(|err| HMetaError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping_mut() else {
        return Err(HMetaError::Core(
            "profile root must be a YAML map or supported proxy subscription".to_owned(),
        ));
    };
    sanitize_app_managed_config(root);
    sanitize_app_managed_dns_for_validation(root);
    remove_app_managed_geodata_fields(root);
    serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
}

pub fn sanitize_profile_for_meow_validation_at(
    raw_yaml: &str,
    store_root: &Path,
) -> Result<String, HMetaError> {
    let sanitized = sanitize_profile_for_meow_validation(raw_yaml)?;
    let mut value: Value =
        serde_yaml::from_str(&sanitized).map_err(|err| HMetaError::Core(err.to_string()))?;
    let Some(root) = value.as_mapping_mut() else {
        return Err(HMetaError::Core(
            "profile root must be a YAML map or supported proxy subscription".to_owned(),
        ));
    };
    patch_geodata_paths(root, store_root)?;
    prune_unavailable_default_subscription_rules(root, store_root);
    serde_yaml::to_string(&value).map_err(|err| HMetaError::Core(err.to_string()))
}

pub(super) fn subscription_candidates(raw_profile: &str) -> Vec<String> {
    let mut candidates = vec![raw_profile.trim().to_owned()];
    let compact = raw_profile
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    for candidate in [raw_profile.trim(), compact.as_str()] {
        if let Some(decoded) = decode_base64_text(candidate) {
            candidates.push(decoded);
        }
    }
    candidates.dedup();
    candidates
}

pub(super) fn decode_base64_text(value: &str) -> Option<String> {
    for text in decode_base64_candidates(value) {
        if text.contains("://") || serde_yaml::from_str::<Value>(&text).is_ok() {
            return Some(text);
        }
    }
    None
}

pub(super) fn decode_base64_component(value: &str) -> Option<String> {
    decode_base64_candidates(value).into_iter().next()
}

pub(super) fn decode_base64_candidates(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut padded = value.trim().to_owned();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::URL_SAFE,
    ]
    .into_iter()
    .filter_map(|engine| {
        let bytes = engine.decode(&padded).ok()?;
        String::from_utf8(bytes).ok()
    })
    .collect()
}

pub(super) fn link_scheme(link: &str) -> &str {
    link.split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or_default()
}

pub(super) fn strip_link_scheme<'a>(link: &'a str, scheme: &str) -> Option<&'a str> {
    let prefix = link.get(..scheme.len())?;
    prefix
        .eq_ignore_ascii_case(scheme)
        .then(|| link.get(scheme.len()..))
        .flatten()
}

pub(super) fn subscription_links_to_yaml(content: &str) -> Result<Option<String>, HMetaError> {
    let mut proxies = Vec::new();
    let mut first_parse_error = None;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_subscription_comment_line(line))
    {
        match parse_subscription_link(line) {
            Ok(Some(proxy)) => proxies.push(proxy),
            Ok(None) => {}
            Err(err) => {
                first_parse_error.get_or_insert(err);
            }
        }
    }
    if proxies.is_empty() {
        if let Some(err) = first_parse_error {
            return Err(err);
        }
        return Ok(None);
    }
    Ok(Some(proxy_subscription_yaml(proxies)?))
}

pub(super) fn is_subscription_comment_line(line: &str) -> bool {
    line.starts_with('#') || line.starts_with("//") || line.starts_with(';')
}

pub(super) fn parse_subscription_link(link: &str) -> Result<Option<Mapping>, HMetaError> {
    match link_scheme(link).to_ascii_lowercase().as_str() {
        "vless" => parse_vless_link(link).map(Some),
        "trojan" => parse_trojan_link(link).map(Some),
        "ss" => parse_ss_link(link).map(Some),
        "ssr" => parse_ssr_link(link).map(Some),
        "vmess" => parse_vmess_link(link).map(Some),
        "hysteria2" | "hy2" => parse_hysteria2_link(link).map(Some),
        "hysteria" => parse_hysteria_link(link).map(Some),
        "tuic" => parse_tuic_link(link).map(Some),
        "http" | "https" => parse_http_link(link).map(Some),
        "socks" | "socks5" => parse_socks5_link(link).map(Some),
        _ => Ok(None),
    }
}

pub(super) fn parse_vless_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "VLESS"),
        "vless",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "uuid", &decode_component(url.username()));
    apply_query_udp_option(&mut proxy, &query, true);
    if vless_tls_enabled(&query) {
        put_bool(&mut proxy, "tls", true);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "serverName", "servername"]) {
        put_string(&mut proxy, "servername", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(flow) = query.get("flow") {
        put_string(&mut proxy, "flow", flow);
    }
    if let Some(encryption) = query.get("encryption") {
        let encryption = encryption.trim();
        if !encryption.is_empty() {
            put_string(&mut proxy, "encryption", &encryption.to_ascii_lowercase());
        }
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    apply_query_transport_options(&mut proxy, &query);
    apply_query_tls_options(&mut proxy, &query);
    Ok(proxy)
}

pub(super) fn parse_trojan_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Trojan"),
        "trojan",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "password", &decode_component(url.username()));
    apply_query_udp_option(&mut proxy, &query, true);
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    apply_query_transport_options(&mut proxy, &query);
    apply_query_tls_options(&mut proxy, &query);
    Ok(proxy)
}

pub(super) fn parse_ss_link(link: &str) -> Result<Mapping, HMetaError> {
    let without_scheme = strip_link_scheme(link, "ss://")
        .ok_or_else(|| HMetaError::Core("invalid ss subscription link".to_owned()))?;
    let (body, fragment) = without_scheme
        .split_once('#')
        .map(|(body, fragment)| (body, Some(fragment)))
        .unwrap_or((without_scheme, None));
    let body = body.split_once('?').map(|(body, _)| body).unwrap_or(body);
    let query = query_without_fragment(without_scheme);
    let sip002 = if body.contains('@') {
        body.to_owned()
    } else {
        decode_base64_component(body)
            .ok_or_else(|| HMetaError::Core("invalid ss subscription link".to_owned()))?
    };
    let parsed = Url::parse(&format!("ss://{sip002}")).map_err(subscription_error)?;
    let query_values = raw_query_map(query);
    let user = decode_component(parsed.username());
    let (cipher, password) = if let Some(password) = parsed.password() {
        (user, decode_component(password))
    } else {
        let decoded = decode_base64_text(&user)
            .ok_or_else(|| HMetaError::Core("invalid ss user info".to_owned()))?;
        decoded
            .split_once(':')
            .map(|(cipher, password)| (cipher.to_owned(), password.to_owned()))
            .ok_or_else(|| HMetaError::Core("invalid ss cipher/password".to_owned()))?
    };
    let mut proxy = proxy_base(
        fragment
            .map(decode_component)
            .filter(|name| !name.is_empty())
            .or_else(|| proxy_name_from_query(&query_values))
            .unwrap_or_else(|| default_proxy_name("SS", &parsed)),
        "ss",
        url_host(&parsed)?,
        url_port(&parsed)?,
    );
    put_string(&mut proxy, "cipher", &cipher);
    put_string(&mut proxy, "password", &password);
    apply_query_udp_option(&mut proxy, &query_values, true);
    apply_ss_plugin_options(&mut proxy, query);
    apply_raw_query_common_proxy_options(&mut proxy, query);
    Ok(proxy)
}

pub(super) fn parse_ssr_link(link: &str) -> Result<Mapping, HMetaError> {
    let encoded = strip_link_scheme(link, "ssr://")
        .ok_or_else(|| HMetaError::Core("invalid ssr subscription link".to_owned()))?
        .trim();
    let decoded = decode_base64_component(encoded)
        .ok_or_else(|| HMetaError::Core("invalid ssr subscription link".to_owned()))?;
    let (endpoint, query) = decoded
        .split_once("/?")
        .or_else(|| decoded.split_once('?'))
        .map(|(endpoint, query)| (endpoint, query))
        .unwrap_or((decoded.as_str(), ""));
    let mut parts = endpoint.splitn(6, ':');
    let server = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing server".to_owned()))?;
    let port = parts
        .next()
        .and_then(|port| port.parse::<u16>().ok())
        .ok_or_else(|| HMetaError::Core("ssr missing port".to_owned()))?;
    let protocol = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing protocol".to_owned()))?;
    let cipher = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing cipher".to_owned()))?;
    let obfs = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing obfs".to_owned()))?;
    let password = parts
        .next()
        .and_then(decode_base64_component)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HMetaError::Core("ssr missing password".to_owned()))?;
    let query = raw_query_map(query);
    let name = ssr_query_base64_any(&query, &["remarks", "remark", "name", "ps"])
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("SSR-{server}"));
    let mut proxy = proxy_base(name, "ssr", server.to_owned(), port);
    put_string(&mut proxy, "cipher", cipher);
    put_string(&mut proxy, "password", &password);
    put_string(&mut proxy, "protocol", protocol);
    put_string(&mut proxy, "obfs", obfs);
    if let Some(protocol_param) = ssr_query_base64_any(
        &query,
        &[
            "protoparam",
            "proto-param",
            "protoParam",
            "protocol-param",
            "protocolParam",
            "protocol_param",
        ],
    )
    .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "protocol-param", &protocol_param);
    }
    if let Some(obfs_param) = ssr_query_base64_any(
        &query,
        &["obfsparam", "obfs-param", "obfsParam", "obfs_param"],
    )
    .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "obfs-param", &obfs_param);
    }
    if let Some(group) = ssr_query_base64_any(&query, &["group", "groupName", "group-name"])
        .filter(|value| !value.is_empty())
    {
        put_string(&mut proxy, "group", &group);
    }
    apply_query_udp_option(&mut proxy, &query, true);
    Ok(proxy)
}

pub(super) fn ssr_query_base64_any(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    query_get_any(query, keys).and_then(decode_base64_component)
}

pub(super) fn parse_vmess_link(link: &str) -> Result<Mapping, HMetaError> {
    let encoded = strip_link_scheme(link, "vmess://")
        .ok_or_else(|| HMetaError::Core("invalid vmess subscription link".to_owned()))?
        .trim();
    let decoded = decode_base64_text(encoded)
        .ok_or_else(|| HMetaError::Core("invalid vmess subscription link".to_owned()))?;
    let value: serde_json::Value = serde_json::from_str(&decoded)
        .map_err(|err| HMetaError::Core(format!("invalid vmess JSON: {err}")))?;
    let server = json_str_any(&value, &["add", "server", "address", "addr"])
        .ok_or_else(|| HMetaError::Core("vmess missing server".to_owned()))?
        .to_owned();
    let port = json_u16_any(&value, &["port"])
        .ok_or_else(|| HMetaError::Core("vmess missing port".to_owned()))?;
    let mut proxy = proxy_base(
        json_str_any(
            &value,
            &["ps", "name", "remarks", "remark", "alias", "node-name"],
        )
        .unwrap_or("VMess")
        .to_owned(),
        "vmess",
        server,
        port,
    );
    if let Some(uuid) = json_str_any(&value, &["id", "uuid"]) {
        put_string(&mut proxy, "uuid", uuid);
    }
    if let Some(alter_id) = json_i64_any(&value, &["aid", "alterId", "alter_id"]) {
        put_i64(&mut proxy, "alterId", alter_id);
    }
    let security = json_str_any(&value, &["security"]);
    let security_is_tls = security.is_some_and(|security| security.eq_ignore_ascii_case("tls"));
    if let Some(cipher) = json_str_any(&value, &["scy", "cipher"]).or_else(|| {
        security
            .filter(|security| !security.eq_ignore_ascii_case("tls"))
            .filter(|security| !security.eq_ignore_ascii_case("none"))
    }) {
        put_string(&mut proxy, "cipher", cipher);
    }
    if json_str_any(&value, &["tls"]).is_some_and(tls_enabled_value) || security_is_tls {
        put_bool(&mut proxy, "tls", true);
    }
    if json_truthy_any(
        &value,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    let network = json_str_any(&value, &["net", "type", "network"]).map(str::to_ascii_lowercase);
    if let Some(network) = &network {
        put_string(&mut proxy, "network", network);
    }
    if let Some(servername) = json_str_any(&value, &["sni", "servername", "serverName"]) {
        put_string(&mut proxy, "servername", servername);
    }
    if let Some(fingerprint) = json_str_any(
        &value,
        &[
            "fp",
            "fingerprint",
            "client-fingerprint",
            "clientFingerprint",
        ],
    ) {
        put_string(&mut proxy, "client-fingerprint", fingerprint);
    }
    if let Some(alpn) = json_str_any(&value, &["alpn"]) {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if json_truthy_any(&value, &["udp"]) {
        put_bool(&mut proxy, "udp", true);
    }
    if json_truthy_any(&value, &["tfo", "fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "tfo", true);
    }
    match network.as_deref() {
        Some("ws") => put_ws_opts(
            &mut proxy,
            json_str_any(&value, &["path", "wsPath"]),
            json_str_any(&value, &["host", "wsHost"]),
            json_str_any(&value, &["ed", "maxEarlyData", "max-early-data"]),
            json_str_any(
                &value,
                &["eh", "earlyDataHeaderName", "early-data-header-name"],
            ),
        ),
        Some("grpc") => put_grpc_opts(
            &mut proxy,
            json_str_any(
                &value,
                &[
                    "path",
                    "serviceName",
                    "service",
                    "service-name",
                    "grpc-service-name",
                ],
            ),
            json_str_any(&value, &["mode", "grpc-mode"]),
        ),
        Some("h2") => put_h2_opts(
            &mut proxy,
            json_str_any(&value, &["path", "h2Path"]),
            json_str_any(&value, &["host", "h2Host"]),
        ),
        Some("httpupgrade") => put_http_upgrade_opts(
            &mut proxy,
            json_str_any(&value, &["path", "httpUpgradePath"]),
            json_str_any(&value, &["host", "httpUpgradeHost"]),
        ),
        _ => {}
    }
    Ok(proxy)
}

pub(super) fn parse_hysteria2_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Hysteria2"),
        "hysteria2",
        url_host(&url)?,
        url_port(&url)?,
    );
    let username = decode_component(url.username());
    if let Some(password) = query_get_any(
        &query,
        &["password", "auth", "auth-str", "auth_str", "authStr"],
    )
    .map(ToOwned::to_owned)
    .or_else(|| (!username.is_empty()).then_some(username))
    {
        put_string(&mut proxy, "password", &password);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(obfs) = query.get("obfs").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "obfs", obfs);
    }
    if let Some(obfs_password) =
        query_get_any(&query, &["obfs-password", "obfsPassword", "obfs_password"])
    {
        put_string(&mut proxy, "obfs-password", obfs_password);
    }
    if let Some(up) = query_get_any(&query, &["up", "upmbps", "upMbps"]) {
        put_string(&mut proxy, "up", up);
    }
    if let Some(down) = query_get_any(&query, &["down", "downmbps", "downMbps"]) {
        put_string(&mut proxy, "down", down);
    }
    if let Some(ports) = query_get_any(
        &query,
        &[
            "ports",
            "mport",
            "port-hopping",
            "port_hopping",
            "portHopping",
        ],
    ) {
        put_string(&mut proxy, "ports", ports);
    }
    if let Some(recv_window_conn) = query_get_any(
        &query,
        &["recv-window-conn", "recv_window_conn", "recvWindowConn"],
    ) {
        put_string(&mut proxy, "recv-window-conn", recv_window_conn);
    }
    if let Some(recv_window) = query_get_any(&query, &["recv-window", "recv_window", "recvWindow"])
    {
        put_string(&mut proxy, "recv-window", recv_window);
    }
    if truthy_query_any(
        &query,
        &[
            "disable-mtu-discovery",
            "disable_mtu_discovery",
            "disableMtuDiscovery",
        ],
    ) {
        put_bool(&mut proxy, "disable-mtu-discovery", true);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

pub(super) fn parse_hysteria_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "Hysteria"),
        "hysteria",
        url_host(&url)?,
        url_port(&url)?,
    );
    let username = decode_component(url.username());
    if let Some(auth) = query_get_any(
        &query,
        &["auth", "auth-str", "auth_str", "authStr", "authString"],
    )
    .map(str::to_owned)
    .or_else(|| (!username.is_empty()).then_some(username))
    {
        put_string(&mut proxy, "auth-str", &auth);
    }
    if let Some(protocol) = query.get("protocol").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "protocol", protocol);
    }
    if let Some(sni) = query_get_any(&query, &["sni", "peer", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(obfs) = query.get("obfs").filter(|value| !value.is_empty()) {
        put_string(&mut proxy, "obfs", obfs);
    }
    if let Some(up) = query_get_any(&query, &["up", "upmbps", "upMbps"]) {
        put_string(&mut proxy, "up", up);
    }
    if let Some(down) = query_get_any(&query, &["down", "downmbps", "downMbps"]) {
        put_string(&mut proxy, "down", down);
    }
    if let Some(ports) = query_get_any(
        &query,
        &[
            "ports",
            "mport",
            "port-hopping",
            "port_hopping",
            "portHopping",
        ],
    ) {
        put_string(&mut proxy, "ports", ports);
    }
    if let Some(recv_window_conn) = query_get_any(
        &query,
        &["recv-window-conn", "recv_window_conn", "recvWindowConn"],
    ) {
        put_string(&mut proxy, "recv-window-conn", recv_window_conn);
    }
    if let Some(recv_window) = query_get_any(&query, &["recv-window", "recv_window", "recvWindow"])
    {
        put_string(&mut proxy, "recv-window", recv_window);
    }
    if truthy_query_any(
        &query,
        &[
            "disable-mtu-discovery",
            "disable_mtu_discovery",
            "disableMtuDiscovery",
        ],
    ) {
        put_bool(&mut proxy, "disable-mtu-discovery", true);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

pub(super) fn parse_tuic_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let uuid = decode_component(url.username());
    let password = url.password().map(decode_component).unwrap_or_default();
    if uuid.is_empty() {
        return Err(HMetaError::Core("tuic missing uuid".to_owned()));
    }
    if password.is_empty() {
        return Err(HMetaError::Core("tuic missing password".to_owned()));
    }
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "TUIC"),
        "tuic",
        url_host(&url)?,
        url_port(&url)?,
    );
    put_string(&mut proxy, "uuid", &uuid);
    put_string(&mut proxy, "password", &password);
    if let Some(sni) = query_get_any(&query, &["sni", "serverName", "servername"]) {
        put_string(&mut proxy, "sni", sni);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(&mut proxy, "alpn", split_list(alpn));
    }
    if let Some(congestion_controller) = query_get_any(
        &query,
        &[
            "congestion-controller",
            "congestion_control",
            "congestionControl",
        ],
    ) {
        put_string(&mut proxy, "congestion-controller", congestion_controller);
    }
    if let Some(udp_relay_mode) = query_get_any(
        &query,
        &["udp-relay-mode", "udp_relay_mode", "udpRelayMode"],
    ) {
        put_string(&mut proxy, "udp-relay-mode", udp_relay_mode);
    }
    if truthy_query_any(&query, &["disable-sni", "disable_sni", "disableSni"]) {
        put_bool(&mut proxy, "disable-sni", true);
    }
    if truthy_query_any(&query, &["reduce-rtt", "reduce_rtt", "reduceRtt"]) {
        put_bool(&mut proxy, "reduce-rtt", true);
    }
    if let Some(request_timeout) = query_get_any(
        &query,
        &["request-timeout", "request_timeout", "requestTimeout"],
    ) {
        put_string(&mut proxy, "request-timeout", request_timeout);
    }
    if let Some(heartbeat_interval) = query_get_any(
        &query,
        &[
            "heartbeat-interval",
            "heartbeat_interval",
            "heartbeatInterval",
        ],
    ) {
        put_string(&mut proxy, "heartbeat-interval", heartbeat_interval);
    }
    if let Some(max_packet_size) = query_get_any(
        &query,
        &[
            "max-udp-relay-packet-size",
            "max_udp_relay_packet_size",
            "maxUdpRelayPacketSize",
        ],
    ) {
        put_string(&mut proxy, "max-udp-relay-packet-size", max_packet_size);
    }
    if truthy_query_any(&query, &["fast-open", "fast_open", "fastOpen"]) {
        put_bool(&mut proxy, "fast-open", true);
    }
    Ok(proxy)
}

pub(super) fn parse_http_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "HTTP"),
        "http",
        url_host(&url)?,
        url_port(&url)?,
    );
    if let Some((username, password)) = url_credentials(&url) {
        put_string(&mut proxy, "username", &username);
        put_string(&mut proxy, "password", &password);
    }
    if url.scheme() == "https" || truthy_query(&query, "tls") {
        put_bool(&mut proxy, "tls", true);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_http_header_query_options(&mut proxy, &query);
    Ok(proxy)
}

pub(super) fn parse_socks5_link(link: &str) -> Result<Mapping, HMetaError> {
    let url = Url::parse(link).map_err(subscription_error)?;
    let query = query_map(&url);
    let mut proxy = proxy_base(
        proxy_name_from_url(&url, &query, "SOCKS5"),
        "socks5",
        url_host(&url)?,
        url_port(&url)?,
    );
    if let Some((username, password)) = url_credentials(&url) {
        put_string(&mut proxy, "username", &username);
        put_string(&mut proxy, "password", &password);
    }
    if truthy_query(&query, "tls") {
        put_bool(&mut proxy, "tls", true);
    }
    if truthy_query_any(
        &query,
        &[
            "allowInsecure",
            "allow-insecure",
            "allow_insecure",
            "insecure",
            "skip-cert-verify",
        ],
    ) {
        put_bool(&mut proxy, "skip-cert-verify", true);
    }
    apply_query_common_proxy_options(&mut proxy, &query);
    Ok(proxy)
}

pub(super) fn apply_ss_plugin_options(proxy: &mut Mapping, query: &str) {
    let query = raw_query_map(query);
    let Some(plugin_value) = query.get("plugin").map(String::as_str) else {
        return;
    };
    let mut plugin_parts = plugin_value.split(';').map(str::trim);
    let Some(plugin) = plugin_parts.next().filter(|plugin| !plugin.is_empty()) else {
        return;
    };
    let inline_opts = plugin_parts
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(";");
    let explicit_opts =
        query_get_any(&query, &["plugin-opts", "pluginOpts", "pluginopts"]).unwrap_or("");
    let opts = [inline_opts.as_str(), explicit_opts]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(";");
    match plugin.to_ascii_lowercase().as_str() {
        "obfs" | "obfs-local" | "simple-obfs" => {
            put_string(proxy, "plugin", "obfs");
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
        "v2ray-plugin" => {
            put_string(proxy, "plugin", "v2ray-plugin");
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
        other => {
            put_string(proxy, "plugin", other);
            if !opts.is_empty() {
                put_string(proxy, "plugin-opts", &opts);
            }
        }
    }
}

pub(super) fn query_without_fragment(value: &str) -> &str {
    value
        .split_once('?')
        .map(|(_, query)| {
            query
                .split_once('#')
                .map(|(query, _)| query)
                .unwrap_or(query)
        })
        .unwrap_or("")
}

pub(super) fn raw_query_map(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .fold(HashMap::new(), |mut map, (key, value)| {
            insert_query_value(&mut map, key, value);
            map
        })
}

pub(super) fn apply_raw_query_common_proxy_options(proxy: &mut Mapping, query: &str) {
    let query = raw_query_map(query);
    apply_query_common_proxy_options(proxy, &query);
}

pub(super) fn apply_query_common_proxy_options(
    proxy: &mut Mapping,
    query: &HashMap<String, String>,
) {
    if truthy_query(query, "udp") {
        put_bool(proxy, "udp", true);
    }
    if truthy_query_any(query, &["tfo", "fast-open", "fast_open", "fastOpen"]) {
        put_bool(proxy, "tfo", true);
    }
}

pub(super) fn apply_http_header_query_options(
    proxy: &mut Mapping,
    query: &HashMap<String, String>,
) {
    let Some(raw_headers) = query_get_any(query, &["headers", "header"]) else {
        return;
    };
    let mut headers = Mapping::new();
    for part in raw_headers
        .split([';', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((key, value)) = part.split_once('=').or_else(|| part.split_once(':')) else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if !key.is_empty() && !value.is_empty() {
            put_string(&mut headers, key, value);
        }
    }
    if !headers.is_empty() {
        proxy.insert(value_key("headers"), Value::Mapping(headers));
    }
}

pub(super) fn apply_query_udp_option(
    proxy: &mut Mapping,
    query: &HashMap<String, String>,
    default: bool,
) {
    match query_bool(query, "udp") {
        Some(udp) => put_bool(proxy, "udp", udp),
        None if default => put_bool(proxy, "udp", true),
        None => {}
    }
}

pub(super) fn vless_tls_enabled(query: &HashMap<String, String>) -> bool {
    query
        .get("security")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "tls" | "reality"
            )
        })
        .unwrap_or(false)
        || truthy_query_any(query, &["tls", "enable-tls", "enable_tls", "enableTls"])
}

pub(super) fn apply_query_transport_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    let network = query
        .get("type")
        .or_else(|| query.get("network"))
        .map(|network| network.to_ascii_lowercase());
    if let Some(network) = &network {
        put_string(proxy, "network", network);
    }
    match network.as_deref() {
        Some("ws") => put_ws_opts(
            proxy,
            query_get_any(query, &["path", "wsPath"]),
            query_get_any(query, &["host", "wsHost"]),
            query_get_any(query, &["ed", "maxEarlyData", "max-early-data"]),
            query_get_any(
                query,
                &["eh", "earlyDataHeaderName", "early-data-header-name"],
            ),
        ),
        Some("grpc") => put_grpc_opts(
            proxy,
            query_get_any(
                query,
                &[
                    "serviceName",
                    "service",
                    "service-name",
                    "grpc-service-name",
                    "path",
                ],
            ),
            query_get_any(query, &["mode", "grpc-mode"]),
        ),
        Some("h2") => put_h2_opts(
            proxy,
            query_get_any(query, &["path", "h2Path"]),
            query_get_any(query, &["host", "h2Host"]),
        ),
        Some("httpupgrade") => put_http_upgrade_opts(
            proxy,
            query_get_any(query, &["path", "httpUpgradePath"]),
            query_get_any(query, &["host", "httpUpgradeHost"]),
        ),
        _ => {}
    }
}

pub(super) fn put_ws_opts(
    proxy: &mut Mapping,
    path: Option<&str>,
    host: Option<&str>,
    max_early_data: Option<&str>,
    early_data_header_name: Option<&str>,
) {
    let mut ws_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut ws_opts, "path", path);
    }
    if let Some(host) = host {
        let mut headers = Mapping::new();
        put_string(&mut headers, "Host", host);
        ws_opts.insert(value_key("headers"), Value::Mapping(headers));
    }
    if let Some(max_early_data) = max_early_data.and_then(parse_positive_i64) {
        put_i64(&mut ws_opts, "max-early-data", max_early_data);
    }
    if let Some(early_data_header_name) = early_data_header_name {
        put_string(
            &mut ws_opts,
            "early-data-header-name",
            early_data_header_name,
        );
    }
    if !ws_opts.is_empty() {
        proxy.insert(value_key("ws-opts"), Value::Mapping(ws_opts));
    }
}

pub(super) fn put_grpc_opts(proxy: &mut Mapping, service_name: Option<&str>, mode: Option<&str>) {
    let mut grpc_opts = Mapping::new();
    if let Some(service_name) = service_name {
        put_string(&mut grpc_opts, "grpc-service-name", service_name);
    }
    if let Some(mode) = mode {
        put_string(&mut grpc_opts, "grpc-mode", mode);
    }
    if !grpc_opts.is_empty() {
        proxy.insert(value_key("grpc-opts"), Value::Mapping(grpc_opts));
    }
}

pub(super) fn put_h2_opts(proxy: &mut Mapping, path: Option<&str>, host: Option<&str>) {
    let mut h2_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut h2_opts, "path", path);
    }
    if let Some(host) = host {
        let hosts = split_list(host);
        if !hosts.is_empty() {
            put_string_sequence(&mut h2_opts, "host", hosts);
        }
    }
    if !h2_opts.is_empty() {
        proxy.insert(value_key("h2-opts"), Value::Mapping(h2_opts));
    }
}

pub(super) fn put_http_upgrade_opts(proxy: &mut Mapping, path: Option<&str>, host: Option<&str>) {
    let mut http_upgrade_opts = Mapping::new();
    if let Some(path) = path {
        put_string(&mut http_upgrade_opts, "path", path);
    }
    if let Some(host) = host {
        put_string(&mut http_upgrade_opts, "host", host);
    }
    if !http_upgrade_opts.is_empty() {
        proxy.insert(
            value_key("http-upgrade-opts"),
            Value::Mapping(http_upgrade_opts),
        );
    }
}

pub(super) fn apply_query_tls_options(proxy: &mut Mapping, query: &HashMap<String, String>) {
    if let Some(fingerprint) = query_get_any(
        query,
        &[
            "fp",
            "fingerprint",
            "client-fingerprint",
            "clientFingerprint",
        ],
    ) {
        put_string(proxy, "client-fingerprint", fingerprint);
    }
    if let Some(alpn) = query.get("alpn") {
        put_string_sequence(proxy, "alpn", split_list(alpn));
    }
    if query
        .get("security")
        .is_some_and(|security| security.eq_ignore_ascii_case("reality"))
    {
        let mut reality_opts = Mapping::new();
        if let Some(public_key) = query_get_any(query, &["pbk", "publicKey", "public-key"]) {
            put_string(&mut reality_opts, "public-key", public_key);
        }
        if let Some(short_id) = query_get_any(query, &["sid", "shortId", "short-id"]) {
            put_string(&mut reality_opts, "short-id", short_id);
        }
        if let Some(spider_x) = query_get_any(query, &["spx", "spiderX", "spider-x"]) {
            put_string(&mut reality_opts, "spider-x", spider_x);
        }
        if !reality_opts.is_empty() {
            proxy.insert(value_key("reality-opts"), Value::Mapping(reality_opts));
        }
    }
}

pub(super) fn proxy_subscription_yaml(mut proxies: Vec<Mapping>) -> Result<String, HMetaError> {
    dedup_proxy_names(&mut proxies);
    let proxy_names = proxies
        .iter()
        .filter_map(|proxy| get_string(proxy, "name"))
        .collect::<Vec<_>>();
    let mut root = Mapping::new();
    put_i64(&mut root, "mixed-port", i64::from(APP_MIXED_PROXY_PORT));
    put_string(&mut root, "mode", "rule");
    put_string(&mut root, "log-level", "info");
    root.insert(
        value_key("proxies"),
        Value::Sequence(proxies.into_iter().map(Value::Mapping).collect()),
    );

    let mut group = Mapping::new();
    put_string(&mut group, "name", "Proxy");
    put_string(&mut group, "type", "select");
    let mut group_proxies = proxy_names
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    group_proxies.push(Value::String("DIRECT".to_owned()));
    group.insert(value_key("proxies"), Value::Sequence(group_proxies));
    root.insert(
        value_key("proxy-groups"),
        Value::Sequence(vec![Value::Mapping(group)]),
    );
    root.insert(
        value_key("rules"),
        Value::Sequence(
            DEFAULT_PROXY_SUBSCRIPTION_RULES
                .iter()
                .map(|rule| Value::String((*rule).to_owned()))
                .collect(),
        ),
    );
    serde_yaml::to_string(&Value::Mapping(root)).map_err(|err| HMetaError::Core(err.to_string()))
}

pub(super) fn proxy_base(name: String, proxy_type: &str, server: String, port: u16) -> Mapping {
    let mut proxy = Mapping::new();
    put_string(&mut proxy, "name", &name);
    put_string(&mut proxy, "type", proxy_type);
    put_string(&mut proxy, "server", &server);
    put_i64(&mut proxy, "port", i64::from(port));
    proxy
}

pub(super) fn dedup_proxy_names(proxies: &mut [Mapping]) {
    let mut counts = HashMap::<String, usize>::new();
    for proxy in proxies {
        let Some(name) = get_string(proxy, "name") else {
            continue;
        };
        let count = counts.entry(name.clone()).or_insert(0);
        if *count > 0 {
            put_string(proxy, "name", &format!("{name} {}", *count + 1));
        }
        *count += 1;
    }
}

pub(super) fn query_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .fold(HashMap::new(), |mut map, (key, value)| {
            insert_query_value(&mut map, key, value);
            map
        })
}

pub(super) fn insert_query_value(map: &mut HashMap<String, String>, key: String, value: String) {
    let lower_key = key.to_ascii_lowercase();
    map.entry(lower_key).or_insert_with(|| value.clone());
    map.entry(key).or_insert(value);
}

pub(super) fn truthy_query(query: &HashMap<String, String>, key: &str) -> bool {
    query_get(query, key).is_some_and(truthy_value)
}

pub(super) fn query_bool(query: &HashMap<String, String>, key: &str) -> Option<bool> {
    query_get(query, key).and_then(parse_boolish_value)
}

pub(super) fn truthy_query_any(query: &HashMap<String, String>, keys: &[&str]) -> bool {
    keys.iter().any(|key| truthy_query(query, key))
}

pub(super) fn truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "allow"
    )
}

pub(super) fn parse_boolish_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "allow" | "on" => Some(true),
        "" | "0" | "false" | "no" | "none" | "off" | "deny" => Some(false),
        _ => None,
    }
}

pub(super) fn tls_enabled_value(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "none"
    )
}

pub(super) fn query_get_any<'a>(
    query: &'a HashMap<String, String>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| query_get(query, key))
        .filter(|value| !value.is_empty())
}

pub(super) fn query_get<'a>(query: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    query
        .get(key)
        .or_else(|| query.get(&key.to_ascii_lowercase()))
        .map(String::as_str)
}

pub(super) fn json_value_any<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| value.get(*key)).or_else(|| {
        let object = value.as_object()?;
        keys.iter().find_map(|key| {
            object
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
        })
    })
}

pub(super) fn json_str_any<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    json_value_any(value, keys)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

pub(super) fn json_u16_any(value: &serde_json::Value, keys: &[&str]) -> Option<u16> {
    json_value_any(value, keys).and_then(|value| match value {
        serde_json::Value::String(value) => value.parse::<u16>().ok(),
        serde_json::Value::Number(value) => value.as_u64().and_then(|value| {
            if value <= u16::MAX as u64 {
                Some(value as u16)
            } else {
                None
            }
        }),
        _ => None,
    })
}

pub(super) fn json_i64_any(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    json_value_any(value, keys).and_then(|value| match value {
        serde_json::Value::String(value) => value.parse::<i64>().ok(),
        serde_json::Value::Number(value) => value.as_i64(),
        _ => None,
    })
}

pub(super) fn json_truthy_any(value: &serde_json::Value, keys: &[&str]) -> bool {
    json_value_any(value, keys).is_some_and(|value| match value {
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::String(value) => truthy_value(value),
        serde_json::Value::Number(value) => value.as_u64() == Some(1),
        _ => false,
    })
}

pub(super) fn url_host(url: &Url) -> Result<String, HMetaError> {
    url.host_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| HMetaError::Core("subscription link missing host".to_owned()))
}

pub(super) fn url_port(url: &Url) -> Result<u16, HMetaError> {
    url.port()
        .ok_or_else(|| HMetaError::Core("subscription link missing port".to_owned()))
}

pub(super) fn url_credentials(url: &Url) -> Option<(String, String)> {
    let username = decode_component(url.username());
    let password = url.password().map(decode_component).unwrap_or_default();
    if username.is_empty() && password.is_empty() {
        None
    } else {
        Some((username, password))
    }
}

pub(super) fn fragment_name(url: &Url) -> Option<String> {
    url.fragment()
        .map(decode_component)
        .filter(|name| !name.is_empty())
}

pub(super) fn proxy_name_from_url(
    url: &Url,
    query: &HashMap<String, String>,
    prefix: &str,
) -> String {
    fragment_name(url)
        .or_else(|| proxy_name_from_query(query))
        .unwrap_or_else(|| default_proxy_name(prefix, url))
}

pub(super) fn proxy_name_from_query(query: &HashMap<String, String>) -> Option<String> {
    query_get_any(
        query,
        &[
            "remarks",
            "remark",
            "name",
            "ps",
            "alias",
            "node",
            "node-name",
            "nodeName",
        ],
    )
    .map(ToOwned::to_owned)
}

pub(super) fn default_proxy_name(prefix: &str, url: &Url) -> String {
    format!(
        "{prefix}-{}",
        url.host_str().unwrap_or("proxy").replace(['[', ']'], "")
    )
}

pub(super) fn decode_component(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

pub(super) fn subscription_error(error: url::ParseError) -> HMetaError {
    HMetaError::Core(format!("invalid subscription link: {error}"))
}
