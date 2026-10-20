//! ShadowQUIC JLS hello-random authentication (upstream `metacubex/jls-tls`
//! `jls.go`).
//!
//! Both peers authenticate through the `random` field of their hello
//! messages: a 16-byte seed sealed with AES-256-GCM under
//! `key = SHA-256(password ‖ authData)` / `nonce = SHA-256(username ‖
//! authData)`, where `authData` is the serialized hello with the random
//! field (and any PSK binders) zeroed. The sealed value is 32 bytes and
//! replaces the random outright — to the rest of TLS it is opaque, so the
//! handshake transcript and key schedule are unaffected.

use boring::symm::{decrypt_aead, encrypt_aead, Cipher};
use sha2::{Digest, Sha256};

use crate::restls::tls13::{HELLO_RANDOM_LEN, HELLO_RANDOM_OFFSET};
use crate::{Result, TransportError};

/// Plaintext seed sealed into the random.
const SEED_LEN: usize = 16;

/// `random` must not end in a TLS 1.2/1.1 downgrade canary or the HRR
/// magic tail — upstream `jlsHasForbiddenRandomSuffix` (JLS v3).
const DOWNGRADE_CANARY_TLS12: [u8; 8] = *b"DOWNGRD\x01";
const DOWNGRADE_CANARY_TLS11: [u8; 8] = *b"DOWNGRD\x00";
/// Last 8 bytes of `SHA-256("HelloRetryRequest")`.
const HRR_RANDOM_SUFFIX: [u8; 8] = [0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c];

fn auth_key(password: &str, auth_data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(password.as_bytes());
    h.update(auth_data);
    h.finalize().into()
}

fn auth_nonce(username: &str, auth_data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(username.as_bytes());
    h.update(auth_data);
    h.finalize().into()
}

fn has_forbidden_suffix(random: &[u8; HELLO_RANDOM_LEN]) -> bool {
    let suffix = &random[HELLO_RANDOM_LEN - 8..];
    suffix == DOWNGRADE_CANARY_TLS12
        || suffix == DOWNGRADE_CANARY_TLS11
        || suffix == HRR_RANDOM_SUFFIX
}

/// Seal a fresh seed into the 32-byte fake random. Regenerates while the
/// ciphertext ends in a forbidden suffix, mirroring upstream. Each retry
/// hits a suffix with probability ~3/2⁶⁴; the bound is insurance against
/// a broken RNG, not a reachable limit.
pub(crate) fn build_fake_random(
    username: &str,
    password: &str,
    auth_data: &[u8],
) -> Result<[u8; HELLO_RANDOM_LEN]> {
    let key = auth_key(password, auth_data);
    let nonce = auth_nonce(username, auth_data);
    for _ in 0..1024 {
        let seed: [u8; SEED_LEN] = rand::random();
        let mut tag = [0u8; 16];
        let ct = encrypt_aead(
            Cipher::aes_256_gcm(),
            &key,
            Some(&nonce),
            &[],
            &seed,
            &mut tag,
        )
        .map_err(|e| TransportError::Tls(format!("jls: fake random seal: {e}")))?;
        let mut random = [0u8; HELLO_RANDOM_LEN];
        random[..SEED_LEN].copy_from_slice(&ct);
        random[SEED_LEN..].copy_from_slice(&tag);
        if !has_forbidden_suffix(&random) {
            return Ok(random);
        }
    }
    Err(TransportError::Tls(
        "jls: fake random regen exceeded 1024 tries".into(),
    ))
}

/// Open a peer's fake random; `true` when it decrypts to a 16-byte seed
/// under the user's credentials (upstream `jlsCheckFakeRandom`).
pub(crate) fn check_fake_random(
    username: &str,
    password: &str,
    auth_data: &[u8],
    random: &[u8; HELLO_RANDOM_LEN],
) -> bool {
    let key = auth_key(password, auth_data);
    let nonce = auth_nonce(username, auth_data);
    let (ct, tag) = random.split_at(SEED_LEN);
    decrypt_aead(Cipher::aes_256_gcm(), &key, Some(&nonce), &[], ct, tag)
        .is_ok_and(|plain| plain.len() == SEED_LEN)
}

/// ServerHello `authData`: the raw wire handshake message with `random`
/// zeroed (upstream `jlsServerHelloAuthData` — `hello.original`).
pub(crate) fn server_hello_auth_data(server_hello_wire: &[u8]) -> Result<Vec<u8>> {
    if server_hello_wire.len() < HELLO_RANDOM_OFFSET + HELLO_RANDOM_LEN {
        return Err(TransportError::Tls("jls: server hello too short".into()));
    }
    let mut msg = server_hello_wire.to_vec();
    msg[HELLO_RANDOM_OFFSET..HELLO_RANDOM_OFFSET + HELLO_RANDOM_LEN].fill(0);
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH_DATA: &[u8] = b"auth-data";

    #[test]
    fn fake_random_round_trip() {
        let random = build_fake_random("alice", "s3cret", AUTH_DATA).unwrap();
        assert!(check_fake_random("alice", "s3cret", AUTH_DATA, &random));
    }

    #[test]
    fn fake_random_wrong_credentials_fail() {
        let random = build_fake_random("alice", "s3cret", AUTH_DATA).unwrap();
        assert!(!check_fake_random("alice", "wrong", AUTH_DATA, &random));
        assert!(!check_fake_random("mallory", "s3cret", AUTH_DATA, &random));
        assert!(!check_fake_random("alice", "s3cret", b"other", &random));
    }

    #[test]
    fn fake_random_tampered_fails() {
        let mut random = build_fake_random("alice", "s3cret", AUTH_DATA).unwrap();
        random[0] ^= 1;
        assert!(!check_fake_random("alice", "s3cret", AUTH_DATA, &random));
    }

    /// The forbidden-suffix predicate fires on both downgrade canaries and
    /// the HRR magic tail, and rejects nothing else.
    #[test]
    fn forbidden_suffix_detection() {
        let mut random = [0u8; HELLO_RANDOM_LEN];
        assert!(!has_forbidden_suffix(&random));
        random[HELLO_RANDOM_LEN - 8..].copy_from_slice(&DOWNGRADE_CANARY_TLS12);
        assert!(has_forbidden_suffix(&random));
        random[HELLO_RANDOM_LEN - 8..].copy_from_slice(&DOWNGRADE_CANARY_TLS11);
        assert!(has_forbidden_suffix(&random));
        random[HELLO_RANDOM_LEN - 8..].copy_from_slice(&HRR_RANDOM_SUFFIX);
        assert!(has_forbidden_suffix(&random));
        // A canary anywhere but the tail is legal.
        random[HELLO_RANDOM_LEN - 8..].copy_from_slice(&[0u8; 8]);
        random[..8].copy_from_slice(&DOWNGRADE_CANARY_TLS12);
        assert!(!has_forbidden_suffix(&random));
    }

    #[test]
    fn server_hello_auth_data_zeroes_random() {
        let mut wire = vec![0x02, 0x00, 0x00, 0x40, 0x03, 0x03];
        wire.extend_from_slice(&[0xaa; 32]);
        wire.extend_from_slice(&[0xbb; 10]);
        let data = server_hello_auth_data(&wire).unwrap();
        assert_eq!(&data[..6], &wire[..6]);
        assert_eq!(&data[6..38], &[0u8; 32]);
        assert_eq!(&data[38..], &wire[38..]);
    }
}
