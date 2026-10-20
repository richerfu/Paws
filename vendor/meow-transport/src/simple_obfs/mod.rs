//! SIP004 simple-obfs — HTTP / TLS traffic obfuscation for shadowsocks.
//!
//! A stream-wrapper codec that frames an inner byte stream as fake HTTP/1.1 or
//! fake TLS so a passive observer sees plain web traffic instead of raw
//! shadowsocks. Port of mihomo's `transport/simple-obfs` Go package.
//!
//! # Layout
//!
//! * `client` — outbound (proxy-client) wrappers. Used by the SS and Snell
//!   adapters in `meow-proxy` when the YAML config sets `plugin: obfs`.
//! * `server` — inbound (proxy-server) wrappers, the counterpart that strips
//!   the fake HTTP request / fake TLS ClientHello. Used by the shadowsocks
//!   *listener*.
//!
//! Both halves share the same fake-TLS record and ClientHello framing, so a
//! single round-trip test (`client` writes → `server` reads) guards the pair
//! against framing drift — the reason the codec lives here rather than being
//! duplicated in `meow-proxy` and `meow-listener`.

pub mod client;
pub mod server;
