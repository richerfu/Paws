#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct YamlSummary {
    pub(crate) lines: usize,
    pub(crate) characters: usize,
    pub(crate) proxies: usize,
    pub(crate) rules: usize,
    pub(crate) providers: usize,
    pub(crate) changed: bool,
    pub(crate) parseable: bool,
}

pub(crate) fn summarize_yaml_edit(current: &str, original: &str) -> YamlSummary {
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(current).ok();
    YamlSummary {
        lines: current.lines().count(),
        characters: current.chars().count(),
        proxies: parsed
            .as_ref()
            .and_then(|value| sequence_len(value, "proxies"))
            .unwrap_or(0),
        rules: parsed
            .as_ref()
            .and_then(|value| sequence_len(value, "rules"))
            .unwrap_or(0),
        providers: parsed
            .as_ref()
            .map(|value| {
                mapping_len(value, "proxy-providers") + mapping_len(value, "rule-providers")
            })
            .unwrap_or(0),
        changed: current != original,
        parseable: parsed.is_some(),
    }
}

fn sequence_len(value: &serde_yaml::Value, key: &str) -> Option<usize> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.to_owned()))?
        .as_sequence()
        .map(Vec::len)
}

fn mapping_len(value: &serde_yaml::Value, key: &str) -> usize {
    value
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String(key.to_owned())))
        .and_then(serde_yaml::Value::as_mapping)
        .map(serde_yaml::Mapping::len)
        .unwrap_or(0)
}
