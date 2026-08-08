use futures::{SinkExt, StreamExt};
use paws_model::{DnsQuerySummary, PawsError, VpnOptions, VpnStack};
use meow_common::sniffer::{sniff_http, sniff_tls, SnifferConfig};
use meow_common::{ConnType, Metadata, Network, ProxyConn, ProxyPacketConn};
use meow_listener::SnifferRuntime;
use meow_trie::DomainTrie;
use meow_tunnel::Tunnel;
use netstack_smoltcp::StackBuilder;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::RawFd;
use std::os::raw::c_void;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};

mod dns;
mod netstack;
mod packet;

use dns::*;
use netstack::*;
use packet::*;

const DNS_BURST_CAP: usize = 256;
const DNS_CACHE_MAX_TTL: u32 = 300;
const DNS_CACHE_MAX_RECORDS: usize = 512;
const DNS_TABLE_MAX_RECORDS: usize = 1024;
const DNS_RECENT_QUERY_LIMIT: usize = 16;
const UDP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const UDP_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const UDP_RESPONSE_BUFFER_SIZE: usize = 64 * 1024;
const SNIFF_BUFFER_SIZE: usize = 8 * 1024;
static LWIP_RUNTIME_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type FlowTasks = Arc<Mutex<Vec<JoinHandle<()>>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VpnLifecycle {
    Stopped,
    Running { fd: i32 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub tcp_packets: u64,
    pub udp_packets: u64,
    pub dns_packets: u64,
    pub dns_cache_hits: u64,
    pub dns_cache_misses: u64,
    pub dropped_packets: u64,
    pub recent_dns_queries: Vec<DnsQuerySummary>,
}

#[derive(Debug, Default)]
struct SharedStats {
    rx_packets: AtomicU64,
    tx_packets: AtomicU64,
    rx_bytes: AtomicU64,
    tx_bytes: AtomicU64,
    tcp_packets: AtomicU64,
    udp_packets: AtomicU64,
    dns_packets: AtomicU64,
    dns_cache_hits: AtomicU64,
    dns_cache_misses: AtomicU64,
    dropped_packets: AtomicU64,
    recent_dns_queries: Mutex<VecDeque<DnsQuerySummary>>,
}

impl SharedStats {
    fn snapshot(&self) -> TunStats {
        let recent_dns_queries = self
            .recent_dns_queries
            .lock()
            .map(|queries| queries.iter().cloned().collect())
            .unwrap_or_default();
        TunStats {
            rx_packets: self.rx_packets.load(Ordering::Relaxed),
            tx_packets: self.tx_packets.load(Ordering::Relaxed),
            rx_bytes: self.rx_bytes.load(Ordering::Relaxed),
            tx_bytes: self.tx_bytes.load(Ordering::Relaxed),
            tcp_packets: self.tcp_packets.load(Ordering::Relaxed),
            udp_packets: self.udp_packets.load(Ordering::Relaxed),
            dns_packets: self.dns_packets.load(Ordering::Relaxed),
            dns_cache_hits: self.dns_cache_hits.load(Ordering::Relaxed),
            dns_cache_misses: self.dns_cache_misses.load(Ordering::Relaxed),
            dropped_packets: self.dropped_packets.load(Ordering::Relaxed),
            recent_dns_queries,
        }
    }

    fn record_dns_query(&self, name: String, record_type: String) {
        if let Ok(mut queries) = self.recent_dns_queries.lock() {
            if let Some(index) = queries.iter().position(|query| {
                query.name.eq_ignore_ascii_case(&name) && query.record_type == record_type
            }) {
                if let Some(mut query) = queries.remove(index) {
                    query.name = name;
                    query.count = query.count.saturating_add(1);
                    queries.push_front(query);
                }
            } else {
                queries.push_front(DnsQuerySummary {
                    name,
                    record_type,
                    count: 1,
                });
            }
            while queries.len() > DNS_RECENT_QUERY_LIMIT {
                queries.pop_back();
            }
        }
    }
}

#[derive(Debug)]
struct RunningTask {
    handle: JoinHandle<()>,
    running: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    dns_table: DnsTable,
    dns_cache: DnsResponseCache,
}

impl RunningTask {
    fn abort(self) {
        self.running.store(false, Ordering::SeqCst);
        let _handle = self.handle;
    }
}

#[derive(Debug, Clone)]
pub struct TunSession {
    state: Arc<Mutex<VpnLifecycle>>,
    options: Arc<Mutex<Option<VpnOptions>>>,
    task: Arc<Mutex<Option<RunningTask>>>,
}

impl Default for TunSession {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(VpnLifecycle::Stopped)),
            options: Arc::new(Mutex::new(None)),
            task: Arc::new(Mutex::new(None)),
        }
    }
}

impl TunSession {
    pub fn start(
        &self,
        fd: i32,
        options: VpnOptions,
        tunnel: Tunnel,
        sniffer_config: SnifferConfig,
    ) -> Result<(), PawsError> {
        if fd < 0 {
            return Err(PawsError::Core(format!("invalid tun fd: {fd}")));
        }
        let stack = VpnStack::try_from(options.stack.as_str())?;
        self.stop()?;

        let duplicated_fd = duplicate_fd(fd)?;
        set_nonblocking(duplicated_fd)?;

        let stats = Arc::new(SharedStats::default());
        let task_stats = stats.clone();
        let running = Arc::new(AtomicBool::new(true));
        let task_running = running.clone();
        let dns_table = DnsTable::default();
        let dns_cache = DnsResponseCache::default();
        let dns_hijacking = options.dns_hijacking;
        let task_dns_table = dns_table.clone();
        let task_dns_cache = dns_cache.clone();
        let handle = tokio::spawn(async move {
            if let Err(error) = run_netstack_vpn(
                stack,
                duplicated_fd,
                tunnel,
                task_stats,
                task_running,
                dns_hijacking,
                sniffer_config,
                task_dns_table,
                task_dns_cache,
            )
            .await
            {
                let _ = error;
            }
        });

        *self
            .state
            .lock()
            .map_err(|_| PawsError::Core("vpn state lock poisoned".to_owned()))? =
            VpnLifecycle::Running { fd };
        *self
            .options
            .lock()
            .map_err(|_| PawsError::Core("vpn options lock poisoned".to_owned()))? = Some(options);
        *self
            .task
            .lock()
            .map_err(|_| PawsError::Core("vpn task lock poisoned".to_owned()))? =
            Some(RunningTask {
                handle,
                running,
                stats,
                dns_table,
                dns_cache,
            });
        Ok(())
    }

    pub fn stop(&self) -> Result<(), PawsError> {
        if let Some(task) = self
            .task
            .lock()
            .map_err(|_| PawsError::Core("vpn task lock poisoned".to_owned()))?
            .take()
        {
            task.abort();
        }
        *self
            .state
            .lock()
            .map_err(|_| PawsError::Core("vpn state lock poisoned".to_owned()))? =
            VpnLifecycle::Stopped;
        *self
            .options
            .lock()
            .map_err(|_| PawsError::Core("vpn options lock poisoned".to_owned()))? = None;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        matches!(
            *self.state.lock().expect("vpn state lock"),
            VpnLifecycle::Running { .. }
        )
    }

    pub fn fd(&self) -> Option<i32> {
        match *self.state.lock().expect("vpn state lock") {
            VpnLifecycle::Stopped => None,
            VpnLifecycle::Running { fd } => Some(fd),
        }
    }

    pub fn options(&self) -> Option<VpnOptions> {
        self.options.lock().expect("vpn options lock").clone()
    }

    pub fn stats(&self) -> Option<TunStats> {
        self.task
            .lock()
            .expect("vpn task lock")
            .as_ref()
            .map(|task| task.stats.snapshot())
    }

    pub fn flush_dns_cache(&self) -> Result<(), PawsError> {
        let task = self
            .task
            .lock()
            .map_err(|_| PawsError::Core("vpn task lock poisoned".to_owned()))?;
        if let Some(task) = task.as_ref() {
            task.dns_table.clear();
            task.dns_cache.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
