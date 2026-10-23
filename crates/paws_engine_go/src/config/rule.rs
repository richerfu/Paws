//! One Clash matcher compiler for the initial route and refreshed providers.
use paws_engine::EngineError;
use serde_json::{json, Value};

const MAX_DEPTH: usize = 64;
// Pinned Mihomo GEOIP,LAN: IsPrivate, IsUnspecified, IsLoopback,
// IsMulticast or IsLinkLocalUnicast. Native !IsPublicAddr is a wider set.
const LAN_NETWORKS: [&str; 12] = [
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "fc00::/7",
    "0.0.0.0/32",
    "::/128",
    "127.0.0.0/8",
    "::1/128",
    "224.0.0.0/4",
    "ff00::/8",
    "169.254.0.0/16",
    "fe80::/10",
];

fn invalid(kind: &str, reason: &'static str) -> EngineError {
    // Never echo an arbitrary source token, regex, target or provider payload.
    let kind = match kind {
        "DOMAIN"
        | "DOMAIN-SUFFIX"
        | "DOMAIN-KEYWORD"
        | "DOMAIN-REGEX"
        | "DOMAIN-WILDCARD"
        | "IP-CIDR"
        | "IP-CIDR6"
        | "SRC-IP-CIDR"
        | "IP-SUFFIX"
        | "SRC-IP-SUFFIX"
        | "DST-PORT"
        | "SRC-PORT"
        | "IN-PORT"
        | "DSCP"
        | "NETWORK"
        | "PROCESS-NAME"
        | "PROCESS-PATH"
        | "PROCESS-NAME-REGEX"
        | "PROCESS-PATH-REGEX"
        | "PROCESS-NAME-WILDCARD"
        | "PROCESS-PATH-WILDCARD"
        | "UID"
        | "IN-NAME"
        | "IN-USER"
        | "IN-TYPE"
        | "REMATCH-NAME"
        | "SUB-RULE"
        | "GEOIP"
        | "GEOSITE"
        | "SRC-GEOIP"
        | "IP-ASN"
        | "SRC-IP-ASN"
        | "IP-VERSION"
        | "RULE-SET"
        | "AND"
        | "OR"
        | "NOT"
        | "MATCH"
        | "FINAL" => kind,
        _ => "UNKNOWN",
    };
    EngineError::InvalidConfig(format!("sing-box rule {kind}: {reason}"))
}

fn kind_and_body(source: &str) -> Result<(String, &str), EngineError> {
    let (kind, body) = source
        .split_once(',')
        .ok_or_else(|| invalid("UNKNOWN", "missing rule fields"))?;
    Ok((kind.to_ascii_uppercase(), body))
}

fn normalize_source(source: &str) -> String {
    // Exact imported Clash grammar: ParseRulePayload trims ASCII spaces from
    // each comma fragment before joining regex/logical payloads again.
    source
        .split(',')
        .map(|part| part.trim_matches(' '))
        .collect::<Vec<_>>()
        .join(",")
}

fn is_regex(kind: &str) -> bool {
    matches!(
        kind,
        "DOMAIN-REGEX" | "PROCESS-NAME-REGEX" | "PROCESS-PATH-REGEX"
    )
}

fn is_logical(kind: &str) -> bool {
    matches!(kind, "AND" | "OR" | "NOT")
}

/// Verify the imported Clash raw-parenthesis grammar as well as regex-aware
/// boundaries. Ambiguous combinations must not acquire different semantics.
fn raw_logical_entries<'a>(kind: &str, inner: &'a str) -> Result<Vec<&'a str>, EngineError> {
    let mut result = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, byte) in inner.bytes().enumerate() {
        match byte {
            b'(' => {
                if depth == 0 {
                    start = index + 1;
                }
                depth += 1;
            }
            b')' => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    invalid(
                        kind,
                        "ambiguous regular expression parentheses in Clash logical syntax",
                    )
                })?;
                if depth == 0 {
                    result.push(inner[start..index].trim_matches(' '));
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(invalid(
            kind,
            "ambiguous regular expression parentheses in Clash logical syntax",
        ));
    }
    Ok(result)
}

/// Regex commas are never split into fields. Escape/class-aware boundaries
/// must also agree with the raw imported logical grammar above.
fn logical_entries<'a>(kind: &str, value: &'a str) -> Result<Vec<&'a str>, EngineError> {
    let inner = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| invalid(kind, "malformed logical expression"))?;
    let mut result = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    let mut escaped = false;
    let mut in_class = false;
    let mut class_first = false;
    let mut expecting_entry = true;
    for (index, byte) in inner.bytes().enumerate() {
        if depth == 0 {
            if byte.is_ascii_whitespace() {
                continue;
            }
            if expecting_entry && byte == b'(' {
                start = index + 1;
                depth = 1;
                expecting_entry = false;
                continue;
            }
            if !expecting_entry && byte == b',' {
                expecting_entry = true;
                continue;
            }
            return Err(invalid(kind, "malformed logical operand"));
        }
        if escaped {
            escaped = false;
            if in_class {
                class_first = false;
            }
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if in_class {
            if byte == b']' && !class_first {
                in_class = false;
            } else if byte != b'^' || !class_first {
                class_first = false;
            }
            continue;
        }
        match byte {
            b'[' => {
                in_class = true;
                class_first = true;
            }
            b'(' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(invalid(kind, "logical nesting limit exceeded"));
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    result.push(inner[start..index].trim_matches(' '));
                }
            }
            _ => {}
        }
    }
    if depth != 0 || escaped || in_class || expecting_entry || result.is_empty() {
        return Err(invalid(kind, "malformed logical expression"));
    }
    if raw_logical_entries(kind, inner)? != result {
        return Err(invalid(
            kind,
            "ambiguous regular expression parentheses in Clash logical syntax",
        ));
    }
    Ok(result)
}

#[derive(Clone, Copy, Default)]
struct Flags {
    source: bool,
    no_resolve: bool,
}

fn flags<'a>(kind: &str, values: impl Iterator<Item = &'a str>) -> Result<Flags, EngineError> {
    let mut flags = Flags::default();
    for value in values {
        if value == "src" {
            flags.source = true;
            flags.no_resolve = true;
        } else if value == "no-resolve" {
            flags.no_resolve = true;
        } else {
            return Err(invalid(kind, "unsupported rule parameter"));
        }
    }
    Ok(flags)
}

struct Compiled {
    value: Value,
    resolve: bool,
    forbids_resolve: bool,
}

fn port(kind: &str, value: &str, field: &str, range_field: &str) -> Result<Value, EngineError> {
    let mut ports = Vec::new();
    let mut ranges = Vec::new();
    for part in value.split('/') {
        if let Some((start, end)) = part.split_once('-') {
            let first = start
                .parse::<u16>()
                .map_err(|_| invalid(kind, "invalid port range"))?;
            let last = end
                .parse::<u16>()
                .map_err(|_| invalid(kind, "invalid port range"))?;
            if first > last {
                return Err(invalid(kind, "invalid port range"));
            }
            ranges.push(format!("{first}:{last}"));
        } else {
            ports.push(
                part.parse::<u16>()
                    .map_err(|_| invalid(kind, "invalid port"))?,
            );
        }
    }
    let mut matches = Vec::new();
    if !ports.is_empty() {
        matches.push(json!({field:ports}));
    }
    if !ranges.is_empty() {
        matches.push(json!({range_field:ranges}));
    }
    Ok(if matches.len() == 1 {
        matches.remove(0)
    } else {
        json!({"type":"logical","mode":"or","rules":matches})
    })
}

fn names<'a>(kind: &str, payload: &'a str) -> Result<Vec<&'a str>, EngineError> {
    payload
        .split('/')
        .map(str::trim)
        .map(|name| {
            if name.is_empty() {
                Err(invalid(kind, "empty name in rule list"))
            } else {
                Ok(name)
            }
        })
        .collect()
}

fn quote_regex(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            output.push('\\');
        }
        output.push(character);
    }
    output
}

fn native_regex(kind: &str, payload: &str) -> Result<String, EngineError> {
    // Both common regex types use IgnoreCase. Native Sing uses Go RE2, not
    // regexp2. Validate a conservative common syntax before publishing JSON.
    // Rust-only flags/classes/escapes must not be accepted as native support.
    let bytes = payload.as_bytes();
    let mut index = 0;
    let mut in_class = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 1;
                let Some(&escaped) = bytes.get(index) else {
                    return Err(invalid(kind, "invalid regular expression"));
                };
                if escaped.is_ascii_alphanumeric()
                    && !matches!(
                        escaped,
                        b'A' | b'z' | b'a' | b'f' | b't' | b'n' | b'r' | b'v' | b'x'
                    )
                {
                    return Err(invalid(
                        kind,
                        "regular expression escape is outside the faithful native subset",
                    ));
                }
            }
            b'[' => {
                if in_class {
                    return Err(invalid(
                        kind,
                        "regular expression class is outside the faithful native subset",
                    ));
                }
                in_class = true;
            }
            b']' => in_class = false,
            b'&' | b'-' | b'~' if in_class && bytes.get(index + 1) == bytes.get(index) => {
                return Err(invalid(
                    kind,
                    "regular expression class operation is not native syntax",
                ));
            }
            b'(' if !in_class && bytes.get(index + 1) == Some(&b'?') => {
                let mut flag = index + 2;
                if bytes.get(flag) != Some(&b':') {
                    while bytes
                        .get(flag)
                        .is_some_and(|byte| matches!(byte, b'i' | b'm' | b's' | b'U' | b'-'))
                    {
                        flag += 1;
                    }
                    if flag == index + 2
                        || !bytes
                            .get(flag)
                            .is_some_and(|byte| matches!(byte, b':' | b')'))
                    {
                        return Err(invalid(
                            kind,
                            "regular expression construct is not supported by the native matcher",
                        ));
                    }
                    if kind == "DOMAIN-REGEX"
                        && payload[index + 2..flag]
                            .split_once('-')
                            .is_some_and(|(_, disabled)| disabled.contains('i'))
                    {
                        return Err(invalid(
                            kind,
                            "native domain matching cannot disable case folding",
                        ));
                    }
                }
            }
            b'{' if !in_class => {
                if let Some(end) = payload[index + 1..].find('}') {
                    let range = &payload[index + 1..index + 1 + end];
                    if range.split(',').all(|part| {
                        part.is_empty() || part.bytes().all(|byte| byte.is_ascii_digit())
                    }) && range
                        .split(',')
                        .filter_map(|part| part.parse::<u64>().ok())
                        .any(|count| count > 1000)
                    {
                        return Err(invalid(
                            kind,
                            "native regular expression repeat limit exceeded",
                        ));
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    let pattern = format!("(?i){payload}");
    let expression = regex::Regex::new(&pattern).map_err(|_| {
        invalid(
            kind,
            "regular expression is outside the supported native syntax",
        )
    })?;
    if expression.is_match("") {
        return Err(invalid(
            kind,
            if kind == "DOMAIN-REGEX" {
                "native domain matching cannot match an empty host"
            } else {
                "native process matching cannot match an empty path"
            },
        ));
    }
    Ok(pattern)
}

fn compile(kind: &str, body: &str, depth: usize) -> Result<Compiled, EngineError> {
    if depth > MAX_DEPTH {
        return Err(invalid(kind, "logical nesting limit exceeded"));
    }
    if is_logical(kind) {
        let operands = logical_entries(kind, body)?
            .into_iter()
            .map(|entry| compile_source(entry, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        if kind == "NOT" && operands.len() != 1 {
            return Err(invalid(kind, "NOT requires exactly one operand"));
        }
        let resolve = operands.iter().any(|operand| operand.resolve);
        let forbids_resolve = operands.iter().any(|operand| operand.forbids_resolve);
        if resolve && forbids_resolve {
            return Err(invalid(
                kind,
                "mixed destination-resolution flags require conditional native resolution",
            ));
        }
        if resolve {
            return Err(invalid(
                kind,
                "logical destination matching requires conditional native resolution",
            ));
        }
        return Ok(Compiled {
            value: json!({"type":"logical","mode":if kind=="OR" {"or"}else{"and"},"invert":kind=="NOT","rules":operands.into_iter().map(|operand|operand.value).collect::<Vec<_>>()}),
            resolve,
            forbids_resolve,
        });
    }
    if is_regex(kind) {
        if body.is_empty() {
            return Err(invalid(kind, "missing regular expression"));
        }
        if kind == "PROCESS-NAME-REGEX" {
            return Err(invalid(
                kind,
                "native matcher has no basename regular expression predicate",
            ));
        }
        let field = if kind == "DOMAIN-REGEX" {
            "domain_regex"
        } else {
            "process_path_regex"
        };
        return Ok(Compiled {
            value: json!({field:[native_regex(kind, body)?]}),
            resolve: false,
            forbids_resolve: false,
        });
    }
    let mut parts = body.split(',');
    let payload = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(kind, "missing rule payload"))?;
    let mut flags = flags(kind, parts)?;
    if matches!(kind, "SRC-IP-CIDR" | "SRC-GEOIP" | "SRC-IP-ASN") {
        flags.source = true;
        flags.no_resolve = true;
    }
    if flags.source
        && !matches!(
            kind,
            "IP-CIDR"
                | "IP-CIDR6"
                | "SRC-IP-CIDR"
                | "GEOIP"
                | "SRC-GEOIP"
                | "IP-ASN"
                | "SRC-IP-ASN"
                | "RULE-SET"
        )
    {
        return Err(invalid(
            kind,
            "source matching is not defined for this rule type",
        ));
    }
    let mut destination_ip = false;
    let value = match kind {
        "DOMAIN" => json!({"domain":[payload]}),
        "DOMAIN-SUFFIX" => json!({"domain_suffix":[payload.trim_start_matches('.')]}),
        "DOMAIN-KEYWORD" => json!({"domain_keyword":[payload]}),
        "DOMAIN-WILDCARD" => return Err(invalid(kind, "native domain matching folds the original host and cannot preserve byte wildcard semantics")),
        "IP-CIDR" | "IP-CIDR6" | "SRC-IP-CIDR" => {
            let (address, prefix) = payload
                .split_once('/')
                .ok_or_else(|| invalid(kind, "invalid IP prefix"))?;
            let address = address
                .parse::<std::net::IpAddr>()
                .map_err(|_| invalid(kind, "invalid IP prefix"))?;
            let prefix = prefix
                .parse::<u8>()
                .map_err(|_| invalid(kind, "invalid IP prefix"))?;
            if prefix > if address.is_ipv4() { 32 } else { 128 } {
                return Err(invalid(kind, "invalid IP prefix"));
            }
            destination_ip = !flags.source;
            json!({(if flags.source {"source_ip_cidr"}else{"ip_cidr"}):[payload]})
        }
        "DST-PORT" => port(kind, payload, "port", "port_range")?,
        "SRC-PORT" => port(kind, payload, "source_port", "source_port_range")?,
        "NETWORK" if payload.eq_ignore_ascii_case("tcp") || payload.eq_ignore_ascii_case("udp") => {
            json!({"network":[payload.to_ascii_lowercase()]})
        }
        "NETWORK" => return Err(invalid(kind, "network must be TCP or UDP")),
        "PROCESS-NAME" => {
            if payload.contains('/') {
                return Err(invalid(kind, "process name must be a basename"));
            }
            json!({"process_path_regex":[format!("(?i)(?:\\A|/){}\\z",quote_regex(payload))]})
        }
        "PROCESS-PATH" => {
            json!({"process_path_regex":[format!("(?i)\\A{}\\z",quote_regex(payload))]})
        }
        "PROCESS-NAME-WILDCARD" | "PROCESS-PATH-WILDCARD" => {
            return Err(invalid(
                kind,
                "byte wildcard and case semantics need a native process predicate",
            ))
        }
        "UID" => {
            json!({"user_id":[payload.parse::<i32>().ok().filter(|value|*value>=0).ok_or_else(||invalid(kind,"native matcher supports one nonnegative signed UID"))?]})
        }
        "IN-NAME" => json!({"inbound":names(kind,payload)?}),
        "IN-USER" => json!({"auth_user":names(kind,payload)?}),
        "GEOIP" | "SRC-GEOIP" if payload.eq_ignore_ascii_case("LAN") => {
            destination_ip = !flags.source;
            json!({(if flags.source {"source_ip_cidr"}else{"ip_cidr"}):LAN_NETWORKS})
        }
        "GEOSITE" => json!({"rule_set":[format!("paws-geosite:{payload}")]}),
        "GEOIP" | "SRC-GEOIP" => {
            destination_ip = !flags.source;
            json!({"rule_set":[format!("paws-geoip:{payload}")],"rule_set_ip_cidr_match_source":flags.source})
        }
        "IP-ASN" | "SRC-IP-ASN" => {
            destination_ip = !flags.source;
            json!({"rule_set":[format!("paws-asn:{payload}")],"rule_set_ip_cidr_match_source":flags.source})
        }
        "IP-VERSION" if payload == "4" || payload == "6" => {
            json!({"ip_version":payload.parse::<u8>().map_err(|_|invalid(kind,"invalid IP version"))?})
        }
        "IP-VERSION" => return Err(invalid(kind, "IP version must be 4 or 6")),
        "RULE-SET" if flags.source => {
            return Err(invalid(
                kind,
                "source RULE-SET requires tuple-aware provider compilation",
            ))
        }
        "RULE-SET" => json!({"rule_set":[format!("paws-provider:{payload}")]}),
        "IN-TYPE" => {
            return Err(invalid(
                kind,
                "native mixed inbound does not expose the original handshake type",
            ))
        }
        "IN-PORT" => {
            return Err(invalid(
                kind,
                "native matcher does not expose the actual listener port",
            ))
        }
        "DSCP" => return Err(invalid(kind, "native matcher does not expose packet DSCP")),
        "IP-SUFFIX" | "SRC-IP-SUFFIX" => {
            return Err(invalid(
                kind,
                "native IP prefix matching cannot represent suffix bits",
            ))
        }
        "REMATCH-NAME" => return Err(invalid(kind, "native routing has no rematch-name state")),
        "SUB-RULE" => {
            return Err(invalid(
                kind,
                "ordered sub-rule expansion is not implemented",
            ))
        }
        _ => return Err(invalid(kind, "unsupported rule type")),
    };
    Ok(Compiled {
        value,
        resolve: destination_ip && !flags.no_resolve,
        forbids_resolve: destination_ip && flags.no_resolve,
    })
}

fn compile_source(source: &str, depth: usize) -> Result<Compiled, EngineError> {
    let (kind, body) = kind_and_body(source)?;
    compile(&kind, body, depth)
}

#[cfg(any(target_env = "ohos", test))]
pub(super) fn matcher(source: &str) -> Result<Value, EngineError> {
    Ok(compile_source(&normalize_source(source), 0)?.value)
}

pub(super) fn route(source: &str) -> Result<(Value, String), EngineError> {
    let source = normalize_source(source);
    let (kind, body) = kind_and_body(&source)?;
    if kind == "MATCH" || kind == "FINAL" {
        if body.is_empty() || body.contains(',') {
            return Err(invalid(
                &kind,
                "final rule requires exactly one route target",
            ));
        }
        return Ok((json!({}), super::sing_box_tag(body)));
    }
    let (matcher_source, target) = if is_regex(&kind) || is_logical(&kind) {
        let (payload, target) = body
            .rsplit_once(',')
            .ok_or_else(|| invalid(&kind, "missing route target"))?;
        (format!("{kind},{payload}"), target)
    } else {
        let mut parts = body.split(',');
        let payload = parts.next().unwrap_or_default();
        let target = parts
            .next()
            .ok_or_else(|| invalid(&kind, "missing route target"))?;
        let mut matcher_source = format!("{kind},{payload}");
        for parameter in parts {
            matcher_source.push(',');
            matcher_source.push_str(parameter);
        }
        (matcher_source, target)
    };
    if target.is_empty() {
        return Err(invalid(&kind, "missing route target"));
    }
    let mut compiled = compile_source(&matcher_source, 0)?;
    if compiled.resolve {
        compiled.value["paws_resolve"] = json!(true);
    }
    // Expanded by the native bridge at this source-rule position.
    Ok((compiled.value, super::sing_box_tag(target)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_rules_keep_nested_conditions_and_targets() {
        let (rule, target) =
            route("AND,((DOMAIN,example.com),(OR,((DST-PORT,443),(NETWORK,udp)))),main").unwrap();
        assert_eq!(target, "main");
        assert_eq!(rule["mode"], "and");
        assert_eq!(rule["rules"][1]["rules"][1]["network"][0], "udp");
    }
    #[test]
    fn invalid_or_unrepresentable_rules_never_become_empty_success() {
        for rule in [
            "UNKNOWN,a,DIRECT",
            "IP-CIDR,no-prefix,DIRECT",
            "AND,((DOMAIN,a),()),DIRECT",
            "MATCH,DIRECT,extra",
        ] {
            assert!(route(rule).is_err(), "{rule}");
        }
        assert!(route("MATCH,DIRECT")
            .unwrap()
            .0
            .as_object()
            .unwrap()
            .is_empty());
    }
    fn expression(rule: &Value, field: &str) -> regex::Regex {
        regex::Regex::new(rule[field][0].as_str().unwrap()).unwrap()
    }

    #[test]
    fn regex_payload_keeps_commas_escaped_parentheses_and_case_contract() {
        for (source, matched, missed) in [
            (
                r"DOMAIN-REGEX,^x{1,3}\.example$,DIRECT",
                "XX.example",
                "xxxx.example",
            ),
            (
                r"DOMAIN-REGEX,^[a,b]+\.example$,DIRECT",
                "A,b.example",
                "c.example",
            ),
            (r"DOMAIN-REGEX,\(literal,DIRECT", "(LITERAL", "literal"),
            (r"DOMAIN-REGEX,^foo, bar$,DIRECT", "foo,bar", "foo, bar"),
        ] {
            let (rule, target) = route(source).unwrap();
            assert_eq!(target, "direct");
            let pattern = expression(&rule, "domain_regex");
            assert!(pattern.is_match(matched), "{source}");
            assert!(!pattern.is_match(missed), "{source}");
            assert_eq!(matcher(source.rsplit_once(',').unwrap().0).unwrap(), rule);
        }
    }

    #[test]
    fn logical_regex_and_nested_wrappers_do_not_consume_regex_syntax() {
        let (rule, target) = route(r"AND,((DOMAIN-REGEX,^[a,b()]{1,3}\.example$),(OR,((DOMAIN-REGEX,\(literal\)),(NETWORK,tcp)))),DIRECT").unwrap();
        assert_eq!(target, "direct");
        assert_eq!(
            rule["rules"][0]["domain_regex"][0],
            r"(?i)^[a,b()]{1,3}\.example$"
        );
        assert_eq!(
            rule["rules"][1]["rules"][0]["domain_regex"][0],
            r"(?i)\(literal\)"
        );
        for source in [
            r"AND,((DOMAIN-REGEX,\(literal),(NETWORK,tcp)),DIRECT",
            r"AND,((DOMAIN-REGEX,[(]),(NETWORK,tcp)),DIRECT",
        ] {
            assert!(route(source)
                .unwrap_err()
                .to_string()
                .contains("Clash logical syntax"));
        }
        for source in [
            "AND,(),DIRECT",
            "AND,((DOMAIN,x),),DIRECT",
            "NOT,((DOMAIN,x),(NETWORK,tcp)),DIRECT",
            "AND,((DOMAIN,x)junk),DIRECT",
        ] {
            assert!(route(source).is_err());
        }
    }

    #[test]
    fn source_and_resolution_flags_do_not_become_the_route_target() {
        for source in [
            "IP-CIDR,192.0.2.0/24,DIRECT,src",
            "IP-CIDR,192.0.2.0/24,DIRECT,no-resolve,src",
            "SRC-IP-CIDR,192.0.2.0/24,DIRECT",
        ] {
            let (rule, target) = route(source).unwrap();
            assert_eq!(target, "direct");
            assert_eq!(rule["source_ip_cidr"][0], "192.0.2.0/24");
            assert!(rule.get("paws_resolve").is_none());
            assert!(rule.get("ip_cidr").is_none());
        }
        assert!(route("IP-CIDR,192.0.2.0/24,DIRECT,SRC,NO-RESOLVE").is_err());
        for source in ["GEOIP,CN,DIRECT,src", "IP-ASN,13335,DIRECT,src"] {
            let (rule, _) = route(source).unwrap();
            assert_eq!(rule["rule_set_ip_cidr_match_source"], true);
            assert!(rule.get("paws_resolve").is_none());
        }
        assert_eq!(
            route("IP-CIDR,192.0.2.0/24,DIRECT").unwrap().0["paws_resolve"],
            true
        );
        assert!(route("IP-CIDR,192.0.2.0/24,DIRECT,no-resolve")
            .unwrap()
            .0
            .get("paws_resolve")
            .is_none());
    }

    #[test]
    fn logical_no_resolve_is_retained_and_mixed_resolution_is_not_faked() {
        for source in [
            "AND,((IP-CIDR,192.0.2.0/24,no-resolve),(NETWORK,tcp)),DIRECT",
            "NOT,((IP-CIDR,192.0.2.0/24,no-resolve)),DIRECT",
            "OR,((IP-CIDR,192.0.2.0/24,src),(NETWORK,tcp)),DIRECT",
        ] {
            assert!(route(source).unwrap().0.get("paws_resolve").is_none());
        }
        let error =
            route("AND,((IP-CIDR,192.0.2.0/24,no-resolve),(IP-CIDR,198.51.100.0/24)),DIRECT")
                .unwrap_err()
                .to_string();
        assert!(error.contains("rule AND:"));
        assert!(error.contains("conditional native resolution"));
        let error = route("RULE-SET,synthetic-private-name,DIRECT,src")
            .unwrap_err()
            .to_string();
        assert!(error.contains("rule RULE-SET:"));
        assert!(error.contains("tuple-aware"));
        assert!(!error.contains("synthetic-private-name"));
        for source in [
            "OR,((DOMAIN,example.test),(IP-CIDR,192.0.2.0/24)),DIRECT",
            "AND,((NETWORK,udp),(IP-CIDR,192.0.2.0/24)),DIRECT",
        ] {
            assert!(route(source)
                .unwrap_err()
                .to_string()
                .contains("conditional native resolution"));
        }
    }

    #[test]
    fn inbound_name_and_authenticated_user_lists_are_trimmed_exact_alternatives() {
        for (source, field) in [
            ("IN-NAME,tun-in / mixed-in,DIRECT", "inbound"),
            ("IN-USER,alice / bob,DIRECT", "auth_user"),
        ] {
            let (rule, target) = route(source).unwrap();
            assert_eq!(target, "direct");
            assert_eq!(rule[field].as_array().unwrap().len(), 2);
            assert!(!rule[field][0].as_str().unwrap().contains('/'));
        }
        assert!(route("IN-NAME,tun-in//mixed-in,DIRECT").is_err());
        assert!(route("IN-USER,alice/,DIRECT").is_err());
    }

    #[test]
    fn raw_case_and_byte_wildcards_are_not_approximated_by_native_domain_regex() {
        for source in [
            "DOMAIN-WILDCARD,*.example.test,DIRECT",
            "DOMAIN-WILDCARD,?.example.test,DIRECT",
            "DOMAIN-WILDCARD,é.example.test,DIRECT",
            "DOMAIN-WILDCARD,**,DIRECT",
        ] {
            let error = route(source).unwrap_err().to_string();
            assert!(error.contains("rule DOMAIN-WILDCARD:"));
            assert!(error.contains("original host"));
            assert!(!error.contains("example.test"));
        }
    }

    #[test]
    fn process_exact_rules_follow_clash_casefold_basename_and_literal_fullpath() {
        let name = route("PROCESS-NAME,Curl,DIRECT").unwrap().0;
        let pattern = expression(&name, "process_path_regex");
        assert!(pattern.is_match("/usr/bin/cURL"));
        assert!(pattern.is_match("CURL"));
        assert!(!pattern.is_match("/usr/curl/bin"));
        assert!(!pattern.is_match("/usr/bin/another-curl"));
        let path = route("PROCESS-PATH,/usr/*/Curl,DIRECT").unwrap().0;
        let pattern = expression(&path, "process_path_regex");
        assert!(pattern.is_match("/USR/*/curl"));
        assert!(!pattern.is_match("/usr/bin/curl"));
        assert!(!pattern.is_match("/usr/*/curl/child"));
        for source in [
            "PROCESS-NAME-REGEX,^curl$,DIRECT",
            "PROCESS-NAME-WILDCARD,curl*,DIRECT",
            "PROCESS-PATH-WILDCARD,/usr/*/curl,DIRECT",
        ] {
            assert!(route(source).is_err());
        }
    }

    #[test]
    fn unsupported_regex_dialects_fail_before_publishing_json() {
        for source in [
            r"DOMAIN-REGEX,^(?!private).*\.example$,DIRECT",
            r"DOMAIN-REGEX,(a)\1,DIRECT",
            r"DOMAIN-REGEX,(?x)a b,DIRECT",
            r"DOMAIN-REGEX,[a&&b],DIRECT",
            r"DOMAIN-REGEX,a{1001},DIRECT",
            r"PROCESS-PATH-REGEX,(?<=/usr/)curl,DIRECT",
            r"DOMAIN-REGEX,.*$,DIRECT",
            r"DOMAIN-REGEX,(?-i)EXAMPLE,DIRECT",
        ] {
            let error = route(source).unwrap_err().to_string();
            assert!(
                error.contains("rule DOMAIN-REGEX:") || error.contains("rule PROCESS-PATH-REGEX:")
            );
            assert!(!error.contains("private"));
            assert!(!error.contains("/usr/"));
        }
        // These contain the characters of an inline flag as literal data.
        // They must not be mistaken for an actual request to disable folding.
        for source in [
            r"DOMAIN-REGEX,^[(?-i)]+$,DIRECT",
            r"DOMAIN-REGEX,\(\?-i\),DIRECT",
        ] {
            assert!(route(source).is_ok());
        }
    }

    #[test]
    fn lan_uses_the_common_address_set_and_source_axis_without_a_geo_database() {
        let destination = route("GEOIP,LAN,DIRECT,no-resolve").unwrap().0;
        let source = route("SRC-GEOIP,LAN,DIRECT").unwrap().0;
        assert_eq!(destination["ip_cidr"], source["source_ip_cidr"]);
        assert_eq!(source["source_ip_cidr"], json!(LAN_NETWORKS));
        assert!(source.get("rule_set").is_none());
        assert!(source.get("paws_resolve").is_none());
    }

    #[test]
    fn failures_expose_only_allowlisted_type_and_static_reason() {
        for source in [
            "PRIVATE-TOKEN,private-payload,private-target",
            "IN-TYPE,private-payload,private-target",
            "DSCP,private-payload,private-target",
            "IP-SUFFIX,private-payload,private-target",
            "SUB-RULE,private-payload,private-target",
        ] {
            let error = route(source).unwrap_err().to_string();
            assert!(error.contains("sing-box rule "));
            assert!(!error.contains("private-payload"));
            assert!(!error.contains("private-target"));
            assert!(!error.contains("PRIVATE-TOKEN"));
            assert!(!error.contains("cannot faithfully compile this rule shape"));
        }
    }

    #[test]
    fn shared_provider_compiler_preserves_fidelity_guards_and_nonempty_dns_matcher() {
        use super::super::provider::{compile, ProviderFormat, ProviderKind, RuleBehavior};
        let compile = |text: &str| {
            compile(
                ProviderKind::Rule,
                RuleBehavior::Classical,
                ProviderFormat::Text,
                text.as_bytes(),
            )
        };
        let compiled = compile("DOMAIN-REGEX,.+").unwrap();
        assert!(compiled.to_string().contains("(?i).+"));
        for source in [
            "DOMAIN-REGEX,.*",
            "DOMAIN-REGEX,(?-i)EXAMPLE",
            "DOMAIN-WILDCARD,*.example.test",
        ] {
            let error = compile(source).unwrap_err().to_string();
            assert!(error.contains("sing-box rule "));
            assert!(!error.contains("example.test"));
        }
    }
}
