//! HarmonyOS physical network binding. No connect is allowed after a failed bind.
use paws_model::PawsError;

#[cfg(target_env = "ohos")]
mod platform {
    use std::ffi::CString;
    use std::io;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::os::fd::RawFd;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    use std::sync::{Arc, Once, OnceLock};

    #[repr(C)]
    struct NetHandle {
        net_id: i32,
    }
    #[link(name = "net_connection")]
    unsafe extern "C" {
        fn OH_NetConn_BindSocket(fd: i32, handle: *mut NetHandle) -> i32;
        fn OH_NetConn_GetAddrInfo(
            host: *mut libc::c_char,
            serv: *mut libc::c_char,
            hint: *mut libc::addrinfo,
            res: *mut *mut libc::addrinfo,
            net_id: i32,
        ) -> i32;
        fn OH_NetConn_FreeDnsResult(res: *mut libc::addrinfo) -> i32;
    }
    static NET_ID: AtomicI32 = AtomicI32::new(0);
    static BINDS: AtomicU64 = AtomicU64::new(0);
    static FAILURES: AtomicU64 = AtomicU64::new(0);
    static INSTALL: Once = Once::new();
    static DNS_WORKERS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    struct Protector;
    impl meow_common::SocketProtector for Protector {
        fn protect(&self, fd: RawFd) -> io::Result<()> {
            let net_id = NET_ID.load(Ordering::Acquire);
            if net_id <= 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "physical network unavailable",
                ));
            }
            let result = unsafe { OH_NetConn_BindSocket(fd, &mut NetHandle { net_id }) };
            BINDS.fetch_add(1, Ordering::Relaxed);
            if result != 0 {
                FAILURES.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    fd,
                    net_id,
                    result,
                    "outbound socket binding refused; connect blocked"
                );
                return Err(io::Error::other(format!(
                    "Harmony socket bind failed: {result}"
                )));
            }
            tracing::debug!(fd, net_id, result, "outbound socket bound before connect");
            Ok(())
        }
    }
    struct Bootstrap;
    #[async_trait::async_trait]
    impl meow_common::HostResolver for Bootstrap {
        async fn resolve(&self, host: &str) -> io::Result<IpAddr> {
            self.resolve_all(host)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| io::Error::other("empty physical DNS result"))
        }
        async fn resolve_all(&self, host: &str) -> io::Result<Vec<IpAddr>> {
            let host = CString::new(host).map_err(|_| io::Error::other("invalid DNS hostname"))?;
            let net_id = NET_ID.load(Ordering::Acquire);
            if net_id <= 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "physical DNS network unavailable",
                ));
            }
            tokio::time::timeout(std::time::Duration::from_secs(8), async move {
                let permit = DNS_WORKERS
                    .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| io::Error::other("physical DNS workers unavailable"))?;
                tokio::task::spawn_blocking(move || {
                    // Retain admission until the OS call exits, even when the caller times out.
                    let _permit = permit;
                    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
                    hints.ai_family = libc::AF_UNSPEC;
                    hints.ai_socktype = libc::SOCK_STREAM;
                    let mut result = std::ptr::null_mut();
                    let rc = unsafe {
                        OH_NetConn_GetAddrInfo(
                            host.as_ptr() as *mut _,
                            std::ptr::null_mut(),
                            &mut hints,
                            &mut result,
                            net_id,
                        )
                    };
                    if rc != 0 {
                        return Err(io::Error::other(format!("physical DNS failed: {rc}")));
                    }
                    let mut addresses = Vec::new();
                    let mut cursor = result;
                    // Platform owns this bounded addrinfo chain until FreeDnsResult.
                    while !cursor.is_null() && addresses.len() < 32 {
                        let info = unsafe { &*cursor };
                        if !info.ai_addr.is_null() {
                            if info.ai_family == libc::AF_INET {
                                let address =
                                    unsafe { &*(info.ai_addr as *const libc::sockaddr_in) };
                                addresses.push(IpAddr::V4(Ipv4Addr::from(
                                    address.sin_addr.s_addr.to_ne_bytes(),
                                )));
                            } else if info.ai_family == libc::AF_INET6 {
                                let address =
                                    unsafe { &*(info.ai_addr as *const libc::sockaddr_in6) };
                                addresses
                                    .push(IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)));
                            }
                        }
                        cursor = info.ai_next;
                    }
                    if !result.is_null() {
                        unsafe { OH_NetConn_FreeDnsResult(result) };
                    }
                    addresses.dedup();
                    if addresses.is_empty() {
                        return Err(io::Error::other("physical DNS returned no addresses"));
                    }
                    Ok(addresses)
                })
                .await
                .map_err(|_| io::Error::other("physical DNS worker failed"))?
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "physical DNS timed out"))?
        }
    }

    pub async fn resolve_host(host: &str) -> io::Result<Vec<IpAddr>> {
        meow_common::HostResolver::resolve_all(&Bootstrap, host).await
    }
    pub fn configure(net_id: i32) -> io::Result<()> {
        if net_id <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "physical network ID must be positive",
            ));
        }
        NET_ID.store(net_id, Ordering::Release);
        INSTALL.call_once(|| {
            meow_common::set_socket_protector(Arc::new(Protector));
            meow_common::socket_protect::set_bootstrap_host_resolver(Arc::new(Bootstrap));
        });
        Ok(())
    }
    pub fn snapshot() -> serde_json::Value {
        serde_json::json!({"physicalNetId":NET_ID.load(Ordering::Acquire),"bindAttempts":BINDS.load(Ordering::Relaxed),"bindFailures":FAILURES.load(Ordering::Relaxed),"failClosed":true})
    }
}

pub fn configure_physical_network(net_id: i32) -> Result<(), PawsError> {
    if net_id <= 0 {
        return Err(PawsError::Core(
            "physical network ID must be positive".into(),
        ));
    }
    #[cfg(target_env = "ohos")]
    platform::configure(net_id).map_err(|e| PawsError::Core(e.to_string()))?;
    Ok(())
}
pub fn physical_network_snapshot() -> serde_json::Value {
    #[cfg(target_env = "ohos")]
    return platform::snapshot();
    #[cfg(not(target_env = "ohos"))]
    serde_json::json!({"platform":"host","bindAttempts":0,"bindFailures":0,"failClosed":true})
}

/// Resolve only on the explicitly selected physical network. Used by the
/// independent bootstrap diagnostic; it does not claim to test remote DNS.
pub async fn resolve_physical_host(host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return Ok(vec![address]);
    }
    #[cfg(target_env = "ohos")]
    {
        platform::resolve_host(host).await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        Ok(tokio::net::lookup_host((host, 0))
            .await?
            .map(|a| a.ip())
            .take(32)
            .collect())
    }
}
