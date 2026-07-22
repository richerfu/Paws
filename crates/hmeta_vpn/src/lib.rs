use futures::{SinkExt, StreamExt};
use hmeta_model::{DnsQuerySummary, HMetaError, VpnOptions, VpnStack};
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
    ) -> Result<(), HMetaError> {
        if fd < 0 {
            return Err(HMetaError::Core(format!("invalid tun fd: {fd}")));
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
            .map_err(|_| HMetaError::Core("vpn state lock poisoned".to_owned()))? =
            VpnLifecycle::Running { fd };
        *self
            .options
            .lock()
            .map_err(|_| HMetaError::Core("vpn options lock poisoned".to_owned()))? = Some(options);
        *self
            .task
            .lock()
            .map_err(|_| HMetaError::Core("vpn task lock poisoned".to_owned()))? =
            Some(RunningTask {
                handle,
                running,
                stats,
                dns_table,
                dns_cache,
            });
        Ok(())
    }

    pub fn stop(&self) -> Result<(), HMetaError> {
        if let Some(task) = self
            .task
            .lock()
            .map_err(|_| HMetaError::Core("vpn task lock poisoned".to_owned()))?
            .take()
        {
            task.abort();
        }
        *self
            .state
            .lock()
            .map_err(|_| HMetaError::Core("vpn state lock poisoned".to_owned()))? =
            VpnLifecycle::Stopped;
        *self
            .options
            .lock()
            .map_err(|_| HMetaError::Core("vpn options lock poisoned".to_owned()))? = None;
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

    pub fn flush_dns_cache(&self) -> Result<(), HMetaError> {
        let task = self
            .task
            .lock()
            .map_err(|_| HMetaError::Core("vpn task lock poisoned".to_owned()))?;
        if let Some(task) = task.as_ref() {
            task.dns_table.clear();
            task.dns_cache.clear();
        }
        Ok(())
    }
}

async fn run_netstack_vpn(
    stack_kind: VpnStack,
    fd: RawFd,
    tunnel: Tunnel,
    stats: Arc<SharedStats>,
    running: Arc<AtomicBool>,
    dns_hijacking: bool,
    sniffer_config: SnifferConfig,
    dns_table: DnsTable,
    dns_cache: DnsResponseCache,
) -> io::Result<()> {
    // lwIP owns process-global C state. Serialize teardown and recreation so
    // a quick VPN reconnect can never leave two stacks mutating it together.
    let _lwip_guard = if stack_kind == VpnStack::Lwip {
        Some(LWIP_RUNTIME_LOCK.lock().await)
    } else {
        None
    };
    if !running.load(Ordering::SeqCst) {
        unsafe {
            libc::close(fd);
        }
        return Ok(());
    }

    let udp_sessions = UdpSessionMap::default();
    let flow_tasks: FlowTasks = Arc::new(Mutex::new(Vec::new()));
    let sniffer = HarmonyTcpSniffer::from_config(sniffer_config).map(Arc::new);
    let runtime = match stack_kind {
        VpnStack::Smoltcp => spawn_smoltcp_backend(
            tunnel.clone(),
            stats.clone(),
            dns_table.clone(),
            udp_sessions.clone(),
            sniffer.clone(),
            flow_tasks.clone(),
        ),
        VpnStack::Lwip => spawn_lwip_backend(
            tunnel.clone(),
            stats.clone(),
            dns_table.clone(),
            udp_sessions.clone(),
            sniffer.clone(),
            flow_tasks.clone(),
        ),
    };
    let NetstackRuntime {
        ingress_tx,
        egress_tx,
        mut egress_rx,
        handles,
    } = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }
    };

    let udp_sweeper_sessions = udp_sessions;
    let udp_sweeper_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(UDP_SWEEP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            udp_sweeper_sessions.retain_active(UDP_IDLE_TIMEOUT);
        }
    });

    let writer_stats = stats.clone();
    let writer_handle = tokio::spawn(async move {
        while let Some(pkt) = egress_rx.recv().await {
            if write_tun_packet(fd, &pkt).await {
                writer_stats.tx_packets.fetch_add(1, Ordering::Relaxed);
                writer_stats
                    .tx_bytes
                    .fetch_add(pkt.len() as u64, Ordering::Relaxed);
            }
        }
    });

    let reader_running = running;
    let reader_stats = stats;
    let reader_dns_table = dns_table;
    let reader_dns_cache = dns_cache;
    let reader_tunnel = tunnel;
    let reader_egress_tx = egress_tx;
    let reader_handle = tokio::spawn(async move {
        let mut read_buf = vec![0_u8; 65535];
        let dns_sem = Arc::new(Semaphore::new(DNS_BURST_CAP));
        while reader_running.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
            let mut did_work = false;
            loop {
                let n =
                    unsafe { libc::read(fd, read_buf.as_mut_ptr() as *mut c_void, read_buf.len()) };
                if n <= 0 {
                    break;
                }
                did_work = true;
                let n = n as usize;
                let ip_data = &read_buf[..n];
                reader_stats.rx_packets.fetch_add(1, Ordering::Relaxed);
                reader_stats.rx_bytes.fetch_add(n as u64, Ordering::Relaxed);

                if let Some((src_ip, src_port, dst_ip, dst_port, payload)) =
                    tun_dns_query_from_packet(dns_hijacking, ip_data)
                {
                    reader_stats.udp_packets.fetch_add(1, Ordering::Relaxed);
                    reader_stats.dns_packets.fetch_add(1, Ordering::Relaxed);
                    if let Some(query) = parse_dns_query(payload) {
                        reader_stats.record_dns_query(query.name, query.kind.as_str().to_owned());
                    }
                    let permit = match dns_sem.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            reader_stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                            if let Some(packet) = build_dns_servfail_udp_packet(
                                src_ip, src_port, dst_ip, dst_port, payload,
                            ) {
                                let _ = reader_egress_tx.send(packet);
                            }
                            continue;
                        }
                    };
                    let query = payload.to_vec();
                    let reply_tx = reader_egress_tx.clone();
                    let dns_table = reader_dns_table.clone();
                    let dns_cache = reader_dns_cache.clone();
                    let tunnel = reader_tunnel.clone();
                    let stats = reader_stats.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        handle_dns_query(
                            tunnel, dns_table, dns_cache, stats, src_ip, src_port, dst_ip,
                            dst_port, query, reply_tx,
                        )
                        .await;
                    });
                    continue;
                }

                match ingress_tx.try_send(ip_data.to_vec()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(frame)) => {
                        let _ = ingress_tx.send(frame).await;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }

            if !did_work {
                tokio::time::sleep(tokio::time::Duration::from_micros(200)).await;
            }
        }
    });

    let _ = reader_handle.await;
    for handle in handles {
        handle.abort();
        let _ = handle.await;
    }
    abort_flow_tasks(&flow_tasks).await;
    udp_sweeper_handle.abort();
    let _ = udp_sweeper_handle.await;
    writer_handle.abort();
    let _ = writer_handle.await;
    unsafe {
        libc::close(fd);
    }
    Ok(())
}

struct NetstackRuntime {
    ingress_tx: mpsc::Sender<Vec<u8>>,
    egress_tx: mpsc::UnboundedSender<Vec<u8>>,
    egress_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    handles: Vec<JoinHandle<()>>,
}

fn spawn_smoltcp_backend(
    tunnel: Tunnel,
    stats: Arc<SharedStats>,
    dns_table: DnsTable,
    udp_sessions: UdpSessionMap,
    sniffer: Option<Arc<HarmonyTcpSniffer>>,
    flow_tasks: FlowTasks,
) -> io::Result<NetstackRuntime> {
    let (mut stack, tcp_runner, udp_socket, tcp_listener) = StackBuilder::default()
        .enable_tcp(true)
        .enable_udp(true)
        .stack_buffer_size(1024)
        .tcp_buffer_size(512)
        .build()?;

    let tcp_runner = tcp_runner.expect("TCP runner");
    let mut tcp_listener = tcp_listener.expect("TCP listener");
    let udp_socket = udp_socket.expect("UDP socket");
    let (ingress_tx, mut ingress_rx) = mpsc::channel::<Vec<u8>>(256);
    let (egress_tx, egress_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (udp_reply_tx, mut udp_reply_rx) = mpsc::unbounded_channel::<UdpReply>();

    let runner_handle = tokio::spawn(async move {
        let _ = tcp_runner.await;
    });

    let egress_tx_for_stack = egress_tx.clone();
    let stack_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                pkt = ingress_rx.recv() => {
                    let Some(frame) = pkt else { break };
                    if stack.send(frame).await.is_err() {
                        break;
                    }
                }
                pkt = stack.next() => {
                    match pkt {
                        Some(Ok(frame)) => {
                            let _ = egress_tx_for_stack.send(frame);
                        }
                        Some(Err(_)) | None => break,
                    }
                }
            }
        }
    });

    let accept_tunnel = tunnel.clone();
    let accept_dns_table = dns_table.clone();
    let accept_stats = stats.clone();
    let accept_sniffer = sniffer.clone();
    let accept_flow_tasks = flow_tasks.clone();
    let accept_handle = tokio::spawn(async move {
        while let Some((stream, local_addr, remote_addr)) = tcp_listener.next().await {
            accept_stats.tcp_packets.fetch_add(1, Ordering::Relaxed);
            let flow_tunnel = accept_tunnel.clone();
            let flow_dns_table = accept_dns_table.clone();
            let flow_sniffer = accept_sniffer.clone();
            let handle = tokio::spawn(async move {
                handle_tcp_stream(
                    flow_tunnel,
                    stream,
                    local_addr,
                    remote_addr,
                    flow_dns_table,
                    flow_sniffer,
                )
                .await;
            });
            track_flow_task(&accept_flow_tasks, handle);
        }
    });

    let (mut udp_read_half, mut udp_write_half) = udp_socket.split();
    let udp_reply_handle = tokio::spawn(async move {
        while let Some(reply) = udp_reply_rx.recv().await {
            let _ = udp_write_half
                .send((reply.data, reply.remote, reply.local))
                .await;
        }
    });

    let udp_tunnel = tunnel.clone();
    let udp_dns_table = dns_table.clone();
    let udp_stats = stats.clone();
    let udp_reply_tx_for_reader = udp_reply_tx.clone();
    let udp_sessions_for_reader = udp_sessions.clone();
    let udp_flow_tasks = flow_tasks;
    let udp_handle = tokio::spawn(async move {
        while let Some((data, local, remote)) = udp_read_half.next().await {
            udp_stats.udp_packets.fetch_add(1, Ordering::Relaxed);
            let tunnel = udp_tunnel.clone();
            let dns_table = udp_dns_table.clone();
            let sessions = udp_sessions_for_reader.clone();
            let reply_tx = udp_reply_tx_for_reader.clone();
            let response_flow_tasks = udp_flow_tasks.clone();
            let handle = tokio::spawn(async move {
                handle_udp_datagram(
                    tunnel,
                    dns_table,
                    sessions,
                    reply_tx,
                    response_flow_tasks,
                    data,
                    local,
                    remote,
                )
                .await;
            });
            track_flow_task(&udp_flow_tasks, handle);
        }
    });

    Ok(NetstackRuntime {
        ingress_tx,
        egress_tx,
        egress_rx,
        handles: vec![
            runner_handle,
            stack_handle,
            accept_handle,
            udp_handle,
            udp_reply_handle,
        ],
    })
}

fn spawn_lwip_backend(
    tunnel: Tunnel,
    stats: Arc<SharedStats>,
    dns_table: DnsTable,
    udp_sessions: UdpSessionMap,
    sniffer: Option<Arc<HarmonyTcpSniffer>>,
    flow_tasks: FlowTasks,
) -> io::Result<NetstackRuntime> {
    let (mut stack, mut tcp_listener, udp_socket) = lwip::NetStack::with_buffer_size(1024, 256)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let (udp_write, mut udp_read) = udp_socket.split();
    let (ingress_tx, mut ingress_rx) = mpsc::channel::<Vec<u8>>(256);
    let (egress_tx, egress_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (udp_reply_tx, mut udp_reply_rx) = mpsc::unbounded_channel::<UdpReply>();

    let driver_egress_tx = egress_tx.clone();
    let driver_udp_reply_tx = udp_reply_tx.clone();
    let driver_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                pkt = ingress_rx.recv() => {
                    let Some(frame) = pkt else { break };
                    if stack.send(frame).await.is_err() {
                        break;
                    }
                }
                pkt = stack.next() => {
                    match pkt {
                        Some(Ok(frame)) => {
                            let _ = driver_egress_tx.send(frame);
                        }
                        Some(Err(_)) | None => break,
                    }
                }
                accepted = tcp_listener.next() => {
                    let Some((stream, local_addr, remote_addr)) = accepted else { break };
                    stats.tcp_packets.fetch_add(1, Ordering::Relaxed);
                    let flow_tunnel = tunnel.clone();
                    let flow_dns_table = dns_table.clone();
                    let flow_sniffer = sniffer.clone();
                    let handle = tokio::spawn(async move {
                        handle_tcp_stream(
                            flow_tunnel,
                            stream,
                            local_addr,
                            remote_addr,
                            flow_dns_table,
                            flow_sniffer,
                        )
                        .await;
                    });
                    track_flow_task(&flow_tasks, handle);
                }
                datagram = udp_read.next() => {
                    let Some((data, local, remote)) = datagram else { break };
                    stats.udp_packets.fetch_add(1, Ordering::Relaxed);
                    let flow_tunnel = tunnel.clone();
                    let flow_dns_table = dns_table.clone();
                    let flow_sessions = udp_sessions.clone();
                    let reply_tx = driver_udp_reply_tx.clone();
                    let response_flow_tasks = flow_tasks.clone();
                    let handle = tokio::spawn(async move {
                        handle_udp_datagram(
                            flow_tunnel,
                            flow_dns_table,
                            flow_sessions,
                            reply_tx,
                            response_flow_tasks,
                            data,
                            local,
                            remote,
                        )
                        .await;
                    });
                    track_flow_task(&flow_tasks, handle);
                }
                reply = udp_reply_rx.recv() => {
                    let Some(reply) = reply else { break };
                    if udp_write
                        .send_to(&reply.data, &reply.remote, &reply.local)
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    Ok(NetstackRuntime {
        ingress_tx,
        egress_tx,
        egress_rx,
        handles: vec![driver_handle],
    })
}

fn track_flow_task(flow_tasks: &FlowTasks, handle: JoinHandle<()>) {
    if let Ok(mut tasks) = flow_tasks.lock() {
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle);
    } else {
        handle.abort();
    }
}

async fn abort_flow_tasks(flow_tasks: &FlowTasks) {
    let tasks = flow_tasks
        .lock()
        .map(|mut tasks| tasks.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
}

async fn handle_tcp_stream<S>(
    tunnel: Tunnel,
    stream: S,
    src_addr: SocketAddr,
    dst_addr: SocketAddr,
    dns_table: DnsTable,
    sniffer: Option<Arc<HarmonyTcpSniffer>>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let mut metadata = tcp_metadata_for_stream(src_addr, dst_addr, &dns_table);
    let mut stream = stream;
    let prefix = match sniffer {
        Some(sniffer) => sniffer.sniff(&mut stream, &mut metadata).await,
        None => Vec::new(),
    };
    let proxy_conn: Box<dyn ProxyConn> = Box::new(ReplayConn::new(stream, prefix));
    let inner = tunnel.inner().clone();
    meow_tunnel::tcp::handle_tcp(&inner, proxy_conn, metadata).await;
}

fn tcp_metadata_for_stream(
    src_addr: SocketAddr,
    dst_addr: SocketAddr,
    dns_table: &DnsTable,
) -> Metadata {
    let (host, dst_ip) = match dns_table.lookup(dst_addr.ip()) {
        Some(host) => (host, None),
        None => (String::new(), Some(dst_addr.ip())),
    };
    Metadata {
        network: Network::Tcp,
        conn_type: ConnType::Inner,
        src_ip: Some(src_addr.ip()),
        src_port: src_addr.port(),
        dst_ip,
        dst_port: dst_addr.port(),
        host: host.into(),
        in_name: "hmeta-vpn".into(),
        in_port: 0,
        ..Metadata::default()
    }
}

#[derive(Debug, Clone, Copy)]
enum SniffProtocol {
    Tls,
    Http,
}

struct HarmonyTcpSniffer {
    config: SnifferConfig,
    runtime: SnifferRuntime,
    force_domains: DomainTrie<()>,
}

impl HarmonyTcpSniffer {
    fn from_config(config: SnifferConfig) -> Option<Self> {
        if !config.enable {
            return None;
        }
        let mut force_domains = DomainTrie::new();
        for domain in &config.force_domain {
            force_domains.insert(domain, ());
        }
        Some(Self {
            runtime: SnifferRuntime::new(config.clone()),
            config,
            force_domains,
        })
    }

    fn protocol_for(&self, port: u16) -> Option<SniffProtocol> {
        // Match meow's dispatch precedence when a port appears in both lists:
        // HTTP is inserted after TLS and therefore wins.
        if self.config.http_ports.contains(&port) {
            Some(SniffProtocol::Http)
        } else if self.config.tls_ports.contains(&port) {
            Some(SniffProtocol::Tls)
        } else {
            None
        }
    }

    fn should_sniff(&self, metadata: &Metadata) -> bool {
        if !self.config.parse_pure_ip || metadata.host.is_empty() {
            return true;
        }
        metadata.host.parse::<IpAddr>().is_ok()
            || self.force_domains.search(&metadata.host).is_some()
    }

    async fn sniff<S>(&self, stream: &mut S, metadata: &mut Metadata) -> Vec<u8>
    where
        S: AsyncRead + Unpin,
    {
        let Some(protocol) = self.protocol_for(metadata.dst_port) else {
            return Vec::new();
        };
        if !self.should_sniff(metadata) {
            return Vec::new();
        }

        let mut prefix = vec![0_u8; SNIFF_BUFFER_SIZE];
        let Ok(Ok(size)) =
            tokio::time::timeout(self.config.timeout, stream.read(&mut prefix)).await
        else {
            return Vec::new();
        };
        prefix.truncate(size);
        let host = match protocol {
            SniffProtocol::Tls => sniff_tls(&prefix),
            SniffProtocol::Http => sniff_http(&prefix),
        };
        if let Some(host) = host {
            self.runtime.maybe_apply_sniff(&host, metadata);
        }
        prefix
    }
}

struct ReplayConn<S> {
    stream: S,
    prefix: Vec<u8>,
    prefix_offset: usize,
}

impl<S> ReplayConn<S> {
    fn new(stream: S, prefix: Vec<u8>) -> Self {
        Self {
            stream,
            prefix,
            prefix_offset: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ReplayConn<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.prefix_offset < self.prefix.len() && buf.remaining() > 0 {
            let available = &self.prefix[self.prefix_offset..];
            let size = available.len().min(buf.remaining());
            buf.put_slice(&available[..size]);
            self.prefix_offset += size;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ReplayConn<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl<S> ProxyConn for ReplayConn<S> where S: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UdpSessionKey {
    local: SocketAddr,
    remote: SocketAddr,
}

#[derive(Clone)]
struct UdpTunSession {
    conn: Arc<dyn ProxyPacketConn>,
    last_activity_ms: Arc<AtomicU64>,
}

impl UdpTunSession {
    fn new(conn: Arc<dyn ProxyPacketConn>) -> Self {
        Self {
            conn,
            last_activity_ms: Arc::new(AtomicU64::new(monotonic_ms())),
        }
    }

    fn touch(&self) {
        self.last_activity_ms
            .store(monotonic_ms(), Ordering::Relaxed);
    }

    fn idle_for(&self) -> Duration {
        let last = self.last_activity_ms.load(Ordering::Relaxed);
        Duration::from_millis(monotonic_ms().saturating_sub(last))
    }
}

#[derive(Clone, Default)]
struct UdpSessionMap {
    sessions: Arc<Mutex<HashMap<UdpSessionKey, UdpTunSession>>>,
}

impl UdpSessionMap {
    fn get(&self, key: &UdpSessionKey) -> Option<UdpTunSession> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(key).cloned())
    }

    fn insert(&self, key: UdpSessionKey, session: UdpTunSession) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(key, session);
        }
    }

    fn remove(&self, key: &UdpSessionKey) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(key);
        }
    }

    fn retain_active(&self, idle_timeout: Duration) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|_, session| session.idle_for() < idle_timeout);
        }
    }
}

struct UdpReply {
    data: Vec<u8>,
    local: SocketAddr,
    remote: SocketAddr,
}

async fn handle_udp_datagram(
    tunnel: Tunnel,
    dns_table: DnsTable,
    sessions: UdpSessionMap,
    reply_tx: mpsc::UnboundedSender<UdpReply>,
    flow_tasks: FlowTasks,
    data: Vec<u8>,
    local: SocketAddr,
    remote: SocketAddr,
) {
    let key = UdpSessionKey { local, remote };
    if let Some(session) = sessions.get(&key) {
        session.touch();
        if session.conn.write_packet(&data, &remote).await.is_err() {
            sessions.remove(&key);
        }
        return;
    }

    let mut metadata = udp_metadata_for_datagram(local, remote, &dns_table);
    let inner = tunnel.inner().clone();
    inner.pre_resolve(&mut metadata).await;

    let Some(dst_ip) = metadata.dst_ip else {
        return;
    };
    let dst_addr = SocketAddr::new(dst_ip, metadata.dst_port);
    let Some((proxy, _rule_name, _rule_payload)) = inner.resolve_proxy(&metadata) else {
        return;
    };
    let Ok(conn) = proxy.dial_udp(&metadata).await else {
        return;
    };
    if conn.write_packet(&data, &dst_addr).await.is_err() {
        return;
    }

    let conn: Arc<dyn ProxyPacketConn> = Arc::from(conn);
    let session = UdpTunSession::new(conn.clone());
    sessions.insert(key, session);

    let handle = tokio::spawn(async move {
        read_udp_responses(key, local, conn, sessions, reply_tx).await;
    });
    track_flow_task(&flow_tasks, handle);
}

fn udp_metadata_for_datagram(
    local: SocketAddr,
    remote: SocketAddr,
    dns_table: &DnsTable,
) -> Metadata {
    Metadata {
        network: Network::Udp,
        conn_type: ConnType::Inner,
        src_ip: Some(local.ip()),
        src_port: local.port(),
        dst_ip: Some(remote.ip()),
        dst_port: remote.port(),
        host: dns_table.lookup(remote.ip()).unwrap_or_default().into(),
        in_name: "hmeta-vpn".into(),
        in_port: 0,
        ..Metadata::default()
    }
}

async fn read_udp_responses(
    key: UdpSessionKey,
    local: SocketAddr,
    conn: Arc<dyn ProxyPacketConn>,
    sessions: UdpSessionMap,
    reply_tx: mpsc::UnboundedSender<UdpReply>,
) {
    let mut buffer = vec![0_u8; UDP_RESPONSE_BUFFER_SIZE];
    loop {
        match conn.read_packet(&mut buffer).await {
            Ok((size, remote)) if size > 0 => {
                if let Some(session) = sessions.get(&key) {
                    session.touch();
                }
                let _ = reply_tx.send(UdpReply {
                    data: buffer[..size].to_vec(),
                    local,
                    remote,
                });
            }
            Ok(_) => {}
            Err(_) => {
                sessions.remove(&key);
                return;
            }
        }
    }
}

fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[derive(Debug, Clone, Default)]
struct DnsTable {
    records: Arc<Mutex<HashMap<IpAddr, DnsTableRecord>>>,
}

#[derive(Debug, Clone)]
struct DnsTableRecord {
    host: String,
    expires_at_ms: u64,
}

impl DnsTable {
    fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }

    fn insert(&self, ip: IpAddr, host: String, ttl: u32) {
        let ttl_ms = u64::from(ttl.clamp(1, 3600)) * 1000;
        if let Ok(mut records) = self.records.lock() {
            prune_expired_dns_table_records(&mut records);
            if !records.contains_key(&ip) && records.len() >= DNS_TABLE_MAX_RECORDS {
                evict_earliest_dns_table_record(&mut records);
            }
            records.insert(
                ip,
                DnsTableRecord {
                    host,
                    expires_at_ms: monotonic_ms().saturating_add(ttl_ms),
                },
            );
        }
    }

    fn lookup(&self, ip: IpAddr) -> Option<String> {
        let mut records = self.records.lock().ok()?;
        let record = records.get(&ip)?;
        if record.expires_at_ms <= monotonic_ms() {
            records.remove(&ip);
            None
        } else {
            Some(record.host.clone())
        }
    }
}

fn prune_expired_dns_table_records(records: &mut HashMap<IpAddr, DnsTableRecord>) {
    let now = monotonic_ms();
    records.retain(|_, record| record.expires_at_ms > now);
}

fn evict_earliest_dns_table_record(records: &mut HashMap<IpAddr, DnsTableRecord>) {
    if let Some(ip) = records
        .iter()
        .min_by_key(|(_, record)| record.expires_at_ms)
        .map(|(ip, _)| *ip)
    {
        records.remove(&ip);
    }
}

#[derive(Debug, Clone, Default)]
struct DnsResponseCache {
    records: Arc<Mutex<HashMap<Vec<u8>, DnsResponseCacheRecord>>>,
}

#[derive(Debug, Clone)]
struct DnsResponseCacheRecord {
    response: Vec<u8>,
    expires_at_ms: u64,
}

impl DnsResponseCache {
    fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }

    fn lookup(&self, query: &[u8]) -> Option<Vec<u8>> {
        let key = dns_cache_key(query)?;
        let mut records = self.records.lock().ok()?;
        let now = monotonic_ms();
        let record = records.get(&key)?;
        if record.expires_at_ms <= now {
            records.remove(&key);
            return None;
        }
        let mut response = record.response.clone();
        if response.len() >= 2 && query.len() >= 2 {
            response[0..2].copy_from_slice(&query[0..2]);
        }
        rewrite_dns_response_question(&mut response, query);
        let remaining_ttl = record.expires_at_ms.saturating_sub(now).saturating_add(999) / 1000;
        rewrite_dns_response_ttls(
            &mut response,
            remaining_ttl.clamp(1, u64::from(u32::MAX)) as u32,
        );
        Some(response)
    }

    fn insert(&self, query: &[u8], response: &[u8], records: &[(IpAddr, String, u32)]) {
        if records.is_empty() {
            return;
        }
        let Some(key) = dns_cache_key(query) else {
            return;
        };
        let ttl = records
            .iter()
            .map(|(_, _, ttl)| *ttl)
            .min()
            .unwrap_or(0)
            .clamp(1, DNS_CACHE_MAX_TTL);
        if let Ok(mut cache_records) = self.records.lock() {
            prune_expired_dns_cache_records(&mut cache_records);
            if !cache_records.contains_key(&key) && cache_records.len() >= DNS_CACHE_MAX_RECORDS {
                evict_earliest_dns_cache_record(&mut cache_records);
            }
            cache_records.insert(
                key,
                DnsResponseCacheRecord {
                    response: response.to_vec(),
                    expires_at_ms: monotonic_ms().saturating_add(u64::from(ttl) * 1000),
                },
            );
        }
    }
}

fn prune_expired_dns_cache_records(records: &mut HashMap<Vec<u8>, DnsResponseCacheRecord>) {
    let now = monotonic_ms();
    records.retain(|_, record| record.expires_at_ms > now);
}

fn evict_earliest_dns_cache_record(records: &mut HashMap<Vec<u8>, DnsResponseCacheRecord>) {
    if let Some(key) = records
        .iter()
        .min_by_key(|(_, record)| record.expires_at_ms)
        .map(|(key, _)| key.clone())
    {
        records.remove(&key);
    }
}

async fn handle_dns_query(
    tunnel: Tunnel,
    dns_table: DnsTable,
    dns_cache: DnsResponseCache,
    stats: Arc<SharedStats>,
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    query: Vec<u8>,
    reply_tx: mpsc::UnboundedSender<Vec<u8>>,
) {
    if let Some(response) = dns_cache.lookup(&query) {
        stats.dns_cache_hits.fetch_add(1, Ordering::Relaxed);
        for (ip, host, ttl) in parse_dns_response_records(&response) {
            dns_table.insert(ip, host, ttl);
        }
        let _ = reply_tx.send(build_udp_packet(
            dst_ip, dst_port, src_ip, src_port, &response,
        ));
        return;
    }
    if dns_cache_key(&query).is_some() {
        stats.dns_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    let response = match meow_dns::DnsServer::handle_query(&query, tunnel.resolver()).await {
        Ok(response) => response,
        Err(_) => build_dns_error_response(&query, DnsResponseCode::ServFail),
    };
    let response_records = parse_dns_response_records(&response);
    dns_cache.insert(&query, &response, &response_records);
    for (ip, host, ttl) in response_records {
        dns_table.insert(ip, host, ttl);
    }
    let _ = reply_tx.send(build_udp_packet(
        dst_ip, dst_port, src_ip, src_port, &response,
    ));
}

fn dns_cache_key(query: &[u8]) -> Option<Vec<u8>> {
    let question = parse_dns_query(query)?;
    let mut key = Vec::with_capacity(question.name.len() + 2);
    key.extend_from_slice(question.name.to_ascii_lowercase().as_bytes());
    key.push(0);
    key.push(match question.kind {
        DnsRecordKind::A => 1,
        DnsRecordKind::Aaaa => 28,
    });
    Some(key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DnsQuery {
    name: String,
    question_end: usize,
    kind: DnsRecordKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsRecordKind {
    A,
    Aaaa,
}

impl DnsRecordKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsResponseCode {
    ServFail = 2,
}

fn parse_dns_query(query: &[u8]) -> Option<DnsQuery> {
    if query.len() < 12 {
        return None;
    }
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return None;
    }
    let mut offset = 12;
    let mut labels = Vec::new();
    loop {
        let len = *query.get(offset)? as usize;
        offset += 1;
        if len == 0 {
            break;
        }
        if len & 0xc0 != 0 || offset + len > query.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&query[offset..offset + len]).ok()?);
        offset += len;
    }
    if offset + 4 > query.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([query[offset], query[offset + 1]]);
    let qclass = u16::from_be_bytes([query[offset + 2], query[offset + 3]]);
    if qclass != 1 {
        return None;
    }
    let kind = match qtype {
        1 => DnsRecordKind::A,
        28 => DnsRecordKind::Aaaa,
        _ => return None,
    };
    Some(DnsQuery {
        name: labels.join("."),
        question_end: offset + 4,
        kind,
    })
}

fn parse_dns_response_records(response: &[u8]) -> Vec<(IpAddr, String, u32)> {
    if response.len() < 12 {
        return Vec::new();
    }
    let qdcount = u16::from_be_bytes([response[4], response[5]]) as usize;
    let ancount = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    let mut question_name = String::new();
    for question_index in 0..qdcount {
        let Some((name, next_offset)) = read_dns_name(response, offset) else {
            return Vec::new();
        };
        if question_index == 0 {
            question_name = name;
        }
        if next_offset + 4 > response.len() {
            return Vec::new();
        }
        offset = next_offset + 4;
    }

    let mut answers = Vec::new();
    for _ in 0..ancount {
        let Some((name, next_offset)) = read_dns_name(response, offset) else {
            break;
        };
        if next_offset + 10 > response.len() {
            break;
        }
        let record_type = u16::from_be_bytes([response[next_offset], response[next_offset + 1]]);
        let record_class =
            u16::from_be_bytes([response[next_offset + 2], response[next_offset + 3]]);
        let ttl = u32::from_be_bytes([
            response[next_offset + 4],
            response[next_offset + 5],
            response[next_offset + 6],
            response[next_offset + 7],
        ]);
        let rdlen =
            u16::from_be_bytes([response[next_offset + 8], response[next_offset + 9]]) as usize;
        let rdata_offset = next_offset + 10;
        let next_record = rdata_offset + rdlen;
        if next_record > response.len() {
            break;
        }
        let host = if name.is_empty() {
            question_name.clone()
        } else {
            name
        };
        if record_class == 1 && record_type == 1 && rdlen == 4 {
            answers.push(DnsAnswer::Address {
                host,
                ip: IpAddr::V4(Ipv4Addr::new(
                    response[rdata_offset],
                    response[rdata_offset + 1],
                    response[rdata_offset + 2],
                    response[rdata_offset + 3],
                )),
                ttl,
            });
        } else if record_class == 1 && record_type == 28 && rdlen == 16 {
            let mut octets = [0_u8; 16];
            octets.copy_from_slice(&response[rdata_offset..rdata_offset + 16]);
            answers.push(DnsAnswer::Address {
                host,
                ip: IpAddr::V6(octets.into()),
                ttl,
            });
        } else if record_class == 1 && record_type == 5 {
            if let Some((target, _)) =
                read_dns_name(response, rdata_offset).filter(|(target, _)| !target.is_empty())
            {
                answers.push(DnsAnswer::Cname { host, target });
            }
        }
        offset = next_record;
    }
    dns_records_from_answers(question_name, answers)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnsAnswer {
    Address { host: String, ip: IpAddr, ttl: u32 },
    Cname { host: String, target: String },
}

fn dns_records_from_answers(
    question_name: String,
    answers: Vec<DnsAnswer>,
) -> Vec<(IpAddr, String, u32)> {
    let mut cname_sources = HashMap::new();
    for answer in &answers {
        let DnsAnswer::Cname { host, target } = answer else {
            continue;
        };
        if host.is_empty() || target.is_empty() {
            continue;
        }
        cname_sources.insert(target.to_ascii_lowercase(), host.clone());
    }

    answers
        .into_iter()
        .filter_map(|answer| match answer {
            DnsAnswer::Address { host, ip, ttl } => Some((
                ip,
                dns_response_host_for_address(&host, &question_name, &cname_sources),
                ttl,
            )),
            DnsAnswer::Cname { .. } => None,
        })
        .collect()
}

fn dns_response_host_for_address(
    host: &str,
    question_name: &str,
    cname_sources: &HashMap<String, String>,
) -> String {
    let mut current = if host.is_empty() {
        question_name.to_owned()
    } else {
        host.to_owned()
    };
    for _ in 0..8 {
        let Some(source) = cname_sources.get(&current.to_ascii_lowercase()) else {
            break;
        };
        current = source.clone();
    }
    if current.is_empty() {
        question_name.to_owned()
    } else {
        current
    }
}

fn rewrite_dns_response_question(response: &mut [u8], query: &[u8]) {
    let Some(query_question_end) = parse_dns_question_end(query) else {
        return;
    };
    let Some(response_question_end) = parse_dns_question_end(response) else {
        return;
    };
    if query_question_end != response_question_end || query_question_end > response.len() {
        return;
    }
    response[12..query_question_end].copy_from_slice(&query[12..query_question_end]);
}

fn rewrite_dns_response_ttls(response: &mut [u8], ttl: u32) {
    if response.len() < 12 {
        return;
    }
    let qdcount = u16::from_be_bytes([response[4], response[5]]) as usize;
    let ancount = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    for _ in 0..qdcount {
        let Some((_, next_offset)) = read_dns_name(response, offset) else {
            return;
        };
        if next_offset + 4 > response.len() {
            return;
        }
        offset = next_offset + 4;
    }

    for _ in 0..ancount {
        let Some((_, next_offset)) = read_dns_name(response, offset) else {
            return;
        };
        if next_offset + 10 > response.len() {
            return;
        }
        response[next_offset + 4..next_offset + 8].copy_from_slice(&ttl.to_be_bytes());
        let rdlen =
            u16::from_be_bytes([response[next_offset + 8], response[next_offset + 9]]) as usize;
        let next_record = next_offset + 10 + rdlen;
        if next_record > response.len() {
            return;
        }
        offset = next_record;
    }
}

fn read_dns_name(packet: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut pos = offset;
    let mut next_offset = offset;
    let mut jumped = false;
    let mut jumps = 0_u8;
    loop {
        let len = *packet.get(pos)?;
        if len & 0xc0 == 0xc0 {
            let second = *packet.get(pos + 1)?;
            let ptr = (usize::from(len & 0x3f) << 8) | usize::from(second);
            if ptr >= packet.len() {
                return None;
            }
            if !jumped {
                next_offset = pos + 2;
            }
            pos = ptr;
            jumped = true;
            jumps = jumps.saturating_add(1);
            if jumps > 8 {
                return None;
            }
            continue;
        }
        if len & 0xc0 != 0 {
            return None;
        }
        pos += 1;
        if len == 0 {
            if !jumped {
                next_offset = pos;
            }
            break;
        }
        let len = usize::from(len);
        if pos + len > packet.len() {
            return None;
        }
        labels.push(
            std::str::from_utf8(&packet[pos..pos + len])
                .ok()?
                .to_owned(),
        );
        pos += len;
    }
    Some((labels.join("."), next_offset))
}

fn build_dns_error_response(query: &[u8], code: DnsResponseCode) -> Vec<u8> {
    if query.len() < 12 {
        return Vec::new();
    }
    let question_end = parse_dns_question_end(query).unwrap_or(query.len().min(12));
    let mut response = Vec::with_capacity(question_end);
    response.extend_from_slice(&query[..question_end]);
    response[2] = 0x81;
    response[3] = 0x80 | code as u8;
    response[6..8].copy_from_slice(&0_u16.to_be_bytes());
    response[8..10].copy_from_slice(&0_u16.to_be_bytes());
    response[10..12].copy_from_slice(&0_u16.to_be_bytes());
    response
}

fn build_dns_servfail_udp_packet(
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    query: &[u8],
) -> Option<Vec<u8>> {
    let response = build_dns_error_response(query, DnsResponseCode::ServFail);
    if response.is_empty() {
        None
    } else {
        Some(build_udp_packet(
            dst_ip, dst_port, src_ip, src_port, &response,
        ))
    }
}

fn parse_dns_question_end(query: &[u8]) -> Option<usize> {
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return Some(12);
    }
    let (_, offset) = read_dns_name(query, 12)?;
    (offset + 4 <= query.len()).then_some(offset + 4)
}

#[cfg(test)]
fn build_dns_response(query: &[u8], request: &DnsQuery, ip: IpAddr) -> Vec<u8> {
    let mut response = Vec::with_capacity(query.len() + 32);
    response.extend_from_slice(&query[..request.question_end]);
    response[2] = 0x81;
    response[3] = 0x80;
    response[6..8].copy_from_slice(&1_u16.to_be_bytes());
    response[8..10].copy_from_slice(&0_u16.to_be_bytes());
    response[10..12].copy_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&[0xc0, 0x0c]);
    match ip {
        IpAddr::V4(ip) => {
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&4_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            response.extend_from_slice(&28_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&16_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
    }
    response
}

fn parse_udp_packet(ip_data: &[u8]) -> Option<(u32, u16, u32, u16, &[u8])> {
    if ip_data.len() < 28 || (ip_data[0] >> 4) != 4 || ip_data[9] != 17 {
        return None;
    }
    let ihl = usize::from(ip_data[0] & 0x0f) * 4;
    if ihl < 20 || ip_data.len() < ihl + 8 {
        return None;
    }
    let total_len = u16::from_be_bytes([ip_data[2], ip_data[3]]) as usize;
    if total_len < ihl + 8 || total_len > ip_data.len() {
        return None;
    }
    let fragment_field = u16::from_be_bytes([ip_data[6], ip_data[7]]);
    if fragment_field & 0x3fff != 0 {
        return None;
    }
    let src_ip = u32::from_ne_bytes([ip_data[12], ip_data[13], ip_data[14], ip_data[15]]);
    let dst_ip = u32::from_ne_bytes([ip_data[16], ip_data[17], ip_data[18], ip_data[19]]);
    let src_port = u16::from_be_bytes([ip_data[ihl], ip_data[ihl + 1]]);
    let dst_port = u16::from_be_bytes([ip_data[ihl + 2], ip_data[ihl + 3]]);
    let udp_len = u16::from_be_bytes([ip_data[ihl + 4], ip_data[ihl + 5]]) as usize;
    if udp_len < 8 || ihl + udp_len > total_len {
        return None;
    }
    let start = ihl + 8;
    let end = ihl + udp_len;
    (start <= end).then_some((src_ip, src_port, dst_ip, dst_port, &ip_data[start..end]))
}

fn tun_dns_query_from_packet(
    dns_hijacking: bool,
    ip_data: &[u8],
) -> Option<(u32, u16, u32, u16, &[u8])> {
    if !dns_hijacking {
        return None;
    }
    let packet = parse_udp_packet(ip_data)?;
    (packet.3 == 53).then_some(packet)
}

fn build_udp_packet(
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total_len = 20 + udp_len;
    let mut packet = vec![0_u8; total_len];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    packet[6] = 0x40;
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&src_ip.to_ne_bytes());
    packet[16..20].copy_from_slice(&dst_ip.to_ne_bytes());
    let checksum = ip_checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());
    packet[20..22].copy_from_slice(&src_port.to_be_bytes());
    packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
    packet[24..26].copy_from_slice(&(udp_len as u16).to_be_bytes());
    packet[28..].copy_from_slice(payload);
    packet
}

fn ip_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for i in (0..header.len()).step_by(2) {
        sum += if i + 1 < header.len() {
            (u32::from(header[i]) << 8) | u32::from(header[i + 1])
        } else {
            u32::from(header[i]) << 8
        };
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !sum as u16
}

async fn write_tun_packet(fd: RawFd, pkt: &[u8]) -> bool {
    let mut retries = 0_u8;
    loop {
        let written = unsafe { libc::write(fd, pkt.as_ptr() as *const c_void, pkt.len()) };
        if written >= 0 {
            return true;
        }
        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EAGAIN && retries < 3 {
            retries += 1;
            tokio::task::yield_now().await;
            continue;
        }
        return false;
    }
}

fn duplicate_fd(fd: i32) -> Result<i32, HMetaError> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        Err(HMetaError::Io(std::io::Error::last_os_error().to_string()))
    } else {
        Ok(duplicated)
    }
}

fn set_nonblocking(fd: RawFd) -> Result<(), HMetaError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(HMetaError::Io(io::Error::last_os_error().to_string()));
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        Err(HMetaError::Io(io::Error::last_os_error().to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meow_common::{DnsMode, MeowError, TunnelMode};
    use meow_dns::Resolver;
    use meow_trie::DomainTrie;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use tokio::io::AsyncWriteExt;

    fn tls_client_hello(hostname: &str) -> Vec<u8> {
        let hostname = hostname.as_bytes();
        let mut sni = Vec::new();
        sni.extend_from_slice(&[0x00, 0x00]);
        sni.extend_from_slice(&((5 + hostname.len()) as u16).to_be_bytes());
        sni.extend_from_slice(&((3 + hostname.len()) as u16).to_be_bytes());
        sni.push(0);
        sni.extend_from_slice(&(hostname.len() as u16).to_be_bytes());
        sni.extend_from_slice(hostname);

        let mut hello = Vec::new();
        hello.extend_from_slice(&[0x03, 0x03]);
        hello.extend_from_slice(&[0; 32]);
        hello.push(0);
        hello.extend_from_slice(&[0, 2, 0, 0x2f]);
        hello.extend_from_slice(&[1, 0]);
        hello.extend_from_slice(&(sni.len() as u16).to_be_bytes());
        hello.extend_from_slice(&sni);

        let mut handshake = vec![
            1,
            ((hello.len() >> 16) & 0xff) as u8,
            ((hello.len() >> 8) & 0xff) as u8,
            (hello.len() & 0xff) as u8,
        ];
        handshake.extend_from_slice(&hello);

        let mut record = vec![0x16, 0x03, 0x01];
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    #[tokio::test]
    async fn harmony_sniffer_extracts_tls_sni_and_replays_client_hello() {
        let sniffer = HarmonyTcpSniffer::from_config(SnifferConfig {
            enable: true,
            override_destination: true,
            tls_ports: vec![443],
            http_ports: Vec::new(),
            ..SnifferConfig::default()
        })
        .expect("enabled sniffer");
        let payload = tls_client_hello("tls.example.test");
        let (mut client, mut server) = tokio::io::duplex(SNIFF_BUFFER_SIZE * 2);
        client.write_all(&payload).await.unwrap();
        client.shutdown().await.unwrap();
        let mut metadata = Metadata {
            network: Network::Tcp,
            dst_ip: Some(Ipv4Addr::new(203, 0, 113, 1).into()),
            dst_port: 443,
            ..Metadata::default()
        };

        let prefix = sniffer.sniff(&mut server, &mut metadata).await;
        let mut replay = ReplayConn::new(server, prefix);
        let mut received = Vec::new();
        replay.read_to_end(&mut received).await.unwrap();

        assert_eq!(metadata.sniff_host, "tls.example.test");
        assert_eq!(metadata.host, "tls.example.test");
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn harmony_sniffer_extracts_http_host_without_overriding_destination() {
        let sniffer = HarmonyTcpSniffer::from_config(SnifferConfig {
            enable: true,
            override_destination: false,
            tls_ports: Vec::new(),
            http_ports: vec![80],
            ..SnifferConfig::default()
        })
        .expect("enabled sniffer");
        let payload = b"GET / HTTP/1.1\r\nHost: http.example.test\r\n\r\n".to_vec();
        let (mut client, mut server) = tokio::io::duplex(SNIFF_BUFFER_SIZE * 2);
        client.write_all(&payload).await.unwrap();
        client.shutdown().await.unwrap();
        let mut metadata = Metadata {
            network: Network::Tcp,
            dst_port: 80,
            host: "203.0.113.2".into(),
            ..Metadata::default()
        };

        let prefix = sniffer.sniff(&mut server, &mut metadata).await;
        let mut replay = ReplayConn::new(server, prefix);
        let mut received = Vec::new();
        replay.read_to_end(&mut received).await.unwrap();

        assert_eq!(metadata.sniff_host, "http.example.test");
        assert_eq!(metadata.host, "203.0.113.2");
        assert_eq!(received, payload);
    }

    struct NoopPacketConn;

    #[async_trait::async_trait]
    impl ProxyPacketConn for NoopPacketConn {
        async fn read_packet(&self, _buf: &mut [u8]) -> meow_common::Result<(usize, SocketAddr)> {
            Err(MeowError::Other("noop packet conn closed".to_owned()))
        }

        async fn write_packet(&self, buf: &[u8], _addr: &SocketAddr) -> meow_common::Result<usize> {
            Ok(buf.len())
        }

        fn local_addr(&self) -> meow_common::Result<SocketAddr> {
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        }

        fn close(&self) -> meow_common::Result<()> {
            Ok(())
        }
    }

    enum ScriptedPacketRead {
        Packet(Vec<u8>, SocketAddr),
        Error(&'static str),
    }

    struct ScriptedPacketConn {
        reads: Mutex<VecDeque<ScriptedPacketRead>>,
    }

    impl ScriptedPacketConn {
        fn new(reads: Vec<ScriptedPacketRead>) -> Self {
            Self {
                reads: Mutex::new(reads.into()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProxyPacketConn for ScriptedPacketConn {
        async fn read_packet(&self, buf: &mut [u8]) -> meow_common::Result<(usize, SocketAddr)> {
            let read = self
                .reads
                .lock()
                .expect("scripted reads lock")
                .pop_front()
                .unwrap_or(ScriptedPacketRead::Error("script exhausted"));
            match read {
                ScriptedPacketRead::Packet(data, addr) => {
                    let size = data.len().min(buf.len());
                    buf[..size].copy_from_slice(&data[..size]);
                    Ok((size, addr))
                }
                ScriptedPacketRead::Error(message) => Err(MeowError::Other(message.to_owned())),
            }
        }

        async fn write_packet(&self, buf: &[u8], _addr: &SocketAddr) -> meow_common::Result<usize> {
            Ok(buf.len())
        }

        fn local_addr(&self) -> meow_common::Result<SocketAddr> {
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        }

        fn close(&self) -> meow_common::Result<()> {
            Ok(())
        }
    }

    fn direct_mode_tunnel() -> Tunnel {
        let resolver = Arc::new(Resolver::new(
            Vec::new(),
            Vec::new(),
            DnsMode::Normal,
            DomainTrie::new(),
            true,
        ));
        let tunnel = Tunnel::new(resolver);
        tunnel.set_mode(TunnelMode::Direct);
        tunnel
    }

    #[test]
    fn duplicate_fd_rejects_invalid_fd() {
        assert!(duplicate_fd(-1).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lwip_stack_accepts_and_replies_to_tun_udp_packets() {
        let (mut stack, _tcp_listener, udp_socket) =
            lwip::NetStack::with_buffer_size(16, 16).expect("lwip stack");
        let (udp_write, mut udp_read) = udp_socket.split();
        let local = SocketAddr::new(Ipv4Addr::new(172, 19, 0, 1).into(), 40123);
        let remote = SocketAddr::new(Ipv4Addr::new(203, 0, 113, 42).into(), 443);
        let packet = build_udp_packet(
            u32::from_ne_bytes([172, 19, 0, 1]),
            local.port(),
            u32::from_ne_bytes([203, 0, 113, 42]),
            remote.port(),
            b"request",
        );

        stack.send(packet).await.expect("send packet into lwip");
        let (payload, src, dst) = tokio::time::timeout(Duration::from_secs(1), udp_read.next())
            .await
            .expect("lwip UDP receive timeout")
            .expect("lwip UDP receive");
        assert_eq!(payload, b"request");
        assert_eq!(src, local);
        assert_eq!(dst, remote);

        udp_write
            .send_to(b"response", &remote, &local)
            .expect("send lwip UDP response");
        let response = tokio::time::timeout(Duration::from_secs(1), stack.next())
            .await
            .expect("lwip egress timeout")
            .expect("lwip egress frame")
            .expect("lwip egress packet");
        let parsed = parse_udp_packet(&response).expect("parse lwip UDP response");
        assert_eq!(parsed.1, remote.port());
        assert_eq!(parsed.3, local.port());
        assert_eq!(parsed.4, b"response");
    }

    #[test]
    fn udp_parser_classifies_ipv4_dns() {
        let packet = [
            0x45, 0, 0, 28, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 2, 1, 1, 1, 1, 0x12, 0x34, 0, 53,
            0, 8, 0, 0,
        ];
        let parsed = parse_udp_packet(&packet).expect("udp packet");
        assert_eq!(parsed.1, 0x1234);
        assert_eq!(parsed.3, 53);
    }

    #[test]
    fn udp_parser_rejects_truncated_payload() {
        let mut packet = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        packet.truncate(packet.len() - 1);

        assert!(parse_udp_packet(&packet).is_none());
    }

    #[test]
    fn udp_parser_rejects_invalid_ipv4_lengths() {
        let mut short_ihl = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        short_ihl[0] = 0x44;
        assert!(parse_udp_packet(&short_ihl).is_none());

        let mut short_total_len = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        short_total_len[2..4].copy_from_slice(&24_u16.to_be_bytes());
        assert!(parse_udp_packet(&short_total_len).is_none());

        let mut oversized_total_len = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        oversized_total_len[2..4].copy_from_slice(&128_u16.to_be_bytes());
        assert!(parse_udp_packet(&oversized_total_len).is_none());
    }

    #[test]
    fn udp_parser_rejects_ipv4_fragments() {
        let mut more_fragments = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        more_fragments[6..8].copy_from_slice(&0x2000_u16.to_be_bytes());
        assert!(parse_udp_packet(&more_fragments).is_none());

        let mut fragment_offset = build_udp_packet(
            u32::from_ne_bytes([10, 0, 0, 2]),
            0x1234,
            u32::from_ne_bytes([1, 1, 1, 1]),
            53,
            b"abcd",
        );
        fragment_offset[6..8].copy_from_slice(&0x0001_u16.to_be_bytes());
        assert!(parse_udp_packet(&fragment_offset).is_none());
    }

    #[test]
    fn udp_packet_builder_roundtrips_payload_and_endpoints() {
        let src_ip = u32::from_ne_bytes([172, 19, 0, 2]);
        let dst_ip = u32::from_ne_bytes([203, 0, 113, 42]);
        let payload = b"udp-session-payload";

        let packet = build_udp_packet(src_ip, 40123, dst_ip, 443, payload);
        let parsed = parse_udp_packet(&packet).expect("built UDP packet parses");

        assert_eq!(parsed.0, src_ip);
        assert_eq!(parsed.1, 40123);
        assert_eq!(parsed.2, dst_ip);
        assert_eq!(parsed.3, 443);
        assert_eq!(parsed.4, payload);
    }

    #[test]
    fn dns_hijack_respects_vpn_option() {
        let packet = build_udp_packet(
            u32::from_ne_bytes([172, 19, 0, 2]),
            40123,
            u32::from_ne_bytes([172, 19, 0, 2]),
            53,
            b"dns-query",
        );

        let hijacked = tun_dns_query_from_packet(true, &packet).expect("dns packet");
        assert_eq!(hijacked.1, 40123);
        assert_eq!(hijacked.3, 53);
        assert_eq!(hijacked.4, b"dns-query");
        assert!(tun_dns_query_from_packet(false, &packet).is_none());

        let non_dns = build_udp_packet(
            u32::from_ne_bytes([172, 19, 0, 2]),
            40123,
            u32::from_ne_bytes([203, 0, 113, 42]),
            443,
            b"udp",
        );
        assert!(tun_dns_query_from_packet(true, &non_dns).is_none());
    }

    #[test]
    fn dns_query_and_response_roundtrip() {
        let query = dns_query("example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        assert_eq!(parsed.name, "example.test");
        assert_eq!(parsed.kind, DnsRecordKind::A);
        let response = build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
        assert_eq!(&response[0..2], &query[0..2]);
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
        assert_eq!(&response[response.len() - 4..], &[1, 2, 3, 4]);
        assert_eq!(
            parse_dns_response_records(&response),
            vec![(
                IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
                "example.test".to_owned(),
                60
            )]
        );
    }

    #[test]
    fn dns_aaaa_query_and_response_roundtrip() {
        let query = dns_query("ipv6.example.test", 28);
        let parsed = parse_dns_query(&query).expect("dns query");
        assert_eq!(parsed.name, "ipv6.example.test");
        assert_eq!(parsed.kind, DnsRecordKind::Aaaa);
        let ip = Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 42);
        let response = build_dns_response(&query, &parsed, IpAddr::V6(ip));
        assert_eq!(&response[0..2], &query[0..2]);
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
        assert_eq!(
            parse_dns_response_records(&response),
            vec![(IpAddr::V6(ip), "ipv6.example.test".to_owned(), 60)]
        );
    }

    #[test]
    fn dns_response_records_map_cname_addresses_to_original_question() {
        let query = dns_query("app.example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response = build_dns_cname_response(
            &query,
            &parsed,
            "edge.example.test",
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 88)),
        );

        assert_eq!(
            parse_dns_response_records(&response),
            vec![(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 88)),
                "app.example.test".to_owned(),
                60
            )]
        );
    }

    #[test]
    fn dns_error_response_preserves_question() {
        let query = dns_query("bad.test", 1);
        let response = build_dns_error_response(&query, DnsResponseCode::ServFail);
        assert_eq!(&response[0..2], &query[0..2]);
        assert_eq!(response[3] & 0x0f, DnsResponseCode::ServFail as u8);
        assert_eq!(&response[12..], &query[12..]);
    }

    #[test]
    fn dns_overflow_servfail_udp_packet_returns_to_client() {
        let query = dns_query("overflow.test", 1);
        let src_ip = u32::from_ne_bytes([172, 19, 0, 2]);
        let dst_ip = u32::from_ne_bytes([172, 19, 0, 1]);

        let packet = build_dns_servfail_udp_packet(src_ip, 40123, dst_ip, 53, &query)
            .expect("servfail packet");
        let parsed = parse_udp_packet(&packet).expect("servfail UDP packet");

        assert_eq!(parsed.0, dst_ip);
        assert_eq!(parsed.1, 53);
        assert_eq!(parsed.2, src_ip);
        assert_eq!(parsed.3, 40123);
        assert_eq!(&parsed.4[0..2], &query[0..2]);
        assert_eq!(parsed.4[3] & 0x0f, DnsResponseCode::ServFail as u8);
        assert_eq!(&parsed.4[12..], &query[12..]);
    }

    #[test]
    fn dns_overflow_servfail_udp_packet_ignores_invalid_queries() {
        assert!(build_dns_servfail_udp_packet(1, 53, 2, 40123, b"short dns").is_none());
    }

    #[test]
    fn dns_table_restores_host() {
        let table = DnsTable::default();
        table.insert(
            IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
            "example.test".to_owned(),
            60,
        );
        assert_eq!(
            table.lookup(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
            Some("example.test".to_owned())
        );
    }

    #[test]
    fn tcp_metadata_restores_host_from_dns_table() {
        let table = DnsTable::default();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
        let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 20));
        let remote = SocketAddr::new(remote_ip, 443);
        table.insert(remote_ip, "tcp.example.test".to_owned(), 60);

        let metadata = tcp_metadata_for_stream(local, remote, &table);

        assert_eq!(metadata.network, Network::Tcp);
        assert_eq!(metadata.src_ip, Some(local.ip()));
        assert_eq!(metadata.src_port, local.port());
        assert_eq!(metadata.dst_ip, None);
        assert_eq!(metadata.dst_port, remote.port());
        assert_eq!(metadata.host, "tcp.example.test");
        assert_eq!(metadata.in_name, "hmeta-vpn");
    }

    #[test]
    fn tcp_metadata_keeps_destination_ip_without_dns_table_host() {
        let table = DnsTable::default();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
        let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 21));
        let remote = SocketAddr::new(remote_ip, 443);

        let metadata = tcp_metadata_for_stream(local, remote, &table);

        assert_eq!(metadata.network, Network::Tcp);
        assert_eq!(metadata.dst_ip, Some(remote_ip));
        assert_eq!(metadata.dst_port, remote.port());
        assert!(metadata.host.is_empty());
    }

    #[test]
    fn udp_metadata_restores_host_from_dns_table() {
        let table = DnsTable::default();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
        let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
        let remote = SocketAddr::new(remote_ip, 443);
        table.insert(remote_ip, "udp.example.test".to_owned(), 60);

        let metadata = udp_metadata_for_datagram(local, remote, &table);

        assert_eq!(metadata.network, Network::Udp);
        assert_eq!(metadata.src_ip, Some(local.ip()));
        assert_eq!(metadata.src_port, local.port());
        assert_eq!(metadata.dst_ip, Some(remote_ip));
        assert_eq!(metadata.dst_port, remote.port());
        assert_eq!(metadata.host, "udp.example.test");
        assert_eq!(metadata.in_name, "hmeta-vpn");
    }

    #[test]
    fn udp_session_map_retains_touched_sessions_and_evicts_idle_ones() {
        let sessions = UdpSessionMap::default();
        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
        let active_remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), 443);
        let idle_remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 11)), 443);
        let active_key = UdpSessionKey {
            local,
            remote: active_remote,
        };
        let idle_key = UdpSessionKey {
            local,
            remote: idle_remote,
        };
        let active_session = UdpTunSession::new(Arc::new(NoopPacketConn));
        let idle_session = UdpTunSession::new(Arc::new(NoopPacketConn));
        sessions.insert(active_key, active_session.clone());
        sessions.insert(idle_key, idle_session.clone());

        std::thread::sleep(Duration::from_millis(20));
        active_session.touch();
        idle_session
            .last_activity_ms
            .store(monotonic_ms().saturating_sub(10), Ordering::Relaxed);
        sessions.retain_active(Duration::from_millis(5));

        assert!(sessions.get(&active_key).is_some());
        assert!(sessions.get(&idle_key).is_none());
    }

    #[test]
    fn udp_response_reader_forwards_packets_and_removes_session_on_read_error() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let sessions = UdpSessionMap::default();
            let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 40123);
            let remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), 443);
            let key = UdpSessionKey { local, remote };
            let conn = Arc::new(ScriptedPacketConn::new(vec![
                ScriptedPacketRead::Packet(b"udp-reply".to_vec(), remote),
                ScriptedPacketRead::Error("udp conn closed"),
            ]));
            sessions.insert(key, UdpTunSession::new(conn.clone()));
            let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();

            read_udp_responses(key, local, conn, sessions.clone(), reply_tx).await;

            let reply = reply_rx.try_recv().expect("forwarded udp reply");
            assert_eq!(reply.data, b"udp-reply");
            assert_eq!(reply.local, local);
            assert_eq!(reply.remote, remote);
            assert!(sessions.get(&key).is_none());
        });
    }

    #[test]
    fn udp_direct_session_forwards_echo_payloads() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let echo = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
            let echo_addr = echo.local_addr().unwrap();
            let echo_task = tokio::spawn(async move {
                let mut buffer = [0_u8; 1024];
                for _ in 0..2 {
                    let (size, peer) = echo.recv_from(&mut buffer).await.unwrap();
                    let mut response = b"echo:".to_vec();
                    response.extend_from_slice(&buffer[..size]);
                    echo.send_to(&response, peer).await.unwrap();
                }
            });
            let tunnel = direct_mode_tunnel();
            let dns_table = DnsTable::default();
            let sessions = UdpSessionMap::default();
            let flow_tasks: FlowTasks = Arc::new(Mutex::new(Vec::new()));
            let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();
            let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 19, 0, 2)), 40123);
            let key = UdpSessionKey {
                local,
                remote: echo_addr,
            };

            handle_udp_datagram(
                tunnel.clone(),
                dns_table.clone(),
                sessions.clone(),
                reply_tx.clone(),
                flow_tasks.clone(),
                b"first".to_vec(),
                local,
                echo_addr,
            )
            .await;
            let first = tokio::time::timeout(Duration::from_secs(2), reply_rx.recv())
                .await
                .unwrap()
                .expect("first udp reply");
            assert_eq!(first.data, b"echo:first");
            assert_eq!(first.local, local);
            assert_eq!(first.remote, echo_addr);
            assert!(sessions.get(&key).is_some());

            handle_udp_datagram(
                tunnel,
                dns_table,
                sessions.clone(),
                reply_tx,
                flow_tasks.clone(),
                b"second".to_vec(),
                local,
                echo_addr,
            )
            .await;
            let second = tokio::time::timeout(Duration::from_secs(2), reply_rx.recv())
                .await
                .unwrap()
                .expect("second udp reply");
            assert_eq!(second.data, b"echo:second");
            assert_eq!(second.local, local);
            assert_eq!(second.remote, echo_addr);
            assert!(sessions.get(&key).is_some());

            echo_task.await.unwrap();
            abort_flow_tasks(&flow_tasks).await;
        });
    }

    #[test]
    fn dns_table_expires_stale_records() {
        let table = DnsTable::default();
        let ip = IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4));
        table.insert(ip, "expired.test".to_owned(), 60);
        {
            let mut records = table.records.lock().unwrap();
            records.get_mut(&ip).unwrap().expires_at_ms = monotonic_ms().saturating_sub(1);
        }
        assert_eq!(table.lookup(ip), None);
        assert!(!table.records.lock().unwrap().contains_key(&ip));
    }

    #[test]
    fn dns_table_evicts_earliest_record_when_full() {
        let table = DnsTable::default();
        let oldest_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for index in 0..DNS_TABLE_MAX_RECORDS {
            let ip = IpAddr::V4(Ipv4Addr::new(
                10,
                ((index >> 16) & 0xff) as u8,
                ((index >> 8) & 0xff) as u8,
                (index & 0xff) as u8,
            ));
            table.insert(ip, format!("host-{index}.test"), 300);
        }
        {
            let mut records = table.records.lock().unwrap();
            records.get_mut(&oldest_ip).unwrap().expires_at_ms = 1;
        }

        table.insert(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55)),
            "new.test".to_owned(),
            300,
        );

        let records = table.records.lock().unwrap();
        assert_eq!(records.len(), DNS_TABLE_MAX_RECORDS);
        assert!(!records.contains_key(&oldest_ip));
    }

    #[test]
    fn dns_response_cache_reuses_response_with_fresh_transaction_id() {
        let cache = DnsResponseCache::default();
        let query = dns_query("cache.example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response =
            build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)));
        let records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &records);

        let mut second_query = query.clone();
        second_query[0] = 0xab;
        second_query[1] = 0xcd;
        let cached = cache.lookup(&second_query).expect("cached response");

        assert_eq!(&cached[0..2], &[0xab, 0xcd]);
        assert_eq!(&cached[2..], &response[2..]);
    }

    #[test]
    fn dns_response_cache_matches_case_insensitive_question_and_rewrites_question() {
        let cache = DnsResponseCache::default();
        let query = dns_query("CaseCache.Example.Test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response =
            build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8)));
        let records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &records);

        let mut second_query = dns_query("casecache.example.test", 1);
        second_query[0] = 0xab;
        second_query[1] = 0xcd;
        second_query[2] = 0x00;
        second_query[3] = 0x00;
        let cached = cache.lookup(&second_query).expect("cached response");
        let second_question_end = parse_dns_question_end(&second_query).unwrap();

        assert_eq!(&cached[0..2], &[0xab, 0xcd]);
        assert_eq!(
            &cached[12..second_question_end],
            &second_query[12..second_question_end]
        );
        assert_eq!(
            parse_dns_response_records(&cached),
            vec![(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8)),
                "casecache.example.test".to_owned(),
                60
            )]
        );
    }

    #[test]
    fn dns_response_cache_rewrites_cached_ttl_to_remaining_lifetime() {
        let cache = DnsResponseCache::default();
        let query = dns_query("ttl-cache.example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response =
            build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)));
        let records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &records);

        let key = dns_cache_key(&query).expect("cache key");
        {
            let mut records = cache.records.lock().unwrap();
            records.get_mut(&key).unwrap().expires_at_ms = monotonic_ms().saturating_add(42_000);
        }

        let cached = cache.lookup(&query).expect("cached response");
        let cached_records = parse_dns_response_records(&cached);

        assert_eq!(
            cached_records,
            vec![(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)),
                "ttl-cache.example.test".to_owned(),
                42
            )]
        );
    }

    #[test]
    fn dns_response_cache_expires_stale_records() {
        let cache = DnsResponseCache::default();
        let query = dns_query("expired-cache.example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response =
            build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)));
        let records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &records);

        let key = dns_cache_key(&query).expect("cache key");
        {
            let mut records = cache.records.lock().unwrap();
            records.get_mut(&key).unwrap().expires_at_ms = monotonic_ms().saturating_sub(1);
        }

        assert_eq!(cache.lookup(&query), None);
        assert!(!cache.records.lock().unwrap().contains_key(&key));
    }

    #[test]
    fn dns_response_cache_evicts_earliest_record_when_full() {
        let cache = DnsResponseCache::default();
        let oldest_query = dns_query("cache-0.example.test", 1);
        for index in 0..DNS_CACHE_MAX_RECORDS {
            let query = dns_query(&format!("cache-{index}.example.test"), 1);
            let parsed = parse_dns_query(&query).expect("dns query");
            let response = build_dns_response(
                &query,
                &parsed,
                IpAddr::V4(Ipv4Addr::new(
                    203,
                    0,
                    ((index >> 8) & 0xff) as u8,
                    index as u8,
                )),
            );
            let records = parse_dns_response_records(&response);
            cache.insert(&query, &response, &records);
        }
        let oldest_key = dns_cache_key(&oldest_query).expect("cache key");
        {
            let mut records = cache.records.lock().unwrap();
            records.get_mut(&oldest_key).unwrap().expires_at_ms = 1;
        }

        let query = dns_query("cache-new.example.test", 1);
        let parsed = parse_dns_query(&query).expect("dns query");
        let response =
            build_dns_response(&query, &parsed, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 99)));
        let response_records = parse_dns_response_records(&response);
        cache.insert(&query, &response, &response_records);

        let records = cache.records.lock().unwrap();
        assert_eq!(records.len(), DNS_CACHE_MAX_RECORDS);
        assert!(!records.contains_key(&oldest_key));
    }

    #[test]
    fn shared_stats_tracks_recent_dns_queries() {
        let stats = SharedStats::default();
        stats.dns_cache_hits.store(2, Ordering::Relaxed);
        stats.dns_cache_misses.store(5, Ordering::Relaxed);
        stats.record_dns_query("first.example.test".to_owned(), "A".to_owned());
        stats.record_dns_query("second.example.test".to_owned(), "AAAA".to_owned());
        stats.record_dns_query("first.example.test".to_owned(), "A".to_owned());
        stats.record_dns_query("First.Example.Test".to_owned(), "A".to_owned());

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.dns_cache_hits, 2);
        assert_eq!(snapshot.dns_cache_misses, 5);
        assert_eq!(snapshot.recent_dns_queries.len(), 2);
        assert_eq!(snapshot.recent_dns_queries[0].name, "First.Example.Test");
        assert_eq!(snapshot.recent_dns_queries[0].record_type, "A");
        assert_eq!(snapshot.recent_dns_queries[0].count, 3);
        assert_eq!(snapshot.recent_dns_queries[1].name, "second.example.test");
    }

    fn dns_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut query = vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        push_dns_name(&mut query, name);
        query.extend_from_slice(&qtype.to_be_bytes());
        query.extend_from_slice(&1_u16.to_be_bytes());
        query
    }

    fn build_dns_cname_response(
        query: &[u8],
        request: &DnsQuery,
        cname_target: &str,
        ip: IpAddr,
    ) -> Vec<u8> {
        let mut response = Vec::with_capacity(query.len() + 96);
        response.extend_from_slice(&query[..request.question_end]);
        response[2] = 0x81;
        response[3] = 0x80;
        response[6..8].copy_from_slice(&2_u16.to_be_bytes());
        response[8..10].copy_from_slice(&0_u16.to_be_bytes());
        response[10..12].copy_from_slice(&0_u16.to_be_bytes());

        response.extend_from_slice(&[0xc0, 0x0c]);
        response.extend_from_slice(&5_u16.to_be_bytes());
        response.extend_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&60_u32.to_be_bytes());
        let mut cname_rdata = Vec::new();
        push_dns_name(&mut cname_rdata, cname_target);
        response.extend_from_slice(&(cname_rdata.len() as u16).to_be_bytes());
        response.extend_from_slice(&cname_rdata);

        push_dns_name(&mut response, cname_target);
        match ip {
            IpAddr::V4(ip) => {
                response.extend_from_slice(&1_u16.to_be_bytes());
                response.extend_from_slice(&1_u16.to_be_bytes());
                response.extend_from_slice(&60_u32.to_be_bytes());
                response.extend_from_slice(&4_u16.to_be_bytes());
                response.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                response.extend_from_slice(&28_u16.to_be_bytes());
                response.extend_from_slice(&1_u16.to_be_bytes());
                response.extend_from_slice(&60_u32.to_be_bytes());
                response.extend_from_slice(&16_u16.to_be_bytes());
                response.extend_from_slice(&ip.octets());
            }
        }
        response
    }

    fn push_dns_name(packet: &mut Vec<u8>, name: &str) {
        for label in name.split('.') {
            packet.push(label.len() as u8);
            packet.extend_from_slice(label.as_bytes());
        }
        packet.push(0);
    }
}
