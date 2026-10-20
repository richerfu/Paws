//! Sliding-window replay filter over a monotonically increasing packet-ID
//! space — the mechanism SIP022 §3.2.4 requires of every AEAD-2022 relay
//! session (the same construction WireGuard uses for its replay defense).
//!
//! The filter is keyed by *session*: both directions of a Shadowsocks 2022
//! UDP relay session carry their own packet-ID counter, so the inbound side
//! (server filtering client IDs) and the outbound side (client filtering
//! server IDs) each keep one window per session.

/// `u64` limbs backing the bitmap.
const WINDOW_LIMBS: usize = 32;
/// Trailing window width in packet IDs (2048). IDs older than
/// `max_seen - WINDOW_BITS` are rejected outright rather than recorded.
const WINDOW_BITS: u64 = WINDOW_LIMBS as u64 * 64;

/// Bitmap sliding window. Position `i` — counted back from `max_seen`, so
/// position 0 is the newest — maps to packet ID `max_seen - i`; position `i`
/// lives at `bits[i / 64] & (1 << (i % 64))`.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    /// Largest packet ID accepted so far. A fresh window starts at 0 with an
    /// all-zero bitmap, so ID 0 is accepted exactly once like any other.
    max_seen: u64,
    bits: [u64; WINDOW_LIMBS],
}

impl ReplayWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` — and records the ID — when `id` may be processed:
    /// it has not been seen before and is not older than the window.
    /// Replays and pre-window IDs return `false`.
    ///
    /// `u64::MAX` is always rejected: accepting it would pin `max_seen` at
    /// the top of the ID space, making every subsequent legitimate ID
    /// pre-window and deadening the session (the reference
    /// `PacketWindowFilter` rejects `id >= limit` with `u64::MAX` passed at
    /// every call site).
    pub fn check_and_set(&mut self, id: u64) -> bool {
        if id == u64::MAX {
            return false;
        }
        if id > self.max_seen {
            self.shift(id - self.max_seen);
            self.max_seen = id;
            self.bits[0] |= 1;
            return true;
        }
        let diff = self.max_seen - id;
        if diff >= WINDOW_BITS {
            return false;
        }
        let limb = &mut self.bits[(diff / 64) as usize];
        let mask = 1u64 << (diff % 64);
        if *limb & mask != 0 {
            return false;
        }
        *limb |= mask;
        true
    }

    /// Advance `max_seen` by `by`, moving every recorded bit `by` positions
    /// toward older offsets (higher limbs). Bits shifted past the window are
    /// forgotten.
    fn shift(&mut self, by: u64) {
        if by >= WINDOW_BITS {
            self.bits = [0; WINDOW_LIMBS];
            return;
        }
        let limbs = (by / 64) as usize;
        let rem = (by % 64) as u32;
        for i in (0..WINDOW_LIMBS).rev() {
            let mut v = if i >= limbs { self.bits[i - limbs] } else { 0 };
            if rem != 0 {
                v <<= rem;
                if i > limbs {
                    v |= self.bits[i - limbs - 1] >> (64 - rem);
                }
            }
            self.bits[i] = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_ids_accept_once() {
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(0));
        assert!(w.check_and_set(1));
        assert!(w.check_and_set(2));
        assert!(!w.check_and_set(0), "replay of an old ID is dropped");
        assert!(!w.check_and_set(1));
        assert!(!w.check_and_set(2));
    }

    #[test]
    fn out_of_order_within_window_accepted() {
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(100));
        assert!(w.check_and_set(50), "in-window reorder is fine");
        assert!(!w.check_and_set(50));
        assert!(!w.check_and_set(100));
        assert!(w.check_and_set(0), "still inside the trailing window");
    }

    #[test]
    fn ids_older_than_window_rejected() {
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(0));
        assert!(w.check_and_set(WINDOW_BITS));
        assert!(!w.check_and_set(0), "pre-window ID is too old");
        assert!(w.check_and_set(1), "still inside the trailing window");
    }

    #[test]
    fn huge_jump_resets_window() {
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(7));
        assert!(w.check_and_set(u64::MAX - 3), "jump past the window width");
        assert!(
            !w.check_and_set(7),
            "everything before the jump is pre-window"
        );
        assert!(!w.check_and_set(u64::MAX - 3));
        assert!(!w.check_and_set(u64::MAX), "u64::MAX is never accepted");
    }

    #[test]
    fn jump_clears_stale_bits() {
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(63), "records position 0 (max_seen = 63)");
        assert!(w.check_and_set(0), "records position 63 (bits[0] bit 63)");
        // A shift of exactly one window width must clear every recorded bit.
        assert!(w.check_and_set(63 + WINDOW_BITS));
        assert!(
            w.check_and_set(63 + WINDOW_BITS - 63),
            "position 63 must not carry the stale bit into the new window"
        );
    }

    #[test]
    fn limb_aligned_shift() {
        // The rem == 0 shift path (by a multiple of 64) is a pure limb copy.
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(5));
        assert!(w.check_and_set(5 + 64));
        assert!(!w.check_and_set(5), "the recorded bit moved a whole limb");
        assert!(w.check_and_set(4), "in-window reorder still tracked");
    }

    #[test]
    fn maximal_non_clearing_shift() {
        // by = WINDOW_BITS - 1: limbs = 31, rem = 63 — the deepest carry.
        let mut w = ReplayWindow::new();
        assert!(w.check_and_set(0));
        assert!(w.check_and_set(WINDOW_BITS - 1));
        assert!(
            !w.check_and_set(0),
            "id 0 must sit at the window's trailing edge"
        );
        assert!(!w.check_and_set(WINDOW_BITS - 1));
    }

    #[test]
    fn partial_limb_shifts_preserve_bits() {
        let mut w = ReplayWindow::new();
        // Non-monotonic acceptance straddling limb boundaries.
        for id in [100, 63, 64, 200, 65, 130] {
            assert!(w.check_and_set(id), "id {id}");
        }
        for id in [100, 63, 64, 200, 65, 130] {
            assert!(!w.check_and_set(id), "replayed id {id}");
        }
        // A small jump keeps old IDs inside the window — still recorded.
        assert!(w.check_and_set(330));
        assert!(!w.check_and_set(130), "its bit must have shifted intact");
        assert!(!w.check_and_set(100));
        assert!(w.check_and_set(229), "fresh in-window ID");
    }
}
