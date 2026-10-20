//! restls tagged-record wire format — a port of `extractRestlsAppData`,
//! `write0x17AuthHeader`, `restlsAuthHeaderHash` and `maskServerAuth` from
//! `restls-client-go` (`conn.go`, `restls_server.go`).
//!
//! Post-handshake application data travels in fake `application_data`
//! records of this shape (TLS 1.2-GCM mode adds an 8-byte nonce prefix):
//!
//! ```text
//! [0x17][03][03][u16 len] [nonce8?] [mac8] [len2 ^ mask][cmd2 ^ mask] data pad
//! ```
//!
//! * `mac` = blake3(secret || server_random || dir || u64be(ctr)
//!   [| client_finished_record] || header || rest)[..8]
//!   — `header` is the record header (5 bytes, or 13 with the nonce), `rest`
//!   is everything after the MAC field (masked len/cmd + data + padding).
//! * `mask` = blake3(secret || server_random || dir || u64be(ctr)
//!   || data[..min(32)])[..4]  — XOR'd over the len/cmd field.
//! * `dir` = `"client-to-server"` / `"server-to-client"`.

use crate::restls::script::Command;
use crate::{Result, TransportError};

/// `restlsHandshakeMACLength` — bytes of the session-id auth tag.
pub(crate) const HANDSHAKE_MAC_LEN: usize = 16;
/// `restlsAppDataMACLength`.
pub(crate) const APP_MAC_LEN: usize = 8;
/// `restlsMaskLength` — masked field width (u16 data len + u16 command).
pub(crate) const MASK_LEN: usize = 4;
/// `restlsAppDataAuthHeaderLength` — MAC + masked field.
pub(crate) const AUTH_HEADER_LEN: usize = APP_MAC_LEN + MASK_LEN;
/// `restlsAppDataOffset` — data offset inside the record payload.
pub(crate) const APP_DATA_OFFSET: usize = AUTH_HEADER_LEN;
/// `restlsAppDataLenOffset` — masked len/cmd field offset.
pub(crate) const LEN_OFFSET: usize = APP_MAC_LEN;
/// `maxCiphertext` bound used by the record reader.
pub(crate) const MAX_RECORD: usize = 18 * 1024;

/// TLS plaintext fragment limit (RFC 8446 §5.1 / RFC 5246 §6.2.1).
pub(crate) const MAX_PLAINTEXT: usize = 16 * 1024;
/// Record header size.
pub(crate) const RECORD_HDR: usize = 5;
/// Extra nonce prefix carried by records in TLS 1.2-GCM mode.
pub(crate) const GCM_NONCE_LEN: usize = 8;

pub(crate) const TLS_RECORD_HANDSHAKE: u8 = 22;
pub(crate) const TLS_RECORD_APPLICATION_DATA: u8 = 23;
pub(crate) const TLS_RECORD_ALERT: u8 = 21;
pub(crate) const TLS_RECORD_CHANGE_CIPHER_SPEC: u8 = 20;

/// Upstream `RestlsHmac` — blake3 keyed mode over the 32-byte restls secret.
pub(crate) fn restls_hasher(secret: &[u8; 32]) -> blake3::Hasher {
    blake3::Hasher::new_keyed(secret)
}

/// Derive the 32-byte restls secret — upstream
/// `blake3.DeriveKey(key, "restls-traffic-key", password)`.
pub(crate) fn derive_secret(password: &[u8]) -> [u8; 32] {
    blake3::derive_key("restls-traffic-key", password)
}

/// Upstream `restlsAuthHeaderHash` — a keyed hash pre-seeded with
/// `server_random`, the direction label and the record counter.
pub(crate) fn auth_header_hash(
    secret: &[u8; 32],
    server_random: &[u8; 32],
    to_client: bool,
    counter: u64,
) -> blake3::Hasher {
    let mut h = restls_hasher(secret);
    h.update(server_random);
    h.update(if to_client {
        b"server-to-client".as_slice()
    } else {
        b"client-to-server".as_slice()
    });
    h.update(&counter.to_be_bytes());
    h
}

/// Upstream `xorWithMac`.
fn xor_with_mac(buf: &mut [u8], mac: &[u8]) {
    for (b, m) in buf.iter_mut().zip(mac.iter()) {
        *b ^= *m;
    }
}

/// Upstream `maskServerAuth` — XOR the server-random mask into the cover's
/// first post-CCS record so the client can detect restls support. For a TLS
/// 1.2-GCM cover whose first record carries a zero explicit nonce the mask
/// starts after the nonce (`parrotGCM`).
pub(crate) fn server_auth_mask(secret: &[u8; 32], server_random: &[u8; 32]) -> [u8; 16] {
    let mut h = restls_hasher(secret);
    h.update(server_random);
    let mut out = [0u8; HANDSHAKE_MAC_LEN];
    out.copy_from_slice(&h.finalize().as_bytes()[..HANDSHAKE_MAC_LEN]);
    out
}

/// Client-side: strip the auth mask from the first post-CCS cover record.
/// Returns the unmasked record bytes. `tls12_gcm` selects the nonce-aware
/// offset and reports whether the server disabled the counter check (nonce
/// != 0 at that position — upstream `restls12GCMServerDisableCtr`).
pub(crate) fn unmask_server_auth(
    record: &[u8],
    secret: &[u8; 32],
    server_random: &[u8; 32],
    tls12_gcm: bool,
) -> (Vec<u8>, bool) {
    let mask = server_auth_mask(secret, server_random);
    let mut out = record.to_vec();
    if out.len() < RECORD_HDR {
        return (out, false);
    }
    let gcm_nonce_zero = tls12_gcm
        && out.len() >= RECORD_HDR + GCM_NONCE_LEN
        && out[RECORD_HDR..RECORD_HDR + GCM_NONCE_LEN] == [0u8; GCM_NONCE_LEN];
    let offset = if gcm_nonce_zero {
        RECORD_HDR + GCM_NONCE_LEN
    } else {
        RECORD_HDR
    };
    xor_with_mac(&mut out[offset..], &mask);
    (out, !gcm_nonce_zero && tls12_gcm)
}

/// Inputs for one tagged record.
pub(crate) struct TaggedRecord<'a> {
    pub(crate) counter: u64,
    pub(crate) data: &'a [u8],
    /// Record payload target — `data_len + padding_len + auth header (+nonce)`.
    pub(crate) payload_len: usize,
    pub(crate) data_len: usize,
    pub(crate) padding_len: usize,
    pub(crate) command: Command,
    /// Sealed client-Finished record mixed into the first record's MAC
    /// (TLS 1.3 and resumed TLS 1.2 only).
    pub(crate) client_finished: Option<&'a [u8]>,
    /// TLS 1.2-GCM: emit the `counter + 1` nonce prefix.
    pub(crate) tls12_gcm: bool,
}

/// Build one tagged record — upstream `write0x17AuthHeader` +
/// `writeRestlsApplicationRecord`'s framing. `padding` is filled with random
/// bytes (upstream `writePadding` uses `rand.Read` for the padding).
pub(crate) fn build_tagged_record(
    secret: &[u8; 32],
    server_random: &[u8; 32],
    spec: &TaggedRecord<'_>,
    rng_pad: impl Fn(&mut [u8]),
) -> Vec<u8> {
    let nonce_len = if spec.tls12_gcm { GCM_NONCE_LEN } else { 0 };
    let header_len = RECORD_HDR + nonce_len;
    // `payload_len` is the TLS record payload — nonce + auth + data + pad —
    // so the wire record is `RECORD_HDR + payload_len`.
    let mut out = Vec::with_capacity(RECORD_HDR + spec.payload_len);
    out.push(TLS_RECORD_APPLICATION_DATA);
    out.extend_from_slice(&[0x03, 0x03]);
    out.extend_from_slice(&(spec.payload_len as u16).to_be_bytes());
    if spec.tls12_gcm {
        out.extend_from_slice(&(spec.counter + 1).to_be_bytes());
    }
    // Reserve: MAC | masked len/cmd | data | padding.
    out.resize(header_len + AUTH_HEADER_LEN, 0);
    out.extend_from_slice(&spec.data[..spec.data_len]);
    let pad_start = out.len();
    out.resize(pad_start + spec.padding_len, 0);
    rng_pad(&mut out[pad_start..]);
    debug_assert_eq!(out.len(), RECORD_HDR + spec.payload_len);

    // len || cmd, XOR-masked with the stream mask.
    let data_region = &out[header_len + APP_DATA_OFFSET..];
    let sample = &data_region[..data_region.len().min(32)];
    let mut hmask = auth_header_hash(secret, server_random, false, spec.counter);
    hmask.update(sample);
    let mask = hmask.finalize();
    let field = &mut out[header_len + LEN_OFFSET..header_len + LEN_OFFSET + MASK_LEN];
    field[..2].copy_from_slice(&(spec.data_len as u16).to_be_bytes());
    field[2..4].copy_from_slice(&spec.command.to_bytes());
    xor_with_mac(field, &mask.as_bytes()[..MASK_LEN]);

    // MAC over (client_finished ||) header || masked-field..end.
    let mut hmac = auth_header_hash(secret, server_random, false, spec.counter);
    if let Some(fin) = spec.client_finished {
        hmac.update(fin);
    }
    hmac.update(&out[..header_len]);
    hmac.update(&out[header_len + LEN_OFFSET..]);
    out[header_len..header_len + APP_MAC_LEN]
        .copy_from_slice(&hmac.finalize().as_bytes()[..APP_MAC_LEN]);
    out
}

/// One decoded inbound tagged record.
pub(crate) struct InboundRecord {
    /// Application bytes carried by the record (borrowed from `payload`).
    pub(crate) data_len: usize,
    pub(crate) command: Command,
    /// Payload view with the auth header stripped — `data[..data_len]` is the
    /// application data; the slice is the caller's buffer so the conn layer
    /// can hand it to the read path without copying.
    pub(crate) payload_start: usize,
}

/// Verify and decode one inbound tagged record — upstream
/// `extractRestlsAppData`. On success returns the data slice bounds inside
/// `record`; the caller owns `record` so we return offsets, not a slice.
///
/// Returns `Err` on any MAC/format failure — the conn layer then retries the
/// record through the real TLS cipher (cover records such as session tickets
/// are relayed verbatim and must not break the stream).
pub(crate) fn extract_tagged_record(
    record: &[u8],
    secret: &[u8; 32],
    server_random: &[u8; 32],
    counter: u64,
    tls12_gcm: bool,
    gcm_ctr_disabled: bool,
) -> Result<InboundRecord> {
    extract_tagged_record_dir(
        record,
        secret,
        server_random,
        counter,
        true,
        tls12_gcm,
        gcm_ctr_disabled,
    )
}

/// Direction-aware variant — tests use it to verify client-built records
/// (`to_client=false` is test-only; the function itself is on the live path).
pub(crate) fn extract_tagged_record_dir(
    record: &[u8],
    secret: &[u8; 32],
    server_random: &[u8; 32],
    counter: u64,
    to_client: bool,
    tls12_gcm: bool,
    gcm_ctr_disabled: bool,
) -> Result<InboundRecord> {
    if record.len() < RECORD_HDR || record[0] != TLS_RECORD_APPLICATION_DATA {
        return Err(TransportError::Tls("restls: not an appdata record".into()));
    }
    if record.len() < RECORD_HDR + AUTH_HEADER_LEN {
        return Err(TransportError::Tls("restls: short tagged record".into()));
    }
    let mut header_len = RECORD_HDR;
    if tls12_gcm && !gcm_ctr_disabled {
        if record.len() < RECORD_HDR + GCM_NONCE_LEN + AUTH_HEADER_LEN {
            return Err(TransportError::Tls(
                "restls: short GCM tagged record".into(),
            ));
        }
        let nonce = u64::from_be_bytes(
            record[RECORD_HDR..RECORD_HDR + GCM_NONCE_LEN]
                .try_into()
                .unwrap(),
        );
        if nonce != counter + 1 {
            return Err(TransportError::Tls("restls: bad GCM nonce".into()));
        }
        header_len += GCM_NONCE_LEN;
    }
    let payload = &record[header_len..];

    // MAC check: blake3(secret||rand||dir||ctr || header || masked..end)[:8].
    let mut hmac = auth_header_hash(secret, server_random, to_client, counter);
    hmac.update(&record[..header_len]);
    hmac.update(&payload[LEN_OFFSET..]);
    let expect = hmac.finalize();
    use subtle::ConstantTimeEq;
    if !bool::from(expect.as_bytes()[..APP_MAC_LEN].ct_eq(&payload[..APP_MAC_LEN])) {
        return Err(TransportError::Tls("restls: bad record MAC".into()));
    }

    // Unmask len||cmd; the mask hashes the first ≤32 bytes of data+padding.
    let mut hmask = auth_header_hash(secret, server_random, to_client, counter);
    let data_region = &payload[APP_DATA_OFFSET..];
    hmask.update(&data_region[..data_region.len().min(32)]);
    let mask = hmask.finalize();
    let mut field = [0u8; MASK_LEN];
    field.copy_from_slice(&payload[LEN_OFFSET..LEN_OFFSET + MASK_LEN]);
    xor_with_mac(&mut field, &mask.as_bytes()[..MASK_LEN]);
    let data_len = u16::from_be_bytes([field[0], field[1]]) as usize;
    let command = super::script::parse_command(&field[2..])?;
    if data_len > payload.len() - APP_DATA_OFFSET {
        return Err(TransportError::Tls(
            "restls: data len exceeds record".into(),
        ));
    }
    Ok(InboundRecord {
        data_len,
        command,
        payload_start: header_len + APP_DATA_OFFSET,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restls::script::Command;

    fn pad_zero(buf: &mut [u8]) {
        buf.fill(0);
    }

    fn spec<'a>(
        data: &'a [u8],
        payload_len: usize,
        data_len: usize,
        padding: usize,
    ) -> TaggedRecord<'a> {
        TaggedRecord {
            counter: 0,
            data,
            payload_len,
            data_len,
            padding_len: padding,
            command: Command::Noop,
            client_finished: None,
            tls12_gcm: false,
        }
    }

    #[test]
    fn record_round_trip() {
        let secret = derive_secret(b"hunter2");
        let sr = [0xabu8; 32];
        let data = b"hello restls";
        let s = spec(data, 100, data.len(), 100 - AUTH_HEADER_LEN - data.len());
        let record = build_tagged_record(&secret, &sr, &s, pad_zero);
        assert_eq!(record.len(), 5 + 100);
        let decoded =
            extract_tagged_record_dir(&record, &secret, &sr, 0, false, false, false).unwrap();
        assert_eq!(decoded.data_len, data.len());
        assert_eq!(
            &record[decoded.payload_start..decoded.payload_start + decoded.data_len],
            data
        );
        assert_eq!(decoded.command, Command::Noop);
    }

    #[test]
    fn record_tamper_rejected() {
        let secret = derive_secret(b"hunter2");
        let sr = [1u8; 32];
        let data = b"payload";
        let s = spec(data, 60, data.len(), 60 - AUTH_HEADER_LEN - data.len());
        let mut record = build_tagged_record(&secret, &sr, &s, pad_zero);
        record[20] ^= 1;
        assert!(extract_tagged_record_dir(&record, &secret, &sr, 0, false, false, false).is_err());
        // Wrong counter also rejected.
        let record = build_tagged_record(&secret, &sr, &s, pad_zero);
        assert!(extract_tagged_record_dir(&record, &secret, &sr, 1, false, false, false).is_err());
    }

    /// Corrupting only the 8-byte MAC field must reject — the masked
    /// len/cmd field stays intact, so nothing else could catch the
    /// tamper. (Pins the ct_eq gate itself, not just downstream bounds.)
    #[test]
    fn record_bad_mac_rejected() {
        let secret = derive_secret(b"hunter2");
        let sr = [1u8; 32];
        let data = b"payload";
        let s = spec(data, 60, data.len(), 60 - AUTH_HEADER_LEN - data.len());
        let mut record = build_tagged_record(&secret, &sr, &s, pad_zero);
        for i in 0..APP_MAC_LEN {
            let mut r = record.clone();
            r[RECORD_HDR + i] ^= 1;
            assert!(
                extract_tagged_record_dir(&r, &secret, &sr, 0, false, false, false).is_err(),
                "MAC byte {i} corrupted"
            );
        }
        // And a structurally valid record with a zeroed MAC.
        record[RECORD_HDR..RECORD_HDR + APP_MAC_LEN].fill(0);
        assert!(extract_tagged_record_dir(&record, &secret, &sr, 0, false, false, false).is_err());
    }

    #[test]
    fn command_embedded_in_record() {
        let secret = derive_secret(b"x");
        let sr = [2u8; 32];
        let mut s = spec(&[], 40, 0, 40 - AUTH_HEADER_LEN);
        s.command = Command::Respond(2);
        let record = build_tagged_record(&secret, &sr, &s, pad_zero);
        let decoded =
            extract_tagged_record_dir(&record, &secret, &sr, 0, false, false, false).unwrap();
        assert_eq!(decoded.command, Command::Respond(2));
        assert_eq!(decoded.data_len, 0);
    }

    #[test]
    fn client_finished_in_mac() {
        let secret = derive_secret(b"p");
        let sr = [3u8; 32];
        let fin = b"sealed-finished-record";
        let data = b"first";
        let mut s = spec(data, 60, data.len(), 60 - AUTH_HEADER_LEN - data.len());
        s.client_finished = Some(fin);
        let record = build_tagged_record(&secret, &sr, &s, pad_zero);
        // Extraction does not take client_finished — the *server* knows it
        // from the relayed handshake. A record built with it must NOT verify
        // against a context lacking it; here we verify the MAC lands
        // differently than the no-Fin variant.
        let s2 = spec(data, 60, data.len(), 60 - AUTH_HEADER_LEN - data.len());
        let record2 = build_tagged_record(&secret, &sr, &s2, pad_zero);
        assert_ne!(&record[5..13], &record2[5..13]);
    }

    #[test]
    fn gcm_nonce_prefix() {
        let secret = derive_secret(b"p");
        let sr = [4u8; 32];
        let data = b"gcm-mode";
        // payload_len covers nonce + auth + data + pad in GCM mode.
        let payload = data.len() + AUTH_HEADER_LEN + GCM_NONCE_LEN + 8;
        let mut s = spec(data, payload, data.len(), 8);
        s.tls12_gcm = true;
        let record = build_tagged_record(&secret, &sr, &s, pad_zero);
        assert_eq!(&record[5..13], &1u64.to_be_bytes()); // nonce = counter+1
        let decoded =
            extract_tagged_record_dir(&record, &secret, &sr, 0, false, true, false).unwrap();
        assert_eq!(
            &record[decoded.payload_start..decoded.payload_start + decoded.data_len],
            data
        );
        // A mismatched nonce is rejected when the counter check is on —
        // and also under `disable_ctr` since the nonce bytes are inside the
        // MAC'd header.
        let mut corrupt = record.clone();
        corrupt[5..13].copy_from_slice(&9u64.to_be_bytes());
        assert!(extract_tagged_record_dir(&corrupt, &secret, &sr, 0, false, true, false).is_err());
        // Wrong counter → different MAC even with a well-formed nonce.
        assert!(extract_tagged_record_dir(&record, &secret, &sr, 9, false, true, true).is_err());
    }

    #[test]
    fn unmask_round_trip() {
        let secret = derive_secret(b"pw");
        let sr = [5u8; 32];
        // Emulate maskServerAuth on a fake record.
        let mut record = b"\x17\x03\x03\x00\x20".to_vec();
        record.extend_from_slice(&[0xEE; 32]);
        let mask = server_auth_mask(&secret, &sr);
        for i in 0..16 {
            record[5 + i] ^= mask[i];
        }
        let (unmasked, disable) = unmask_server_auth(&record, &secret, &sr, false);
        assert!(!disable);
        assert_eq!(&unmasked[5..21], &[0xEE; 16]);
        assert_eq!(&unmasked[21..], &[0xEE; 16]);
    }
}
