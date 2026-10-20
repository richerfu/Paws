//! Reed-Solomon FEC codec — a wire-faithful port of kcp-go's `fec.go`
//! (metacubex fork), plus the `autotune.go` decoder-side parameter probe.
//!
//! Packet layouts (all little-endian):
//!
//! ```text
//! data shard:   seqid(4) ‖ 0xf1(2) ‖ len(2) ‖ payload(len-2)
//! parity shard: seqid(4) ‖ 0xf2(2) ‖ RS-parity(padded to maxSize)
//! ```
//!
//! `len` counts itself plus the payload (`len ≥ 2`), matching upstream's
//! `PutUint16(b[payloadOffset:], len(b[payloadOffset:]))`. The RS code spans
//! the `len‖payload` region — data-shard `i` of a generation carries
//! `seqid == generation*shardSize + i`, parities occupy the trailing slots.
//!
//! Decoding is generational: a shard set collects packets keyed by
//! `seqid / shardSize`; once `dataShards` arrive (data+parity mixed),
//! `ReconstructData` recovers the missing rows. `autoTune` watches the
//! data/parity flag sequence to adopt the peer's shard ratio when it
//! diverges from ours — upstream initialises the decoder lazily at 1+1
//! precisely so a `datashard=0` client still decodes a FEC-enabled server.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use reed_solomon_erasure::galois_8::ReedSolomon;

use super::crypt::MTU_LIMIT;

/// `fecHeaderSize` upstream.
pub(crate) const FEC_HEADER_SIZE: usize = 6;
/// `fecHeaderSizePlus2` upstream: header + the u16 payload-size field.
pub(crate) const FEC_HEADER_SIZE_PLUS2: usize = FEC_HEADER_SIZE + 2;

pub(crate) const TYPE_DATA: u16 = 0xf1;
pub(crate) const TYPE_PARITY: u16 = 0xf2;
// `typeOOB = 0xf3` exists upstream for `SendUnreliable` datagrams — the
// plugin only runs stream mode, so we neither emit nor decode it (an OOB
// seqid of 0xffffffff trips the `paws` check and is dropped).

/// `maxShardSets` upstream — concurrent generations kept before eviction.
const MAX_SHARD_SETS: usize = 3;

/// Serial-time difference with wraparound (`_itimediff` upstream).
fn itimediff(a: u32, b: u32) -> i32 {
    (a.wrapping_sub(b)) as i32
}

/// FEC encoder state — one instance per KCP connection.
pub(crate) struct FecEncoder {
    data_shards: usize,
    parity_shards: usize,
    shard_size: usize,
    /// `paws`: protect-against-wrapped-seqid bound for `next`.
    paws: u32,
    next: u32,

    shard_count: usize,
    max_size: usize,
    header_offset: usize,
    payload_offset: usize,

    shard_cache: Vec<Vec<u8>>,
    /// `tsLatestPacket` — `None` mirrors upstream's zero value: a
    /// generation that completes before any prior packet timestamp
    /// exists (only possible for `data_shards == 1`) is discontinuous.
    ts_latest: Option<Instant>,
    codec: ReedSolomon,
}

impl FecEncoder {
    /// `newFECEncoder` — `None` when either shard count is zero
    /// (FEC disabled upstream means *no encoder*; the decoder still exists).
    pub(crate) fn new(
        data_shards: usize,
        parity_shards: usize,
        header_offset: usize,
    ) -> Option<Self> {
        if data_shards == 0 || parity_shards == 0 || data_shards.saturating_add(parity_shards) > 256
        {
            return None;
        }
        let codec = ReedSolomon::new(data_shards, parity_shards).ok()?;
        let shard_size = data_shards + parity_shards;
        Some(Self {
            data_shards,
            parity_shards,
            shard_size,
            paws: u32::MAX / shard_size as u32 * shard_size as u32,
            next: 0,
            shard_count: 0,
            max_size: 0,
            header_offset,
            payload_offset: header_offset + FEC_HEADER_SIZE,
            shard_cache: (0..shard_size).map(|_| vec![0u8; MTU_LIMIT]).collect(),
            ts_latest: None,
            codec,
        })
    }

    /// `encode` — seal `buf` as a data shard and, once `dataShards` have
    /// been collected, emit the parity shards (unless the generation was
    /// discontinuous — upstream skips parity when the inter-packet gap
    /// exceeds `rto_ms` since parity can only help a bursty loss anyway).
    ///
    /// `buf` layout on entry: `header_offset` scratch bytes ‖ payload.
    /// On return it is a fully-formed data packet: `… ‖ seqid ‖ 0xf1 ‖
    /// len ‖ payload`. Returned parity packets have the same layout with
    /// `0xf2` and are scratch-prefixed identically.
    pub(crate) fn encode(&mut self, buf: &mut [u8], rto_ms: u64) -> Vec<Vec<u8>> {
        // sealData: seqid ‖ typeData into the FEC header slot.
        buf[self.header_offset..self.header_offset + 4].copy_from_slice(&self.next.to_le_bytes());
        buf[self.header_offset + 4..self.payload_offset].copy_from_slice(&TYPE_DATA.to_le_bytes());
        self.next = self.next.wrapping_add(1) % self.paws;

        let payload_len = buf.len() - self.payload_offset;
        buf[self.payload_offset..self.payload_offset + 2]
            .copy_from_slice(&(payload_len as u16).to_le_bytes());

        let sz = buf.len();
        let row = &mut self.shard_cache[self.shard_count];
        // Go reslices `shardCache[k][:sz]` — capacity survives. `truncate`
        // would shrink permanently, panicking on the next larger packet.
        row.clear();
        row.extend_from_slice(&buf[..sz]);
        self.shard_count += 1;
        self.max_size = self.max_size.max(sz);

        let mut parity = Vec::new();
        let now = Instant::now();
        if self.shard_count == self.data_shards {
            // Skip parity generation on a discontinuous generation — the
            // seqid must still advance to preserve monotonicity upstream.
            let continuous = self
                .ts_latest
                .is_some_and(|t| now.duration_since(t).as_millis() < rto_ms as u128);
            if continuous {
                let max_size = self.max_size;
                // RS-encode the `len‖payload` region (payload_offset..)
                // across all shard rows, zero-padding short rows.
                let payload_offset = self.payload_offset;
                let mut shards: Vec<&mut [u8]> = Vec::with_capacity(self.shard_size);
                for row in &mut self.shard_cache {
                    row.resize(max_size, 0);
                    shards.push(&mut row[payload_offset..max_size]);
                }
                if self.codec.encode(&mut shards).is_ok() {
                    drop(shards);
                    for k in self.data_shards..self.shard_size {
                        let row = &mut self.shard_cache[k];
                        row[self.header_offset..self.header_offset + 4]
                            .copy_from_slice(&self.next.to_le_bytes());
                        row[self.header_offset + 4..self.payload_offset]
                            .copy_from_slice(&TYPE_PARITY.to_le_bytes());
                        self.next = self.next.wrapping_add(1) % self.paws;
                        parity.push(row[..max_size].to_vec());
                    }
                } else {
                    self.skip_parity();
                }
            } else {
                self.skip_parity();
            }
            self.shard_count = 0;
            self.max_size = 0;
        }
        self.ts_latest = Some(now);
        parity
    }

    /// `skipParity` — keep seqid monotonic when parity is not emitted.
    fn skip_parity(&mut self) {
        self.next = self.next.wrapping_add(self.parity_shards as u32) % self.paws;
    }
}

/// One decoded FEC packet — a borrowed view; `data()` yields `len‖payload`.
struct FecPacket<'a>(&'a [u8]);

impl FecPacket<'_> {
    fn seqid(&self) -> u32 {
        u32::from_le_bytes(self.0[..4].try_into().unwrap())
    }
    fn flag(&self) -> u16 {
        u16::from_le_bytes(self.0[4..6].try_into().unwrap())
    }
}

/// Per-generation shard set — upstream's `shardHeap` (a seq-ordered set;
/// ordering only matters for `discardShards`, so a HashSet + seq check
/// suffices).
#[derive(Default)]
struct ShardSet {
    seqids: HashSet<u32>,
    packets: Vec<Vec<u8>>,
}

/// `autoTune` — ring of (is_data, seqid) pulses; `find_period` detects a
/// full pulse width for a given bit value in seq order.
struct AutoTune {
    pulses: [(bool, u32); 258],
    head: usize,
    tail: usize,
    count: usize,
}

impl AutoTune {
    fn new() -> Self {
        Self {
            pulses: [(false, 0); 258],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    fn sample(&mut self, bit: bool, seq: u32) {
        self.pulses[self.tail] = (bit, seq);
        self.tail = (self.tail + 1) % 258;
        if self.count < 258 {
            self.count += 1;
        } else {
            self.head = (self.head + 1) % 258;
        }
    }

    /// `FindPeriod` — width of the first complete pulse of `bit` in
    /// seq-sorted samples; `-1` upstream → `None` here.
    fn find_period(&self, bit: bool) -> Option<usize> {
        if self.count < 3 {
            return None;
        }
        let mut sorted: Vec<(bool, u32)> = (0..self.count)
            .map(|i| self.pulses[(self.head + i) % 258])
            .collect();
        // Upstream orders by the wrap-aware serial diff — but `itimediff`
        // is not a total order (a pair 2³¹ apart compares <0 in both
        // directions), and Rust's `sort_by` panics on that since 1.81.
        // Sort by forward distance from the set's serial-minimum instead:
        // a real total order that yields the same serial-ascending result.
        let origin = sorted.iter().map(|&(_, s)| s).fold(sorted[0].1, |acc, s| {
            if itimediff(s, acc) < 0 {
                s
            } else {
                acc
            }
        });
        sorted.sort_by_key(|&(_, s)| s.wrapping_sub(origin));

        // Left edge: first transition into `bit` on consecutive seqs —
        // a gap anywhere before the edge aborts the scan (upstream).
        let mut left = None;
        for i in 1..sorted.len() {
            let (prev_bit, prev_seq) = sorted[i - 1];
            let (cur_bit, cur_seq) = sorted[i];
            if prev_seq.wrapping_add(1) == cur_seq {
                if prev_bit != bit && cur_bit == bit {
                    left = Some(i);
                    break;
                }
            } else {
                return None;
            }
        }
        let left = left?;

        // Right edge: first transition out of `bit` after `left`.
        for i in left + 1..sorted.len() {
            let (prev_bit, prev_seq) = sorted[i - 1];
            let (cur_bit, cur_seq) = sorted[i];
            if prev_seq.wrapping_add(1) == cur_seq {
                if prev_bit == bit && cur_bit != bit {
                    return Some(i - left);
                }
            } else {
                return None;
            }
        }
        None
    }
}

/// FEC decoder — always constructed (upstream lazily defaults to 1+1 so a
/// no-FEC client still decodes a FEC-enabled peer via autotune).
pub(crate) struct FecDecoder {
    data_shards: usize,
    parity_shards: usize,
    shard_size: usize,
    paws: u32,
    should_tune: bool,

    shard_set: HashMap<u32, ShardSet>,
    newest_shard_id: u32,
    auto_tune: AutoTune,
    codec: ReedSolomon,
}

impl FecDecoder {
    pub(crate) fn new(data_shards: usize, parity_shards: usize) -> Self {
        // Upstream `newFECDecoder(0,0)` returns nil; the session then falls
        // back to a lazy 1+1 decoder on the first FEC packet. We fold that
        // fallback in here: a zeroed config decodes nothing but keeps
        // autotune ready to latch onto the peer's real parameters.
        // `reed-solomon-erasure` rejects ds+ps > 256; upstream's decoder
        // (lazy nil → tuned from the peer's parity flags) has no such
        // construction failure. A config exceeding the RS limit can't
        // reconstruct anyway — clamp the pair itself (not just the codec)
        // to the 1+1 fallback so `shard_size`/`paws` math can't overflow
        // on a provider-supplied `datashard=usize::MAX`; autotune still
        // latches onto the peer's real parameters.
        let (ds, ps) = if data_shards == 0
            || parity_shards == 0
            || data_shards.saturating_add(parity_shards) > 256
        {
            (1, 1)
        } else {
            (data_shards, parity_shards)
        };
        let codec = ReedSolomon::new(ds, ps).expect("bounded shard counts build");
        let shard_size = ds + ps;
        Self {
            data_shards: ds,
            parity_shards: ps,
            shard_size,
            paws: u32::MAX / shard_size as u32 * shard_size as u32,
            should_tune: false,
            shard_set: HashMap::new(),
            newest_shard_id: 0,
            auto_tune: AutoTune::new(),
            codec,
        }
    }

    /// `decode` — feed one decrypted packet. Returns recovered data-shard
    /// `len‖payload` blobs (already RS-reconstructed); the caller feeds
    /// `r[2..2+len]` to `kcp.input`.
    pub(crate) fn decode(&mut self, pkt: &[u8]) -> Vec<Vec<u8>> {
        if pkt.len() < FEC_HEADER_SIZE_PLUS2 {
            return Vec::new();
        }
        let f = FecPacket(pkt);
        self.auto_tune.sample(f.flag() == TYPE_DATA, f.seqid());

        if f.seqid() >= self.paws {
            return Vec::new();
        }

        // Packet type vs position sanity: a mismatch means the peer's
        // shard ratio differs from ours — autotune to it.
        if (f.seqid() % self.shard_size as u32) < self.data_shards as u32 {
            if f.flag() != TYPE_DATA {
                self.should_tune = true;
            }
        } else if f.flag() != TYPE_PARITY {
            self.should_tune = true;
        }

        if self.should_tune {
            let auto_ds = self.auto_tune.find_period(true);
            let auto_ps = self.auto_tune.find_period(false);
            if let (Some(ds), Some(ps)) = (auto_ds, auto_ps) {
                // Same RS ceiling as the encoder (<=256) — a peer at
                // exactly 256 total shards is legal and must still tune.
                // Upstream's autotune gate is `< 256` while its own
                // `newFECDecoder` accepts `<= 256` — we follow the
                // constructor bound (refusing to tune to a legal config
                // would wedge FEC for the session).
                if ds > 0 && ps > 0 && ds + ps <= 256 {
                    if ds != self.data_shards || ps != self.parity_shards {
                        self.data_shards = ds;
                        self.parity_shards = ps;
                        self.shard_size = ds + ps;
                        self.shard_set.clear();
                        if let Ok(codec) = ReedSolomon::new(ds, ps) {
                            self.codec = codec;
                        }
                        self.paws = u32::MAX / self.shard_size as u32 * self.shard_size as u32;
                    }
                    self.should_tune = false;
                }
            }
            return Vec::new();
        }

        let shard_id = f.seqid() / self.shard_size as u32;
        let shard = self.shard_set.entry(shard_id).or_default();
        if shard.seqids.contains(&f.seqid()) {
            return Vec::new();
        }
        shard.seqids.insert(f.seqid());
        shard.packets.push(pkt.to_vec());

        let mut recovered = Vec::new();
        if shard.packets.len() >= self.data_shards {
            // Drain the generation into the decode cache indexed by
            // `seqid % shardSize`; rows hold the `len‖payload` region.
            let mut shards: Vec<Option<Vec<u8>>> = vec![None; self.shard_size];
            let mut present_data = vec![false; self.shard_size];
            let mut num_data = 0usize;
            let mut maxlen = 0usize;
            for pkt in shard.packets.drain(..) {
                let seqid = u32::from_le_bytes(pkt[..4].try_into().unwrap());
                let flag = u16::from_le_bytes(pkt[4..6].try_into().unwrap());
                let idx = (seqid % self.shard_size as u32) as usize;
                if flag == TYPE_DATA {
                    num_data += 1;
                    present_data[idx] = true;
                }
                maxlen = maxlen.max(pkt.len() - FEC_HEADER_SIZE);
                shards[idx] = Some(pkt[FEC_HEADER_SIZE..].to_vec());
            }
            shard.seqids.clear();

            // case 2 upstream: some data shards missing → reconstruct.
            // Present rows are zero-padded to maxlen; missing rows stay
            // `None` — `reconstruct_data` fills only absent *data* rows.
            if num_data < self.data_shards {
                for d in shards.iter_mut().flatten() {
                    d.resize(maxlen, 0);
                }
                if self.codec.reconstruct_data(&mut shards).is_ok() {
                    for (k, row) in shards.iter_mut().enumerate().take(self.data_shards) {
                        if !present_data[k] {
                            if let Some(d) = row.take() {
                                recovered.push(d);
                            }
                        }
                    }
                }
            }
        }

        if itimediff(
            shard_id.wrapping_mul(self.shard_size as u32),
            self.newest_shard_id.wrapping_mul(self.shard_size as u32),
        ) > 0
        {
            self.newest_shard_id = shard_id;
        }
        // `discardShards`: drop generations older than MAX_SHARD_SETS.
        let shard_size = self.shard_size as u32;
        let newest = self.newest_shard_id;
        self.shard_set.retain(|&sid, _| {
            itimediff(
                newest.wrapping_mul(shard_size),
                sid.wrapping_mul(shard_size),
            ) <= MAX_SHARD_SETS as i32 * shard_size as i32
        });

        recovered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a data packet the way `encode` expects it: `header_offset`
    /// scratch bytes + the FEC-header+len scratch region, then payload.
    /// With `header_offset = 0` the scratch is `FEC_HEADER_SIZE_PLUS2`.
    fn make_encoder(ds: usize, ps: usize) -> FecEncoder {
        FecEncoder::new(ds, ps, 0).unwrap()
    }

    fn data_packet(payload: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0u8; FEC_HEADER_SIZE_PLUS2];
        pkt.extend_from_slice(payload);
        pkt
    }

    fn shard_payloads(pkt: &[u8]) -> &[u8] {
        // Strip seqid+flag+len → bare payload.
        let len = u16::from_le_bytes(pkt[6..8].try_into().unwrap()) as usize;
        &pkt[8..6 + len]
    }

    #[test]
    fn disabled_when_shards_zero() {
        assert!(FecEncoder::new(0, 0, 0).is_none());
        assert!(FecEncoder::new(10, 0, 0).is_none());
        assert!(FecEncoder::new(0, 3, 0).is_none());
        // Upstream refuses >256 total shards (seqid space is mod 256).
        assert!(FecEncoder::new(200, 100, 0).is_none());
    }

    #[test]
    fn roundtrip_no_loss() {
        let mut enc = make_encoder(3, 2);
        let mut dec = FecDecoder::new(3, 2);
        // Parity is emitted when each generation completes (packets 2 and
        // 5 here): upstream compares against the previous packet's
        // timestamp, so a back-to-back generation is "continuous".
        for i in 0..6u8 {
            let mut pkt = data_packet(&[i; 17]);
            let parity = enc.encode(&mut pkt, 1000);
            assert_eq!(parity.len(), if i % 3 == 2 { 2 } else { 0 });
            assert!(dec.decode(&pkt).is_empty());
            for p in &parity {
                assert!(dec.decode(p).is_empty());
            }
            assert_eq!(shard_payloads(&pkt), &[i; 17]);
        }
    }

    #[test]
    fn recovers_lost_data_shard() {
        let mut enc = make_encoder(4, 2);
        let mut dec = FecDecoder::new(4, 2);
        let mut packets = Vec::new();
        let mut parity = Vec::new();
        for i in 0..4u8 {
            let mut pkt = data_packet(&vec![i + 1; 100 + i as usize]); // varying sizes
            parity.extend(enc.encode(&mut pkt, 1000));
            packets.push(pkt);
        }
        assert_eq!(parity.len(), 2);
        // Drop data shard 1; deliver 0,2,3 + both parities → 5 packets ≥ ds.
        let mut recovered = Vec::new();
        for pkt in packets.iter().skip(2).chain(parity.iter()) {
            recovered.extend(dec.decode(pkt));
        }
        recovered.extend(dec.decode(&packets[0]));
        // The reconstruct triggered at 4 packets recovers *both* missing
        // data rows — shard 0 hadn't been delivered yet either. Rows come
        // back in `seqid % shard_size` order: recovered[0]=shard0,
        // recovered[1]=shard1 (the `len‖payload` region).
        assert_eq!(recovered.len(), 2);
        let len = u16::from_le_bytes(recovered[1][..2].try_into().unwrap()) as usize;
        assert_eq!(&recovered[1][2..len], &vec![2u8; 101][..]);
    }

    #[test]
    fn duplicate_and_short_packets_ignored() {
        let mut dec = FecDecoder::new(3, 2);
        let mut enc = make_encoder(3, 2);
        let mut pkt = data_packet(&[7u8; 20]);
        enc.encode(&mut pkt, 1000);
        assert!(dec.decode(&pkt).is_empty());
        assert!(dec.decode(&pkt).is_empty()); // dup
        assert!(dec.decode(&[0u8; 5]).is_empty()); // short
    }

    #[test]
    fn decoder_autotunes_to_peer_ratio() {
        // Peer sends 2+1, our decoder configured for 4+2 — autotune must
        // latch onto 2+1 and recover. Tuning needs two full generations
        // of flag pulses; packets fed while `should_tune` holds are
        // consumed by the tuner and produce nothing.
        let mut enc = make_encoder(2, 1);
        let mut dec = FecDecoder::new(4, 2);
        for gen in 0..2u8 {
            let mut a = data_packet(&[gen; 33]);
            let mut b = data_packet(&[gen + 10; 33]);
            assert!(enc.encode(&mut a, 1000).is_empty());
            let par = enc.encode(&mut b, 1000);
            assert_eq!(par.len(), 1);
            assert!(dec.decode(&a).is_empty());
            assert!(dec.decode(&b).is_empty());
            for p in &par {
                dec.decode(p);
            }
        }

        // Tuned to 2+1 now: drop data shard 1 of the next generation,
        // deliver data shard 0 plus the parity — recovery must fire.
        let mut pkt = data_packet(&[9u8; 33]);
        assert!(enc.encode(&mut pkt, 1000).is_empty()); // generation open
        let mut dropped = data_packet(&[8u8; 33]);
        let parity = enc.encode(&mut dropped, 1000); // completes gen
        assert_eq!(parity.len(), 1); // 2+1: one parity shard

        let mut recovered = dec.decode(&pkt);
        for p in &parity {
            recovered.extend(dec.decode(p));
        }
        // The dropped data shard comes back as its `len‖payload` region.
        assert_eq!(recovered.len(), 1, "tuned decoder must recover a loss");
        let len = u16::from_le_bytes(recovered[0][..2].try_into().unwrap()) as usize;
        assert_eq!(&recovered[0][2..len], &[8u8; 33][..]);
    }
}
