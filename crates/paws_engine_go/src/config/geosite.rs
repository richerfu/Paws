//! Borrowed V2Ray category decoding for sing-box matcher compilation.
//! Adapted from meow-rs' MIT-licensed `geosite_dat` reader. Paws owns this
//! conversion boundary; it does not require a fork of the runtime rule engine.
//! See LICENSES/meow-rs/LICENSE for the original license.
use std::ops::ControlFlow;

/// Protobuf wire-type tags we care about.
const WIRE_VARINT: u32 = 0;
const WIRE_LEN_DELIM: u32 = 2;
const WIRE_I64: u32 = 1;
const WIRE_I32: u32 = 5;

/// Field numbers in the V2Ray geosite schema (above).
const FIELD_GEOSITELIST_ENTRY: u32 = 1;
const FIELD_GEOSITE_COUNTRY_CODE: u32 = 1;
const FIELD_GEOSITE_DOMAIN: u32 = 2;
const FIELD_DOMAIN_TYPE: u32 = 1;
const FIELD_DOMAIN_VALUE: u32 = 2;
const FIELD_DOMAIN_ATTRIBUTE: u32 = 3;
const FIELD_ATTRIBUTE_KEY: u32 = 1;

/// `Domain.Type` enum values.
const DOMAIN_TYPE_PLAIN: u64 = 0;
const DOMAIN_TYPE_REGEX: u64 = 1;
const DOMAIN_TYPE_DOMAIN: u64 = 2;
const DOMAIN_TYPE_FULL: u64 = 3;

#[derive(Debug, thiserror::Error)]
pub(super) enum DatError {
    #[error("geosite.dat: visitor stopped before completing the requested category")]
    VisitorStopped,
    #[error("geosite.dat: requested category is missing or invalid")]
    MissingCategory,
    #[error("geosite.dat: unknown domain pattern type {0}")]
    UnknownDomainType(u64),
    #[error("geosite.dat: truncated at offset {0}")]
    Truncated(usize),
    #[error("geosite.dat: varint overflow at offset {0}")]
    VarintOverflow(usize),
    #[error("geosite.dat: invalid utf-8 in field at offset {0}")]
    InvalidUtf8(usize),
    #[error("geosite.dat: unknown wire type {1} at offset {0}")]
    UnknownWireType(usize, u32),
}

/// Minimal protobuf reader — only the wire-format primitives needed for the
/// geosite schema. Holds a byte slice + cursor; all reads advance the cursor.
struct PbReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> PbReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn read_varint(&mut self) -> Result<u64, DatError> {
        let start = self.pos;
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if self.pos >= self.buf.len() {
                return Err(DatError::Truncated(start));
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            if shift >= 64 || (shift == 63 && b > 1) {
                return Err(DatError::VarintOverflow(start));
            }
            result |= u64::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// Read a wire tag — returns `(field_number, wire_type)`.
    fn read_tag(&mut self) -> Result<(u32, u32), DatError> {
        let tag = self.read_varint()?;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u32;
        Ok((field, wire))
    }

    fn read_length_delimited(&mut self) -> Result<&'a [u8], DatError> {
        let start = self.pos;
        let len =
            usize::try_from(self.read_varint()?).map_err(|_| DatError::VarintOverflow(start))?;
        if self.remaining() < len {
            return Err(DatError::Truncated(start));
        }
        let bytes = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(bytes)
    }

    /// Skip a field whose tag was just consumed. Required when an unknown
    /// field is encountered (e.g. `Domain.attribute`, field 3 wire-type 2).
    fn skip_field(&mut self, wire: u32) -> Result<(), DatError> {
        let start = self.pos;
        match wire {
            WIRE_VARINT => {
                let _ = self.read_varint()?;
            }
            WIRE_LEN_DELIM => {
                let _ = self.read_length_delimited()?;
            }
            WIRE_I64 => {
                if self.remaining() < 8 {
                    return Err(DatError::Truncated(start));
                }
                self.pos += 8;
            }
            WIRE_I32 => {
                if self.remaining() < 4 {
                    return Err(DatError::Truncated(start));
                }
                self.pos += 4;
            }
            other => return Err(DatError::UnknownWireType(start, other)),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GeoSitePatternKind {
    Keyword,
    Regex,
    Suffix,
    Full,
}

pub(super) struct GeositeCategory<'a> {
    data: &'a [u8],
    category: &'a str,
}

impl<'a> GeositeCategory<'a> {
    pub fn new(data: &'a [u8], category: &'a str) -> Self {
        Self { data, category }
    }

    pub fn visit(
        &self,
        mut visit: impl FnMut(GeoSitePatternKind, &str) -> ControlFlow<()>,
    ) -> Result<usize, DatError> {
        let category = self.category.to_ascii_lowercase();
        let (base, attribute) = category
            .split_once('@')
            .map_or((category.as_str(), None), |(base, attribute)| {
                (base, Some(attribute))
            });
        if base.is_empty() || attribute.is_some_and(|a| a.is_empty() || a.contains('@')) {
            return Err(DatError::MissingCategory);
        }
        let mut reader = PbReader::new(self.data);
        let mut found = false;
        let mut visited = 0;
        while !reader.is_at_end() {
            let (field, wire) = reader.read_tag()?;
            if field != FIELD_GEOSITELIST_ENTRY || wire != WIRE_LEN_DELIM {
                reader.skip_field(wire)?;
                continue;
            }
            let bytes = reader.read_length_delimited()?;
            let mut entry = PbReader::new(bytes);
            let mut country = None;
            // Locate the country regardless of protobuf field order. Scan a
            // second time only for the selected category instead of retaining
            // one reference per domain in every category of the resource.
            while !entry.is_at_end() {
                let (field, wire) = entry.read_tag()?;
                if field == FIELD_GEOSITE_COUNTRY_CODE && wire == WIRE_LEN_DELIM {
                    country = Some(
                        std::str::from_utf8(entry.read_length_delimited()?)
                            .map_err(|_| DatError::InvalidUtf8(entry.pos))?,
                    );
                } else {
                    entry.skip_field(wire)?;
                }
            }
            if !country.is_some_and(|c| c.eq_ignore_ascii_case(base)) {
                continue;
            }
            if attribute.is_none() {
                found = true;
            }
            let mut entry = PbReader::new(bytes);
            while !entry.is_at_end() {
                let (field, wire) = entry.read_tag()?;
                if field != FIELD_GEOSITE_DOMAIN || wire != WIRE_LEN_DELIM {
                    entry.skip_field(wire)?;
                    continue;
                }
                let Some(domain) = parse_domain_entry(entry.read_length_delimited()?)? else {
                    continue;
                };
                if attribute
                    .is_some_and(|a| !domain.attrs.iter().any(|v| v.eq_ignore_ascii_case(a)))
                {
                    continue;
                }
                found = true;
                let kind = match domain.dom_type {
                    DOMAIN_TYPE_PLAIN => GeoSitePatternKind::Keyword,
                    DOMAIN_TYPE_REGEX => GeoSitePatternKind::Regex,
                    DOMAIN_TYPE_DOMAIN => GeoSitePatternKind::Suffix,
                    DOMAIN_TYPE_FULL => GeoSitePatternKind::Full,
                    other => return Err(DatError::UnknownDomainType(other)),
                };
                if visit(kind, &domain.value).is_break() {
                    return Err(DatError::VisitorStopped);
                }
                visited += 1;
            }
        }
        if !found {
            return Err(DatError::MissingCategory);
        }
        Ok(visited)
    }
}

struct ParsedDomainEntry {
    dom_type: u64,
    value: String,
    attrs: Vec<String>,
}

fn parse_domain_entry(data: &[u8]) -> Result<Option<ParsedDomainEntry>, DatError> {
    let mut r = PbReader::new(data);
    let mut dom_type: u64 = DOMAIN_TYPE_DOMAIN;
    let mut value: Option<String> = None;
    let mut attrs = Vec::new();
    let mut saw_type = false;

    while !r.is_at_end() {
        let (field, wire) = r.read_tag()?;
        match (field, wire) {
            (FIELD_DOMAIN_TYPE, WIRE_VARINT) => {
                dom_type = r.read_varint()?;
                saw_type = true;
            }
            (FIELD_DOMAIN_VALUE, WIRE_LEN_DELIM) => {
                let bytes = r.read_length_delimited()?;
                let s = std::str::from_utf8(bytes)
                    .map_err(|_| DatError::InvalidUtf8(r.pos))?
                    .to_ascii_lowercase();
                value = Some(s);
            }
            (FIELD_DOMAIN_ATTRIBUTE, WIRE_LEN_DELIM) => {
                let bytes = r.read_length_delimited()?;
                if let Some(key) = parse_attribute_key(bytes)? {
                    attrs.push(key);
                }
            }
            (_, w) => r.skip_field(w)?,
        }
    }

    if !saw_type {
        dom_type = DOMAIN_TYPE_PLAIN;
    }

    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }

    Ok(Some(ParsedDomainEntry {
        dom_type,
        value,
        attrs,
    }))
}

fn parse_attribute_key(data: &[u8]) -> Result<Option<String>, DatError> {
    let mut r = PbReader::new(data);
    let mut key = None;
    while !r.is_at_end() {
        let (field, wire) = r.read_tag()?;
        match (field, wire) {
            (FIELD_ATTRIBUTE_KEY, WIRE_LEN_DELIM) => {
                let bytes = r.read_length_delimited()?;
                let s = std::str::from_utf8(bytes)
                    .map_err(|_| DatError::InvalidUtf8(r.pos))?
                    .trim()
                    .to_ascii_lowercase();
                if !s.is_empty() {
                    key = Some(s);
                }
            }
            (_, w) => r.skip_field(w)?,
        }
    }
    Ok(key)
}
