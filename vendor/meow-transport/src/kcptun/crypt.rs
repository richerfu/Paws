//! kcptun packet encryption — a wire-faithful port of kcp-go's `crypt.go`
//! (metacubex fork, used by mihomo's `transport/kcptun`).
//!
//! Two envelope layouts share the wire:
//!
//! * CFB ciphers (`aes*`, `tea`, `xtea`, `blowfish`, `cast5`, `3des`,
//!   `twofish`, `none`, `xor`, `salsa20`):
//!   `nonce(16B random) ‖ crc32le(payload) ‖ payload`, the whole thing fed
//!   through the cipher transform. `none` leaves the envelope in plaintext,
//!   `xor` XORs it with a PBKDF2-expanded table, `salsa20` leaves the first
//!   8 nonce bytes in the clear (they are the stream nonce) and XORs the rest.
//! * AEAD (`aes-128-gcm`): `nonce(12B) ‖ AES-128-GCM.Seal(payload)` — the
//!   GCM tag replaces the CRC32, so the envelope is nonce-only.
//!
//! `null` selects no crypt layer at all: the KCP datagram goes out raw.
//!
//! The block-cipher transform is textbook full-block CFB seeded with the
//! fixed `INITIAL_VECTOR` — kcp-go hand-rolls the loop for performance, but
//! `tbl = E(IV)` followed by `ct = pt ^ tbl; tbl = E(ct)` is exactly CFB
//! with that IV, so we keep the same tiny loop over `BlockCipherEncrypt`.

use std::io;

use cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, BlockSizeUser, KeyInit};

/// Fixed IV from kcp-go's `crypt.go` (`initialVector`). Only the first
/// `block_size` bytes are used per cipher.
const INITIAL_VECTOR: [u8; 16] = [
    167, 115, 79, 156, 18, 172, 27, 1, 164, 21, 242, 193, 252, 120, 230, 107,
];

/// Salt for the `xor` crypt's PBKDF2 key expansion (`saltxor` upstream).
const SALT_XOR: &[u8] = b"sH3CIVoF#rWLtJo6";

/// Salt for the session key derivation (`SALT` in upstream `common.go`).
pub(crate) const KDF_SALT: &[u8] = b"kcp-go";

/// PBKDF2 rounds for both derivations.
const KDF_ROUNDS: u32 = 4096;

/// `nonceSize` upstream: 16-byte random nonce prefix per CFB packet.
pub(crate) const NONCE_SIZE: usize = 16;

/// CRC32 field size inside the CFB envelope.
pub(crate) const CRC_SIZE: usize = 4;

/// `cryptHeaderSize` upstream: nonce + crc32.
pub(crate) const CRYPT_HEADER_SIZE: usize = NONCE_SIZE + CRC_SIZE;

/// `aes-128-gcm` nonce size (standard 96-bit GCM nonce).
const GCM_NONCE_SIZE: usize = 12;

/// `mtuLimit` upstream: caps the XOR key table and FEC shard buffers.
pub(crate) const MTU_LIMIT: usize = 1500;

/// Derive the session key: `pbkdf2(key, "kcp-go", 4096, 32, sha1)`.
fn session_key(key: &[u8]) -> [u8; 32] {
    let mut pass = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(key, KDF_SALT, KDF_ROUNDS, &mut pass);
    pass
}

/// A cipher-0.5 block cipher erasure — enough to drive the CFB loop. CFB
/// decryption never calls the block *decrypt* direction (the keystream is
/// `E(ct)` either way), so only `encrypt_block` is needed.
pub(crate) trait BlockCipher: Send + Sync {
    fn block_size(&self) -> usize;
    fn encrypt_block(&self, block: &mut [u8]);
}

struct Blk<C>(C);

impl<C> BlockCipher for Blk<C>
where
    C: BlockCipherEncrypt + BlockCipherDecrypt + BlockSizeUser + Send + Sync,
{
    fn block_size(&self) -> usize {
        C::block_size()
    }
    fn encrypt_block(&self, block: &mut [u8]) {
        self.0.encrypt_block(block.try_into().expect("block size"));
    }
}

/// Full-block CFB, ciphertext feedback (kcp-go's `encrypt`/`decrypt` —
/// the unrolled loops there are equivalent to this straight loop).
/// Stack `tbl`/`ct` only: every cipher in the suite has `bs <= 16`, so the
/// per-packet hot path allocates nothing.
fn cfb(c: &dyn BlockCipher, buf: &mut [u8], decrypt: bool) {
    let bs = c.block_size();
    debug_assert!(bs <= INITIAL_VECTOR.len());
    let mut tbl = [0u8; 16];
    tbl[..bs].copy_from_slice(&INITIAL_VECTOR[..bs]);
    c.encrypt_block(&mut tbl[..bs]);
    let mut off = 0;
    while off + bs <= buf.len() {
        let block = &mut buf[off..off + bs];
        if decrypt {
            // Save ciphertext before whitening — it feeds the next keystream.
            let mut ct = [0u8; 16];
            ct[..bs].copy_from_slice(block);
            for (b, t) in block.iter_mut().zip(&tbl[..bs]) {
                *b ^= *t;
            }
            c.encrypt_block(&mut ct[..bs]);
            tbl[..bs].copy_from_slice(&ct[..bs]);
        } else {
            for (b, t) in block.iter_mut().zip(&tbl[..bs]) {
                *b ^= *t;
            }
            tbl[..bs].copy_from_slice(block);
            c.encrypt_block(&mut tbl[..bs]);
        }
        off += bs;
    }
    // Partial tail block: whitening only, no feedback update needed.
    for (b, t) in buf[off..].iter_mut().zip(&tbl[..bs]) {
        *b ^= *t;
    }
}

/// TEA with 16 rounds — the parameterisation kcp-go uses
/// (`tea.NewCipherWithRounds(key, 16)`); RustCrypto ships no TEA crate,
/// so the eight lines of Feistel live here instead of pulling a crate.
struct Tea16 {
    k: [u32; 4],
}

impl Tea16 {
    const DELTA: u32 = 0x9E37_79B9;
    /// Upstream `tea.NewCipherWithRounds(key, 16)` — x/crypto counts
    /// *half*-rounds and loops `rounds/2` pairs, so the wire count is 8
    /// v0+v1 pairs, not 16.
    const PAIRS: u32 = 8;

    fn new(key: &[u8; 16]) -> Self {
        let k: Vec<u32> = key
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_be_bytes)
            .collect();
        Self {
            k: [k[0], k[1], k[2], k[3]],
        }
    }

    fn encrypt(&self, block: &mut [u8; 8]) {
        let (mut v0, mut v1) = (
            u32::from_be_bytes(block[..4].try_into().unwrap()),
            u32::from_be_bytes(block[4..].try_into().unwrap()),
        );
        let [k0, k1, k2, k3] = self.k;
        let mut sum = 0u32;
        for _ in 0..Self::PAIRS {
            sum = sum.wrapping_add(Self::DELTA);
            v0 = v0.wrapping_add(
                ((v1 << 4).wrapping_add(k0)) ^ v1.wrapping_add(sum) ^ ((v1 >> 5).wrapping_add(k1)),
            );
            v1 = v1.wrapping_add(
                ((v0 << 4).wrapping_add(k2)) ^ v0.wrapping_add(sum) ^ ((v0 >> 5).wrapping_add(k3)),
            );
        }
        block[..4].copy_from_slice(&v0.to_be_bytes());
        block[4..].copy_from_slice(&v1.to_be_bytes());
    }
}

impl BlockCipher for Tea16 {
    fn block_size(&self) -> usize {
        8
    }
    fn encrypt_block(&self, block: &mut [u8]) {
        self.encrypt(block.try_into().unwrap());
    }
}

/// XTEA — 64 half-rounds (32 pairs), x/crypto's wire form. The RustCrypto
/// `xtea` crate reads keys and blocks *little*-endian, which is a different
/// cipher on the wire; kcp-go goes through `xtea.NewCipher`, which is
/// big-endian throughout, so like TEA the eleven lines live here.
struct Xtea {
    k: [u32; 4],
}

impl Xtea {
    const DELTA: u32 = 0x9E37_79B9;
    /// x/crypto `numRounds = 64` half-rounds = 32 v0+v1 pairs.
    const PAIRS: u32 = 32;

    fn new(key: &[u8; 16]) -> Self {
        let k: Vec<u32> = key
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_be_bytes)
            .collect();
        Self {
            k: [k[0], k[1], k[2], k[3]],
        }
    }

    /// `encryptBlock` — Go precalculates `sum + k[…]` into `table[i]`;
    /// expanding it inline keeps the same arithmetic without the table.
    fn encrypt(&self, block: &mut [u8; 8]) {
        let (mut v0, mut v1) = (
            u32::from_be_bytes(block[..4].try_into().unwrap()),
            u32::from_be_bytes(block[4..].try_into().unwrap()),
        );
        let k = self.k;
        let mut sum = 0u32;
        for _ in 0..Self::PAIRS {
            v0 = v0.wrapping_add(
                ((v1 << 4) ^ (v1 >> 5)).wrapping_add(v1) ^ sum.wrapping_add(k[(sum & 3) as usize]),
            );
            sum = sum.wrapping_add(Self::DELTA);
            v1 = v1.wrapping_add(
                ((v0 << 4) ^ (v0 >> 5)).wrapping_add(v0)
                    ^ sum.wrapping_add(k[((sum >> 11) & 3) as usize]),
            );
        }
        block[..4].copy_from_slice(&v0.to_be_bytes());
        block[4..].copy_from_slice(&v1.to_be_bytes());
    }
}

impl BlockCipher for Xtea {
    fn block_size(&self) -> usize {
        8
    }
    fn encrypt_block(&self, block: &mut [u8]) {
        self.encrypt(block.try_into().unwrap());
    }
}

/// Packet crypt transform — kcp-go's `BlockCrypt` plus the envelope duties
/// (`nonce ‖ crc32 ‖ payload`) that upstream keeps in `sess.go`.
pub(crate) enum Crypt {
    /// `crypt: null` — raw KCP datagrams, no envelope, no integrity tag.
    Null,
    /// `crypt: none` — envelope but the transform is a copy.
    None,
    /// `crypt: xor` — whole envelope XOR'd with the PBKDF2 key table.
    Xor(Vec<u8>),
    /// `crypt: salsa20` — bytes 8.. XOR'd with Salsa20 keyed by bytes 0..8.
    Salsa20(Box<cipher::Key<salsa20::Salsa20>>),
    /// CFB block ciphers — `aes`/`aes-256`, `aes-128`, `aes-192`, `tea`,
    /// `xtea`, `blowfish`, `cast5`, `3des`, `twofish`.
    Cfb(Box<dyn BlockCipher>),
    /// `crypt: aes-128-gcm` — `nonce(12) ‖ AES-128-GCM.Seal(payload)`.
    AesGcm(Box<aes_gcm11::Aes128Gcm>),
}

impl Crypt {
    /// `Config.NewBlock` — crypt name → transform. `pass` slicing is done
    /// per name exactly as upstream; unknown names map to AES-256-CFB.
    /// `sm4` is the one *known* upstream crypt we cannot honor — erroring
    /// beats silently dialing a session that can never authenticate.
    pub(crate) fn new(name: &str, key: &[u8]) -> io::Result<Self> {
        let pass = session_key(key);
        Ok(match name {
            "null" => Self::Null,
            "tea" => Self::Cfb(Box::new(Tea16::new(pass[..16].try_into().unwrap()))),
            "xor" => {
                // Upstream `NewSimpleXORBlockCrypt(pass)` re-derives the
                // table from the *session* key, not the raw key — a second
                // PBKDF2 with the `sH3CIVoF#rWLtJo6` salt.
                let mut tbl = vec![0u8; MTU_LIMIT];
                pbkdf2::pbkdf2_hmac::<sha1::Sha1>(&pass, SALT_XOR, 32, &mut tbl);
                Self::Xor(tbl)
            }
            "none" => Self::None,
            "aes-128" => Self::Cfb(Box::new(Blk(
                aes::Aes128::new_from_slice(&pass[..16]).expect("aes-128 key")
            ))),
            "aes-192" => Self::Cfb(Box::new(Blk(
                aes::Aes192::new_from_slice(&pass[..24]).expect("aes-192 key")
            ))),
            "blowfish" => Self::Cfb(Box::new(Blk(
                blowfish::Blowfish::<byteorder::BigEndian>::new_from_slice(&pass)
                    .expect("blowfish accepts 32B keys"),
            ))),
            "twofish" => Self::Cfb(Box::new(Blk(
                twofish::Twofish::new_from_slice(&pass).expect("twofish accepts 32B keys")
            ))),
            "cast5" => Self::Cfb(Box::new(Blk(
                cast5::Cast5::new_from_slice(&pass[..16]).expect("cast5 accepts 16B keys")
            ))),
            "3des" => Self::Cfb(Box::new(Blk(
                des::TdesEde3::new_from_slice(&pass[..24]).expect("3des accepts 24B keys")
            ))),
            "xtea" => Self::Cfb(Box::new(Xtea::new(pass[..16].try_into().unwrap()))),
            "salsa20" => Self::Salsa20(Box::new(Array::from(pass))),
            "aes-128-gcm" => Self::AesGcm(Box::new(
                aes_gcm11::Aes128Gcm::new_from_slice(&pass[..16]).expect("aes-128-gcm key"),
            )),
            // `sm4` is a real upstream crypt (tjfoc/gmsm) with no stable
            // Rust crate — refuse loudly rather than negotiate AES-256
            // with a server that expects SM4.
            "sm4" => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "kcptun: crypt=sm4 is not supported by this build",
                ));
            }
            // `aes`, `aes-256` and every unknown name land here — upstream
            // maps `default:` to AES-256-CFB rather than erroring.
            _ => Self::Cfb(Box::new(Blk(
                aes::Aes256::new_from_slice(&pass).expect("aes-256 key")
            ))),
        })
    }

    /// Bytes reserved at the head of every outgoing packet — upstream's
    /// `sess.headerSize` crypt component (16 nonce + 4 crc, or the 12-byte
    /// GCM nonce, or 0 for `null`).
    pub(crate) fn header_size(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::AesGcm(_) => GCM_NONCE_SIZE,
            _ => CRYPT_HEADER_SIZE,
        }
    }

    /// AEAD tag bytes appended by `seal` — upstream `aead.Overhead()`. The
    /// tag rides the wire packet but is not part of the `header_size()`
    /// scratch prefix, so MTU math must subtract it separately.
    pub(crate) fn aead_overhead(&self) -> usize {
        match self {
            Self::AesGcm(_) => 16,
            _ => 0,
        }
    }

    /// Encrypt one datagram in place. `buf` must start with `header_size()`
    /// scratch bytes followed by the payload; the head is filled with the
    /// nonce (+crc32 for the CFB family), then the transform is applied.
    /// AEAD output is `payload.len() + overhead` longer than the input.
    pub(crate) fn seal(&self, buf: &mut Vec<u8>) {
        match self {
            Self::Null => {}
            Self::AesGcm(aead) => {
                use aes_gcm11::aead::{Aead, Payload};
                use rand::RngCore;

                debug_assert!(buf.len() >= GCM_NONCE_SIZE);
                let mut nonce = [0u8; GCM_NONCE_SIZE];
                rand::rng().fill_bytes(&mut nonce);
                let ct = aead
                    .encrypt(
                        (&nonce[..]).try_into().expect("96-bit gcm nonce"),
                        Payload {
                            msg: &buf[GCM_NONCE_SIZE..],
                            aad: &[],
                        },
                    )
                    .expect("gcm seal is infallible");
                buf.truncate(GCM_NONCE_SIZE);
                buf[..GCM_NONCE_SIZE].copy_from_slice(&nonce);
                buf.extend_from_slice(&ct);
            }
            _ => {
                use rand::RngCore;
                rand::rng().fill_bytes(&mut buf[..NONCE_SIZE]);
                let sum = crc32fast::hash(&buf[CRYPT_HEADER_SIZE..]);
                buf[NONCE_SIZE..CRYPT_HEADER_SIZE].copy_from_slice(&sum.to_le_bytes());
                self.transform(buf, false);
            }
        }
    }

    /// Decrypt one datagram; on success `buf` shrinks to the bare payload.
    /// Bad CRC32/GCM tag → `Err`, the packet is dropped (tamper or garbage).
    pub(crate) fn open(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Null => Ok(()),
            Self::AesGcm(aead) => {
                use aes_gcm11::aead::Aead;

                if buf.len() < GCM_NONCE_SIZE + 16 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "kcp: short aead packet",
                    ));
                }
                let nonce: &aes_gcm11::Nonce<aes_gcm11::aead::consts::U12> = (&buf
                    [..GCM_NONCE_SIZE])
                    .try_into()
                    .expect("96-bit gcm nonce");
                let pt = aead
                    .decrypt(nonce, &buf[GCM_NONCE_SIZE..])
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "kcp: gcm open"))?;
                buf.clear();
                buf.extend_from_slice(&pt);
                Ok(())
            }
            _ => {
                if buf.len() < CRYPT_HEADER_SIZE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "kcp: short crypted packet",
                    ));
                }
                self.transform(buf, true);
                let stored =
                    u32::from_le_bytes(buf[NONCE_SIZE..CRYPT_HEADER_SIZE].try_into().unwrap());
                if stored != crc32fast::hash(&buf[CRYPT_HEADER_SIZE..]) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "kcp: packet crc mismatch",
                    ));
                }
                buf.drain(..CRYPT_HEADER_SIZE);
                Ok(())
            }
        }
    }

    /// The per-cipher transform over the whole envelope (CFB, XOR, salsa20
    /// or copy) — kcp-go's `BlockCrypt.Encrypt/Decrypt`.
    fn transform(&self, buf: &mut [u8], decrypt: bool) {
        match self {
            Self::Null | Self::None => {}
            Self::Xor(tbl) => {
                // Upstream `subtle.XORBytes` stops at the table length —
                // the tail of an oversized packet passes through
                // untouched (and fails CRC either way).
                for (b, t) in buf.iter_mut().take(MTU_LIMIT).zip(tbl.iter()) {
                    *b ^= *t;
                }
            }
            Self::Salsa20(key) => {
                use cipher::{KeyIvInit, StreamCipher};
                if buf.len() < 8 {
                    return;
                }
                let nonce: &salsa20::Nonce = (&buf[..8]).try_into().expect("salsa20 nonce");
                let mut cipher = salsa20::Salsa20::new(key.as_ref(), nonce);
                cipher.apply_keystream(&mut buf[8..]);
            }
            Self::Cfb(c) => cfb(&**c, buf, decrypt),
            Self::AesGcm(_) => unreachable!("aead path bypasses transform"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(name: &str, key: &[u8], payload: &[u8]) {
        let crypt = Crypt::new(name, key).unwrap();
        let mut buf = vec![0u8; crypt.header_size()];
        buf.extend_from_slice(payload);
        crypt.seal(&mut buf);
        let wire = buf.clone();
        crypt
            .open(&mut buf)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(buf, payload, "cipher {name} left {buf:?}");
        // `null` is a no-op; every other crypt fills a fresh random nonce
        // per packet (the envelope exists even for `none`), so two seals
        // of the same payload must differ on the wire.
        if name != "null" {
            let mut again = vec![0u8; crypt.header_size()];
            again.extend_from_slice(payload);
            crypt.seal(&mut again);
            assert_ne!(again, wire, "{name} reused its nonce");
        }
    }

    #[test]
    fn all_crypts_roundtrip() {
        let payload = b"kcptun wire compat \x00\xff with binary \xf0\x9f";
        for name in [
            "null",
            "none",
            "xor",
            "aes",
            "aes-256",
            "aes-128",
            "aes-192",
            "aes-128-gcm",
            "salsa20",
            "blowfish",
            "cast5",
            "3des",
            "twofish",
            "xtea",
            "tea",
            "unknown-name-maps-to-aes-256",
        ] {
            roundtrip(name, b"it's a secrect", payload);
            // Empty payload must round-trip too (KCP keepalive segments).
            roundtrip(name, b"it's a secrect", b"");
            // An odd-length payload exercises the CFB partial tail block.
            roundtrip(name, b"it's a secrect", b"odd!");
        }
    }

    #[test]
    fn crc_and_gcm_detect_tamper() {
        for name in ["aes", "none", "xor", "salsa20", "aes-128-gcm"] {
            let crypt = Crypt::new(name, b"it's a secrect").unwrap();
            let mut buf = vec![0u8; crypt.header_size()];
            buf.extend_from_slice(b"payload bytes");
            crypt.seal(&mut buf);
            let last = buf.len() - 1;
            buf[last] ^= 0x80;
            assert!(
                crypt.open(&mut buf).is_err(),
                "{name} accepted tampered packet"
            );
        }
        // `null` has no integrity tag — tampering is undetectable by design.
        let crypt = Crypt::new("null", b"key").unwrap();
        let mut buf = b"payload".to_vec();
        crypt.seal(&mut buf);
        buf[0] ^= 1;
        crypt.open(&mut buf).unwrap();
    }

    #[test]
    fn short_packet_rejected() {
        for name in ["aes", "none", "xor", "aes-128-gcm"] {
            let crypt = Crypt::new(name, b"it's a secrect").unwrap();
            assert!(crypt.open(&mut vec![0u8; 3]).is_err(), "{name}");
        }
    }

    /// x/crypto-generated vectors — a self-consistent roundtrip cannot
    /// catch a symmetric misimplementation, so the in-tree ciphers are
    /// pinned byte-exact here (and again end-to-end by the Go e2e).
    /// `tea.NewCipherWithRounds(key, 16)` / `xtea.NewCipher(key)`.
    #[test]
    fn in_tree_ciphers_match_go_vectors() {
        let key: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut blk = [0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48];
        Tea16::new(&key).encrypt(&mut blk);
        assert_eq!(blk, [0x1c, 0x5f, 0x50, 0xd6, 0xf3, 0xd8, 0xed, 0xb9]);
        let mut blk = [0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48];
        Xtea::new(&key).encrypt(&mut blk);
        assert_eq!(blk, [0x49, 0x7d, 0xf3, 0xd0, 0x72, 0x61, 0x2c, 0xb5]);

        let key2: [u8; 16] = [
            0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe, 1, 2, 3, 4, 5, 6, 7, 8,
        ];
        let mut blk2 = [0xff, 0x00, 0xaa, 0x55, 0x12, 0x34, 0x56, 0x78];
        Tea16::new(&key2).encrypt(&mut blk2);
        assert_eq!(blk2, [0x1c, 0xc6, 0x86, 0xa3, 0x64, 0xb0, 0x6b, 0xec]);
        let mut blk2 = [0xff, 0x00, 0xaa, 0x55, 0x12, 0x34, 0x56, 0x78];
        Xtea::new(&key2).encrypt(&mut blk2);
        assert_eq!(blk2, [0x2f, 0x53, 0x4f, 0x95, 0x6f, 0xd1, 0x2a, 0x7a]);
    }

    /// `NewSimpleXORBlockCrypt(pass)` derives its table via a second
    /// PBKDF2 over the *session* key — pin the head, tail, and length so
    /// an off-by-one in `take(MTU_LIMIT)` or a salt/rounds typo fails
    /// locally instead of only under the Go e2e.
    #[test]
    fn xor_table_matches_go_pbkdf2() {
        let Crypt::Xor(tbl) = Crypt::new("xor", b"it's a secrect").unwrap() else {
            panic!("xor must build the Xor variant");
        };
        assert_eq!(tbl.len(), 1500, "table must cover the MTU limit");
        let head: [u8; 32] = tbl[..32].try_into().unwrap();
        assert_eq!(
            head,
            [
                0x0f, 0xfe, 0x89, 0x87, 0xad, 0x94, 0x75, 0x71, 0x54, 0x79, 0x1b, 0xc8, 0xfa, 0x01,
                0xb6, 0x2c, 0x6b, 0xce, 0x01, 0x06, 0xb2, 0x95, 0x40, 0x13, 0xcd, 0x52, 0x3f, 0xbc,
                0x1f, 0x7a, 0xac, 0x8c,
            ]
        );
        let tail: [u8; 16] = tbl[1484..].try_into().unwrap();
        assert_eq!(
            tail,
            [
                0xdf, 0x44, 0x60, 0x78, 0x71, 0xb1, 0x9a, 0x36, 0x2f, 0x76, 0x40, 0xdb, 0x6a, 0x16,
                0x01, 0x14,
            ]
        );
    }

    /// `crypt=sm4` has no stable Rust implementation — the build refuses
    /// loudly at crypt-construction, not just at `plugin-opts` parsing.
    #[test]
    fn sm4_hard_errors() {
        match Crypt::new("sm4", b"key") {
            Err(e) => assert!(e.to_string().contains("sm4"), "{e}"),
            Ok(_) => panic!("sm4 must not map to AES"),
        }
    }

    /// The envelope must catch a wrong session key: for every crypt that
    /// carries integrity (CRC or GCM tag), opening a packet sealed under
    /// a different key fails. `none`/`null` carry no key-dependent check
    /// — by design, like upstream.
    #[test]
    fn wrong_key_fails_envelope() {
        for name in [
            "aes",
            "aes-128",
            "aes-192",
            "aes-128-gcm",
            "salsa20",
            "blowfish",
            "cast5",
            "3des",
            "twofish",
            "xtea",
            "tea",
            "xor",
        ] {
            let sealer = Crypt::new(name, b"it's a secrect").unwrap();
            let mut buf = vec![0u8; sealer.header_size()];
            buf.extend_from_slice(b"payload bytes");
            sealer.seal(&mut buf);
            let opener = Crypt::new(name, b"a different key").unwrap();
            assert!(
                opener.open(&mut buf).is_err(),
                "{name} opened a packet sealed under another key"
            );
        }
    }
}
