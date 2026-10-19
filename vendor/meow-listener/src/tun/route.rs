//! RAII route installation for the TUN inbound's `auto-route`.
//!
//! v1 deliberately routes only the fake-IP range into the device (see the
//! module docs in `mod.rs` for the loop-freedom argument). Routes are added
//! with the blocking `route_manager` API at listener startup and removed on
//! drop; a failed add is a warning, not a fatal error, because the device
//! subnet's own on-link route frequently already covers the range (in which
//! case some platforms report "route exists").

use ipnet::IpNet;
use route_manager::{Route, RouteManager};
use tracing::{debug, warn};

pub(super) struct RouteGuard {
    manager: RouteManager,
    installed: Vec<Route>,
}

impl RouteGuard {
    /// Install one on-link route per net through interface `if_index`.
    /// Individual failures are logged and skipped so a pre-existing
    /// equivalent route does not abort listener startup.
    pub(super) fn setup(if_index: u32, nets: &[IpNet]) -> std::io::Result<Self> {
        let mut manager = RouteManager::new()?;
        let mut installed = Vec::with_capacity(nets.len());
        for net in nets {
            let route = Route::new(net.network(), net.prefix_len()).with_if_index(if_index);
            match manager.add(&route) {
                Ok(()) => {
                    debug!("tun auto-route: added {net} via if_index {if_index}");
                    installed.push(route);
                }
                Err(e) => warn!(
                    "tun auto-route: failed to add {net} via if_index {if_index}: {e} \
                     (continuing — the device subnet may already cover it)"
                ),
            }
        }
        Ok(Self { manager, installed })
    }
}

impl Drop for RouteGuard {
    fn drop(&mut self) {
        for route in &self.installed {
            if let Err(e) = self.manager.delete(route) {
                warn!("tun auto-route: failed to remove {route}: {e}");
            }
        }
    }
}

/// Detect the physical interface carrying the IPv4 default route, for
/// global route scope's outbound-socket binding (#375). Linux-only for now:
/// reads `/proc/net/route` and returns the interface of the first UP
/// `0.0.0.0/0` entry — captured **before** the TUN's own split defaults go
/// in, so the TUN device can never be the answer.
pub(super) fn default_interface() -> std::io::Result<String> {
    #[cfg(target_os = "linux")]
    {
        let table = std::fs::read_to_string("/proc/net/route")?;
        parse_default_interface(&table).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no IPv4 default route found in /proc/net/route",
            )
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "default-interface auto-detection is Linux-only (tracked on #375)",
        ))
    }
}

/// Pure parser behind [`default_interface`], split out for unit testing on
/// every host (hence `test` in the cfg — only Linux uses it at runtime).
/// `/proc/net/route` columns: Iface, Destination (hex LE),
/// Gateway, Flags (hex; bit 0 = RTF_UP), … A default route has destination
/// `00000000` and the UP flag set.
#[cfg(any(target_os = "linux", test))]
fn parse_default_interface(table: &str) -> Option<String> {
    for line in table.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let (Some(iface), Some(dest), _gateway, Some(flags)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let up = u32::from_str_radix(flags, 16).is_ok_and(|f| f & 0x1 != 0);
        if dest == "00000000" && up {
            return Some(iface.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_default_interface;

    const SAMPLE: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";

    #[test]
    fn picks_the_up_default_route_interface() {
        assert_eq!(parse_default_interface(SAMPLE).as_deref(), Some("eth0"));
    }

    #[test]
    fn ignores_down_defaults_and_empty_tables() {
        // Same default entry but with the UP bit clear → not a candidate.
        let down = SAMPLE.replace("00000000\t0101A8C0\t0003", "00000000\t0101A8C0\t0002");
        assert_eq!(parse_default_interface(&down), None);
        assert_eq!(parse_default_interface("Iface\tDestination\n"), None);
        assert_eq!(parse_default_interface(""), None);
    }
}
