//! Pure resource compilation. The bridge reads explicit prepared paths; this
//! module consumes bounded bytes and returns native headless matcher options.
use maxminddb::{PathElement, Reader};
use meow_rules::geosite_dat::{visit_dat_category, DatError, GeoSitePatternKind};
use paws_engine::EngineError;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ops::ControlFlow;

// The shipped ASN resource is ~12 MiB; a provider's 8 MiB budget is not a
// meaningful Geo bound. Keep a separate cap, with explicit output/entry limits.
pub(crate) const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_CATEGORY_BYTES: usize = 512;

#[derive(Clone, Copy)]
pub(crate) enum GeoKind {
    Site,
    Country,
    Asn,
}
fn error(reason: &'static str) -> EngineError {
    EngineError::InvalidConfig(format!("sing-box geodata conversion: {reason}"))
}

/// Input is a little-endian u32 category byte length, UTF-8 category, then the
/// original binary resource. It has no NUL scanning or implicit file access.
pub(crate) fn compile(kind: GeoKind, bytes: &[u8]) -> Result<Value, EngineError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(error("resource exceeds 64 MiB budget"));
    }
    let length = u32::from_le_bytes(
        bytes
            .get(..4)
            .ok_or_else(|| error("missing category header"))?
            .try_into()
            .map_err(|_| error("invalid category header"))?,
    ) as usize;
    if length == 0 || length > MAX_CATEGORY_BYTES {
        return Err(error("invalid category length"));
    }
    let end = 4usize
        .checked_add(length)
        .ok_or_else(|| error("category length overflow"))?;
    let category = std::str::from_utf8(
        bytes
            .get(4..end)
            .ok_or_else(|| error("truncated category"))?,
    )
    .map_err(|_| error("category is not UTF-8"))?;
    let source = bytes
        .get(end..)
        .ok_or_else(|| error("missing resource bytes"))?;
    let rules = match kind {
        GeoKind::Site => site_rules(category, source)?,
        GeoKind::Country | GeoKind::Asn => vec![network_rule(kind, category, source)?],
    };
    Ok(json!({"ruleCount":rules.len(),"rules":rules}))
}
fn site_rules(category: &str, source: &[u8]) -> Result<Vec<Value>, EngineError> {
    let mut parts = category.split('@');
    let base = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error("empty geosite category"))?;
    let attributes: Vec<_> = parts.collect();
    let buckets: Vec<String> = if attributes.is_empty() {
        vec![base.to_owned()]
    } else {
        attributes
            .iter()
            .map(|attribute| {
                if attribute.is_empty() {
                    Err(error("empty geosite attribute"))
                } else {
                    Ok(format!("{base}@{attribute}"))
                }
            })
            .collect::<Result<_, _>>()?
    };
    let mut rules = Vec::with_capacity(buckets.len());
    let mut estimated_bytes = 1024usize;
    for bucket in buckets {
        let mut fields: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        let mut entries = 0usize;
        visit_dat_category(source, &bucket, |kind, value| {
            entries += 1;
            estimated_bytes = estimated_bytes.saturating_add(json_string_bytes(value) + 3);
            if entries > MAX_ENTRIES || estimated_bytes > super::provider::MAX_OUTPUT_BYTES {
                return ControlFlow::Break(());
            }
            let key = match kind {
                GeoSitePatternKind::Keyword => "domain_keyword",
                GeoSitePatternKind::Regex => "domain_regex",
                GeoSitePatternKind::Suffix => "domain_suffix",
                GeoSitePatternKind::Full => "domain",
            };
            fields.entry(key).or_default().push(value.to_owned());
            ControlFlow::Continue(())
        })
        .map_err(|error_kind| match error_kind {
            DatError::VisitorStopped => {
                error("geosite category exceeds entry or output byte budget")
            }
            DatError::MissingCategory => error("requested geosite category is missing"),
            _ => error("invalid geosite binary resource"),
        })?;
        if entries > MAX_ENTRIES {
            return Err(error("geosite category exceeds entry budget"));
        }
        if entries == 0 {
            return Err(error("geosite category is empty"));
        }
        for pattern in fields.get("domain_regex").into_iter().flatten() {
            regex::Regex::new(pattern).map_err(|_| error("geosite regex is not representable"))?;
        }
        rules.push(
            serde_json::to_value(fields).map_err(|_| error("geosite matcher encoding failed"))?,
        );
    }
    // GeositeDB's multi-attribute semantics are intersection, not a union of
    // entries that happen to carry any one attribute.
    if rules.len() == 1 {
        Ok(rules)
    } else {
        Ok(vec![json!({"type":"logical","mode":"and","rules":rules})])
    }
}
fn network_rule(kind: GeoKind, category: &str, source: &[u8]) -> Result<Value, EngineError> {
    let reader = Reader::from_source(source).map_err(|_| error("invalid MMDB resource"))?;
    let asn = if matches!(kind, GeoKind::Asn) {
        Some(
            category
                .strip_prefix("AS")
                .or_else(|| category.strip_prefix("as"))
                .unwrap_or(category)
                .parse::<u32>()
                .map_err(|_| error("invalid ASN category"))?,
        )
    } else {
        None
    };
    if asn.is_none()
        && (category.is_empty()
            || category.len() > 8
            || !category
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    {
        return Err(error("invalid country category"));
    }
    let networks = reader
        .networks(Default::default())
        .map_err(|_| error("MMDB network iteration failed"))?;
    let mut offsets = BTreeMap::new();
    let mut cidrs = Vec::new();
    let mut estimated_bytes = 128usize;
    for entry in networks {
        let entry = entry.map_err(|_| error("invalid MMDB network entry"))?;
        let Some(offset) = entry.offset() else {
            continue;
        };
        let matches = if let Some(matches) = offsets.get(&offset) {
            *matches
        } else {
            let matches = if let Some(asn) = asn {
                entry
                    .decode_path::<u32>(&[PathElement::Key("autonomous_system_number")])
                    .map_err(|_| error("invalid ASN record"))?
                    == Some(asn)
            } else {
                entry
                    .decode_path::<&str>(&[
                        PathElement::Key("country"),
                        PathElement::Key("iso_code"),
                    ])
                    .map_err(|_| error("invalid country record"))?
                    .is_some_and(|country| country.eq_ignore_ascii_case(category))
            };
            if offsets.len() < MAX_ENTRIES {
                offsets.insert(offset, matches);
            }
            matches
        };
        if !matches {
            continue;
        }
        let network = entry.network().map_err(|_| error("invalid MMDB prefix"))?;
        let cidr = format!("{}/{}", network.network(), network.prefix());
        estimated_bytes += cidr.len() + 3;
        if estimated_bytes > super::provider::MAX_OUTPUT_BYTES {
            return Err(error("MMDB category exceeds output byte budget"));
        }
        cidrs.push(cidr);
        if cidrs.len() > MAX_ENTRIES {
            return Err(error("MMDB category exceeds entry budget"));
        }
    }
    if cidrs.is_empty() {
        return Err(error("requested MMDB category has no networks"));
    }
    Ok(json!({"ip_cidr":cidrs}))
}

fn json_string_bytes(value: &str) -> usize {
    value
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_lengths_and_binary_resources_are_explicit_errors() {
        for bytes in [
            vec![],
            1u32.to_le_bytes().to_vec(),
            [1u32.to_le_bytes().as_slice(), b"x", b"not protobuf"].concat(),
        ] {
            assert!(compile(GeoKind::Site, &bytes).is_err());
        }
        let bytes = [2u32.to_le_bytes().as_slice(), b"CN", b"not mmdb"].concat();
        assert!(compile(GeoKind::Country, &bytes).is_err());
    }
}
