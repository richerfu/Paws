//! In-process kcptun client transport (issue #533) — wire-compatible with
//! mihomo's `transport/kcptun` (kcp-go + smux + snappy).
//!
//! This crate supplies the bottom two layers only: `KcpStream` is the
//! KCP-over-UDP reliable byte stream with the crypt/FEC envelope, and
//! `CompStream` is the snappy framing wrapper. The smux session,
//! connection pool and scavenger live in `meow-proxy`'s kcptun plugin —
//! the crate boundary keeps this module transport-only.
//!
//! ```text
//! SS stream ─ smux stream ─ CompStream(snappy) ─ KcpStream ─ UDP socket
//!                                                │
//!                              crypt envelope (nonce‖crc‖payload,
//!                              or aead nonce‖seal) + optional RS-FEC
//! ```

mod conn;
mod crypt;
mod fec;
/// Vendored `kcp` crate reworked to `kcp-go` v5.6.72 semantics — see the
/// file header for the ported retransmission/`Input` deltas.
#[allow(
    dead_code,
    reason = "vendored library — keeps the full upstream API surface; conn.rs uses a subset"
)]
mod kcp_go;

pub use conn::{KcpStream, SocketIo};

/// `NewCompStream` upstream — the snappy framed-stream layer a kcptun client
/// applies when `nocomp` is off (the default). Wire-identical to
/// golang/snappy's `NewBufferedWriter`/`NewReader` pair.
pub type CompStream = tokio_snappy::SnappyIO<KcpStream>;

/// Wrap a connected `KcpStream` in [`CompStream`].
pub fn comp_stream(stream: KcpStream) -> CompStream {
    CompStream::new(stream)
}

/// `mtuLimit` upstream — socket scratch sizing.
pub(crate) use crypt::MTU_LIMIT;

/// `kcptun.Config` — same field names/semantics as upstream; `fill_defaults`
/// reproduces `FillDefaults()` including the mode-preset overrides.
#[derive(Debug, Clone)]
pub struct KcpConfig {
    /// PBKDF2 key material for the crypt layer.
    pub key: String,
    /// Crypt name (`crypt` SIP003 key); `none`/`null` select plaintext modes.
    pub crypt: String,
    /// `mode` — selects the `normal`/`fast`/`fast2`/`fast3`/`manual` preset.
    pub mode: String,
    /// `conn` — smux sessions pooled per client.
    pub conn: u16,
    /// `autoexpire` — seconds before a pooled session is rotated.
    pub auto_expire: u32,
    /// `scavengettl` — parsed for upstream parity; the pool keeps live
    /// sessions rather than upstream's per-conn scavenge list, so this is
    /// currently advisory.
    pub scavenge_ttl: u32,
    /// `mtu` — UDP payload ceiling; drives KCP `mss`.
    pub mtu: u32,
    /// `ratelimit` — outbound token bucket, bytes/sec; 0 disables it.
    /// Enforcement granularity is the KCP interval (upstream uses a
    /// `rate.Limiter`); functional and bounded either way.
    pub rate_limit: u64,
    /// `sndwnd` — KCP send window.
    pub snd_wnd: u32,
    /// `rcvwnd` — KCP receive window.
    pub rcv_wnd: u32,
    /// `datashard` — Reed-Solomon data shards (`0` disables FEC).
    pub data_shard: usize,
    /// `parityshard` — Reed-Solomon parity shards.
    pub parity_shard: usize,
    /// `dscp` — raw TOS byte for the underlying socket.
    pub dscp: u32,
    /// `nocomp` — skips snappy compression.
    pub no_comp: bool,
    /// `acknodelay` — KCP ACK pacing.
    pub ack_nodelay: bool,
    /// `nodelay`/`interval`/`resend`/`nc` — the KCP tuning tuple, normally
    /// set via `mode` presets.
    pub nodelay: u32,
    pub interval: u32,
    pub resend: i32,
    pub nc: u32,
    /// `sockbuf` — OS socket buffer size (direct-socket path only).
    pub sock_buf: u32,
    /// `smuxver` — smux protocol version; only `1` is supported.
    pub smux_ver: u16,
    /// `smuxbuf` — smux session buffer.
    pub smux_buf: u32,
    /// `framesize` — smux max frame size.
    pub frame_size: u32,
    /// `streambuf` — smux per-stream buffer.
    pub stream_buf: u32,
    /// `keepalive` — smux keepalive interval in seconds.
    pub keep_alive: u32,
}

impl Default for KcpConfig {
    fn default() -> Self {
        let mut c = Self::blank();
        c.fill_defaults();
        c
    }
}

impl KcpConfig {
    /// An all-zero config — upstream's `Config` before `FillDefaults`.
    /// Option parsers must start here and call [`fill_defaults`](Self::fill_defaults)
    /// exactly once at the end: starting from `Default` would apply a mode
    /// preset *before* the user's `mode` is known, so `manual` would
    /// inherit the `fast` leftovers instead of the zero-state fields.
    pub fn blank() -> Self {
        Self {
            key: String::new(),
            crypt: String::new(),
            mode: String::new(),
            conn: 0,
            auto_expire: 0,
            scavenge_ttl: 0,
            mtu: 0,
            rate_limit: 0,
            snd_wnd: 0,
            rcv_wnd: 0,
            data_shard: 0,
            parity_shard: 0,
            dscp: 0,
            no_comp: false,
            ack_nodelay: false,
            nodelay: 0,
            interval: 0,
            resend: 0,
            nc: 0,
            sock_buf: 0,
            smux_ver: 0,
            smux_buf: 0,
            frame_size: 0,
            stream_buf: 0,
            keep_alive: 0,
        }
    }

    /// `FillDefaults()` — verbatim upstream ordering, including the mode
    /// presets clobbering explicit nodelay/interval/resend/nc (they only
    /// survive under `mode: manual`).
    pub fn fill_defaults(&mut self) {
        if self.key.is_empty() {
            self.key = "it's a secrect".into();
        }
        if self.crypt.is_empty() {
            self.crypt = "aes".into();
        }
        if self.mode.is_empty() {
            self.mode = "fast".into();
        }
        if self.conn == 0 {
            self.conn = 1;
        }
        if self.scavenge_ttl == 0 {
            self.scavenge_ttl = 600;
        }
        if self.mtu == 0 {
            self.mtu = 1350;
        }
        if self.snd_wnd == 0 {
            self.snd_wnd = 128;
        }
        if self.rcv_wnd == 0 {
            self.rcv_wnd = 512;
        }
        if self.data_shard == 0 {
            self.data_shard = 10;
        }
        if self.parity_shard == 0 {
            self.parity_shard = 3;
        }
        if self.interval == 0 {
            self.interval = 50;
        }
        if self.sock_buf == 0 {
            self.sock_buf = 4194304;
        }
        if self.smux_ver == 0 {
            self.smux_ver = 1;
        }
        if self.smux_buf == 0 {
            self.smux_buf = 4194304;
        }
        if self.frame_size == 0 {
            self.frame_size = 8192;
        }
        if self.stream_buf == 0 {
            self.stream_buf = 2097152;
        }
        if self.keep_alive == 0 {
            self.keep_alive = 10;
        }
        match self.mode.as_str() {
            "normal" => (self.nodelay, self.interval, self.resend, self.nc) = (0, 40, 2, 1),
            "fast" => (self.nodelay, self.interval, self.resend, self.nc) = (0, 30, 2, 1),
            "fast2" => (self.nodelay, self.interval, self.resend, self.nc) = (1, 20, 2, 1),
            "fast3" => (self.nodelay, self.interval, self.resend, self.nc) = (1, 10, 2, 1),
            _ => {}
        }
        // Upstream `FillDefaults` bounds the flush interval to [10, 5000]ms
        // — a 0 here would spin `ikcp_check` always-due.
        self.interval = self.interval.clamp(10, 5000);
    }
}
