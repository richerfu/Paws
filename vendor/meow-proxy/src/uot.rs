//! Shared `uot.AddrParser` per-packet address headers — the datagram
//! framing inside a UDP-over-TCP stream, used by both the anytls uot
//! reader and kcptun's legacy `sp.udp-over-tcp.arpa` relay.
//!
//! Per-packet layout (both directions):
//!
//! ```text
//! | ATYP | Address | Port  |
//! | u8   | var     | u16be |
//! ```
//!
//! ATYP is the uot family table (NOT the SOCKS5 one): 0=IPv4, 1=IPv6,
//! 2=len-prefixed domain.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use meow_common::error::{MeowError, Result};
use tokio::io::{AsyncRead, AsyncReadExt};

/// `uot.AddrParser` address-family bytes — every **per-packet** header.
pub(crate) const UOT_ATYP_IPV4: u8 = 0x00;
pub(crate) const UOT_ATYP_IPV6: u8 = 0x01;
pub(crate) const UOT_ATYP_DOMAIN: u8 = 0x02;

/// Append a per-packet uot address header (`uot.AddrParser` bytes).
pub(crate) fn encode_uot_addr(buf: &mut Vec<u8>, addr: &SocketAddr) {
    match addr.ip() {
        IpAddr::V4(v4) => {
            buf.push(UOT_ATYP_IPV4);
            buf.extend_from_slice(&v4.octets());
        }
        // Upstream unmaps before serializing, so a `::ffff:a.b.c.d` peer goes
        // out as plain IPv4 rather than as an IPv6 address the server would
        // then have to unmap itself.
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => {
                buf.push(UOT_ATYP_IPV4);
                buf.extend_from_slice(&v4.octets());
            }
            None => {
                buf.push(UOT_ATYP_IPV6);
                buf.extend_from_slice(&v6.octets());
            }
        },
    }
    buf.extend_from_slice(&addr.port().to_be_bytes());
}

/// Append a per-packet uot domain address header — the form a chained
/// `UdpTarget::Name` stamps so the server resolves the name (issue #657).
/// `host` must be ≤255 bytes (the length prefix is a u8).
///
/// Only the opt-in `anytls` adapter writes the domain form — kcptun's UoT
/// target is always the `SocketAddr` magic — so without the feature this
/// serializer has no production caller and would trip `-D warnings`.
#[cfg(any(feature = "anytls", test))]
pub(crate) fn encode_uot_addr_domain(buf: &mut Vec<u8>, host: &str, port: u16) {
    let host_bytes = host.as_bytes();
    debug_assert!(
        host_bytes.len() <= u8::MAX as usize,
        "domain length checked by caller"
    );
    buf.push(UOT_ATYP_DOMAIN);
    buf.push(u8::try_from(host_bytes.len()).expect("domain length checked by caller"));
    buf.extend_from_slice(host_bytes);
    buf.extend_from_slice(&port.to_be_bytes());
}

/// Read a per-packet uot address header.
///
/// Domain-form replies are best-effort, matching `trojan.rs`: an IP literal is
/// parsed, anything else degrades to `0.0.0.0:<port>`. Servers echo the IP
/// form here — the FQDN branch exists because the serializer allows it.
pub(crate) async fn read_uot_addr<R: AsyncRead + Unpin>(reader: &mut R) -> Result<SocketAddr> {
    let mut atyp = [0u8; 1];
    reader.read_exact(&mut atyp).await.map_err(MeowError::Io)?;
    let ip = match atyp[0] {
        UOT_ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            reader
                .read_exact(&mut octets)
                .await
                .map_err(MeowError::Io)?;
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        UOT_ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            reader
                .read_exact(&mut octets)
                .await
                .map_err(MeowError::Io)?;
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        UOT_ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            reader.read_exact(&mut len).await.map_err(MeowError::Io)?;
            let mut domain = vec![0u8; len[0] as usize];
            reader
                .read_exact(&mut domain)
                .await
                .map_err(MeowError::Io)?;
            std::str::from_utf8(&domain)
                .ok()
                .and_then(|d| d.parse::<IpAddr>().ok())
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        }
        other => {
            return Err(MeowError::Proxy(format!(
                "uot: unknown address type {other:#x}"
            )));
        }
    };
    let mut port = [0u8; 2];
    reader.read_exact(&mut port).await.map_err(MeowError::Io)?;
    Ok(SocketAddr::new(ip, u16::from_be_bytes(port)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_header_encodes_uot_family_bytes() {
        let mut buf = Vec::new();
        encode_uot_addr(&mut buf, &"1.2.3.4:80".parse().unwrap());
        assert_eq!(buf, vec![UOT_ATYP_IPV4, 1, 2, 3, 4, 0, 80]);

        let mut buf = Vec::new();
        encode_uot_addr(&mut buf, &"[::1]:80".parse().unwrap());
        assert_eq!(buf[0], UOT_ATYP_IPV6);
        assert_eq!(buf.len(), 1 + 16 + 2);

        // v4-mapped peers are unmapped, as upstream does before serializing.
        let mut buf = Vec::new();
        encode_uot_addr(&mut buf, &"[::ffff:1.2.3.4]:80".parse().unwrap());
        assert_eq!(buf, vec![UOT_ATYP_IPV4, 1, 2, 3, 4, 0, 80]);
    }

    #[tokio::test]
    async fn read_uot_addr_round_trips_encode() {
        for addr in ["1.2.3.4:53", "[2001:db8::1]:443"] {
            let addr: SocketAddr = addr.parse().unwrap();
            let mut buf = Vec::new();
            encode_uot_addr(&mut buf, &addr);
            let mut reader: &[u8] = &buf;
            assert_eq!(read_uot_addr(&mut reader).await.unwrap(), addr);
        }
    }

    /// The domain write form (`UdpTarget::Name`, issue #657) round-trips
    /// through the parser's FQDN branch — which degrades a non-literal to
    /// `0.0.0.0:<port>` — and an IP-literal host survives as an address.
    #[tokio::test]
    async fn encode_uot_addr_domain_wire_layout() {
        let mut buf = Vec::new();
        encode_uot_addr_domain(&mut buf, "back.internal", 8388);
        assert_eq!(
            buf[..2],
            [UOT_ATYP_DOMAIN, 13],
            "atyp + one-byte host length"
        );
        assert_eq!(&buf[2..15], b"back.internal");
        assert_eq!(&buf[15..], &8388u16.to_be_bytes());

        // Parser accepts the form (degrades non-literal to unspecified+port).
        let mut reader: &[u8] = &buf;
        assert_eq!(
            read_uot_addr(&mut reader).await.unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8388)
        );

        let mut buf = Vec::new();
        encode_uot_addr_domain(&mut buf, "203.0.113.9", 5300);
        let mut reader: &[u8] = &buf;
        assert_eq!(
            read_uot_addr(&mut reader).await.unwrap(),
            "203.0.113.9:5300".parse().unwrap(),
            "IP-literal domain form round-trips to the literal"
        );
    }

    /// Domain-form replies degrade to `0.0.0.0:<port>` unless the "domain" is
    /// an IP literal — same best-effort rule as `trojan.rs`.
    #[tokio::test]
    async fn read_uot_addr_handles_domain_form() {
        let mut wire = vec![UOT_ATYP_DOMAIN, 11];
        wire.extend_from_slice(b"example.com");
        wire.extend_from_slice(&443u16.to_be_bytes());
        let mut reader: &[u8] = &wire;
        assert_eq!(
            read_uot_addr(&mut reader).await.unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 443)
        );

        let mut wire = vec![UOT_ATYP_DOMAIN, 7];
        wire.extend_from_slice(b"1.2.3.4");
        wire.extend_from_slice(&53u16.to_be_bytes());
        let mut reader: &[u8] = &wire;
        assert_eq!(
            read_uot_addr(&mut reader).await.unwrap(),
            "1.2.3.4:53".parse::<SocketAddr>().unwrap()
        );
    }

    #[tokio::test]
    async fn read_uot_addr_rejects_unknown_family() {
        let mut reader: &[u8] = &[0x09, 0, 0, 0, 0, 0, 0];
        let Err(err) = read_uot_addr(&mut reader).await else {
            panic!("unknown address family must error");
        };
        assert!(err.to_string().contains("unknown address type"), "{err}");
    }
}
