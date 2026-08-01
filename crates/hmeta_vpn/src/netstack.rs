use super::*;

pub(super) async fn run_netstack_vpn(
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

pub(super) struct NetstackRuntime {
    ingress_tx: mpsc::Sender<Vec<u8>>,
    egress_tx: mpsc::UnboundedSender<Vec<u8>>,
    egress_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    handles: Vec<JoinHandle<()>>,
}

pub(super) fn spawn_smoltcp_backend(
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

pub(super) fn spawn_lwip_backend(
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

pub(super) fn track_flow_task(flow_tasks: &FlowTasks, handle: JoinHandle<()>) {
    if let Ok(mut tasks) = flow_tasks.lock() {
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle);
    } else {
        handle.abort();
    }
}

pub(super) async fn abort_flow_tasks(flow_tasks: &FlowTasks) {
    let tasks = flow_tasks
        .lock()
        .map(|mut tasks| tasks.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
}

pub(super) async fn handle_tcp_stream<S>(
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

pub(super) fn tcp_metadata_for_stream(
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
pub(super) enum SniffProtocol {
    Tls,
    Http,
}

pub(super) struct HarmonyTcpSniffer {
    config: SnifferConfig,
    runtime: SnifferRuntime,
    force_domains: DomainTrie<()>,
}

impl HarmonyTcpSniffer {
    pub(super) fn from_config(config: SnifferConfig) -> Option<Self> {
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

    pub(super) fn protocol_for(&self, port: u16) -> Option<SniffProtocol> {
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

    pub(super) fn should_sniff(&self, metadata: &Metadata) -> bool {
        if !self.config.parse_pure_ip || metadata.host.is_empty() {
            return true;
        }
        metadata.host.parse::<IpAddr>().is_ok()
            || self.force_domains.search(&metadata.host).is_some()
    }

    pub(super) async fn sniff<S>(&self, stream: &mut S, metadata: &mut Metadata) -> Vec<u8>
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

pub(super) struct ReplayConn<S> {
    stream: S,
    prefix: Vec<u8>,
    prefix_offset: usize,
}

impl<S> ReplayConn<S> {
    pub(super) fn new(stream: S, prefix: Vec<u8>) -> Self {
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
pub(super) struct UdpSessionKey {
    pub(super) local: SocketAddr,
    pub(super) remote: SocketAddr,
}

#[derive(Clone)]
pub(super) struct UdpTunSession {
    conn: Arc<dyn ProxyPacketConn>,
    pub(super) last_activity_ms: Arc<AtomicU64>,
}

impl UdpTunSession {
    pub(super) fn new(conn: Arc<dyn ProxyPacketConn>) -> Self {
        Self {
            conn,
            last_activity_ms: Arc::new(AtomicU64::new(monotonic_ms())),
        }
    }

    pub(super) fn touch(&self) {
        self.last_activity_ms
            .store(monotonic_ms(), Ordering::Relaxed);
    }

    pub(super) fn idle_for(&self) -> Duration {
        let last = self.last_activity_ms.load(Ordering::Relaxed);
        Duration::from_millis(monotonic_ms().saturating_sub(last))
    }
}

#[derive(Clone, Default)]
pub(super) struct UdpSessionMap {
    sessions: Arc<Mutex<HashMap<UdpSessionKey, UdpTunSession>>>,
}

impl UdpSessionMap {
    pub(super) fn get(&self, key: &UdpSessionKey) -> Option<UdpTunSession> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(key).cloned())
    }

    pub(super) fn insert(&self, key: UdpSessionKey, session: UdpTunSession) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(key, session);
        }
    }

    pub(super) fn remove(&self, key: &UdpSessionKey) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(key);
        }
    }

    pub(super) fn retain_active(&self, idle_timeout: Duration) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|_, session| session.idle_for() < idle_timeout);
        }
    }
}

pub(super) struct UdpReply {
    pub(super) data: Vec<u8>,
    pub(super) local: SocketAddr,
    pub(super) remote: SocketAddr,
}

pub(super) async fn handle_udp_datagram(
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

pub(super) fn udp_metadata_for_datagram(
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

pub(super) async fn read_udp_responses(
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
