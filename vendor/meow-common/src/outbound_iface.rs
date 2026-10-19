//! Outbound-socket interface binding for TUN global-route mode (#375).
//!
//! With `tun.auto-route: global` the split default routes send *all* IPv4
//! traffic into the TUN device — including, without countermeasures, meow's
//! own dials to proxy upstreams and DIRECT destinations, which would re-enter
//! the device and loop. The countermeasure is per-socket: every outbound
//! socket meow creates is bound to the physical interface **before**
//! `connect()`/`bind()`, so its packets take the physical route regardless of
//! the routing table (Linux `SO_BINDTODEVICE`; macOS `IP_BOUND_IF` and
//! Windows `IP_UNICAST_IF` are follow-ups tracked on #375).
//!
//! This module is the process-global registry for that interface, mirroring
//! the `SocketProtector` pattern in [`crate::socket_protect`]: the TUN
//! listener installs the interface at startup (and clears it on teardown),
//! and the dial chokepoints ([`crate::connect_tcp`], [`crate::bind_udp`],
//! plus the marked-socket path in `meow-proxy`) apply it to each socket.
//!
//! The registry itself compiles on every platform so call sites stay free of
//! `cfg` spaghetti; [`set_outbound_interface`] fails with `Unsupported` on
//! platforms where the binding syscall is not implemented yet, which lets the
//! TUN listener fail closed instead of starting a looping configuration.

use std::io;
use std::sync::Arc;

use parking_lot::RwLock;

static INTERFACE: RwLock<Option<Arc<str>>> = RwLock::new(None);

/// Install the physical interface every subsequent outbound socket binds to.
/// Validates that the interface exists (Linux `if_nametoindex`). Errors with
/// `Unsupported` on platforms where per-socket binding is not implemented —
/// callers must treat that as fatal for global-route mode, not a warning.
pub fn set_outbound_interface(name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "outbound interface name is empty",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let c_name = std::ffi::CString::new(name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("outbound interface name '{name}' contains a NUL byte"),
            )
        })?;
        // SAFETY: `c_name` is a valid NUL-terminated string for the call.
        let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
        if index == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("outbound interface '{name}' does not exist"),
            ));
        }
        *INTERFACE.write() = Some(Arc::from(name));
        tracing::info!("outbound sockets bound to interface '{name}' (SO_BINDTODEVICE)");
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "outbound interface binding ('{name}') is not implemented on this \
                 platform yet (tracked on #375; Linux only for now)"
            ),
        ))
    }
}

/// Remove the installed interface; subsequent sockets bind normally.
pub fn clear_outbound_interface() {
    if INTERFACE.write().take().is_some() {
        tracing::info!("outbound interface binding cleared");
    }
}

/// The currently installed interface, if any.
pub fn outbound_interface() -> Option<Arc<str>> {
    INTERFACE.read().clone()
}

/// Bind `socket` to the installed interface, if one is installed. No-op when
/// none is. Callers must invoke this **before** `connect()`/`bind()` so the
/// very first packet already takes the physical route.
#[cfg(target_os = "linux")]
pub fn apply_outbound_interface(socket: &socket2::Socket) -> io::Result<()> {
    if let Some(name) = outbound_interface() {
        socket.bind_device(Some(name.as_bytes()))?;
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// One test drives the whole install → apply → clear sequence because the
    /// registry is process-global; separate `#[test]` fns would race.
    #[test]
    fn install_apply_clear_roundtrip() {
        assert!(outbound_interface().is_none());

        // A bogus interface must be rejected and leave nothing installed.
        assert!(set_outbound_interface("no-such-iface-zz9").is_err());
        assert!(set_outbound_interface("").is_err());
        assert!(outbound_interface().is_none());

        // Loopback exists on every Linux CI host.
        set_outbound_interface("lo").expect("lo must exist");
        assert_eq!(outbound_interface().as_deref(), Some("lo"));

        // Binding a fresh socket to lo succeeds and loopback dials still work.
        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .unwrap();
        apply_outbound_interface(&socket).expect("bind_device to lo");

        clear_outbound_interface();
        assert!(outbound_interface().is_none());

        // With nothing installed, apply is a no-op.
        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .unwrap();
        apply_outbound_interface(&socket2).expect("no-op apply");
    }
}
