//! Opt-in native C ABI acceptance, never a fake provider converter/controller.
//! Root builds the pinned `paws_host_test` Go shared library separately.
#![cfg(unix)]

#[path = "native_dns_tests.rs"]
mod native_dns_tests;

use paws_engine::{EngineManagement, EngineNetworkServices, EngineProviderKind};
use paws_model::{RuntimeMode, VpnEngine, VpnOptions};
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::{c_void, CStr, CString};
use std::net::Ipv4Addr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

type HostStart = unsafe extern "C" fn(*const u8, usize, *const u8, usize) -> libc::c_int;
type HostStop = unsafe extern "C" fn() -> libc::c_int;
type CopyError = unsafe extern "C" fn(*mut u8, usize) -> usize;
type SetCompiler = unsafe extern "C" fn(*const c_void) -> libc::c_int;

struct NativeLibrary {
    // Go's runtime has process-lifetime threads even after Box.Close. Keep one
    // resident image; never dlclose it or repeatedly load/leak new runtimes.
    _handle: usize,
    path: PathBuf,
    start: HostStart,
    stop: HostStop,
    copy_error: CopyError,
    set_compiler: SetCompiler,
}
impl NativeLibrary {
    unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> T {
        let symbol = unsafe { libc::dlsym(handle, name.as_ptr()) };
        assert!(!symbol.is_null(), "required host C ABI export is missing");
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&symbol));
        unsafe { std::mem::transmute_copy(&symbol) }
    }

    fn get() -> &'static Self {
        static LIBRARY: OnceLock<NativeLibrary> = OnceLock::new();
        let path = PathBuf::from(
            std::env::var_os("PAWS_SING_HOST_LIBRARY")
                .expect("opt-in test requires Root-built PAWS_SING_HOST_LIBRARY"),
        );
        assert!(
            path.is_absolute(),
            "native library path must be explicit and absolute"
        );
        let path = path.canonicalize().expect("host shared library must exist");
        let library = LIBRARY.get_or_init(|| {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            assert!(
                !handle.is_null(),
                "cannot load the Root-built native host library"
            );
            unsafe {
                Self {
                    _handle: handle as usize,
                    path: path.clone(),
                    start: Self::symbol(handle, c"TinyPawsSingHostStart"),
                    stop: Self::symbol(handle, c"TinyPawsSingHostStop"),
                    copy_error: Self::symbol(handle, c"TinyPawsSingHostCopyLastError"),
                    set_compiler: Self::symbol(handle, c"TinyPawsSingSetProviderCompiler"),
                }
            }
        });
        assert_eq!(library.path, path, "one Go image per test process");
        library
    }

    fn register_real_compiler(&self) {
        let callback: crate::provider_bridge::CompileProvider =
            crate::provider_bridge::compile_provider;
        assert_eq!(
            unsafe { (self.set_compiler)(callback as *const c_void) },
            1,
            "native bridge must accept the actual production Rust callback"
        );
    }

    fn last_error(&self) -> String {
        let mut buffer = [0u8; 4096];
        let length =
            unsafe { (self.copy_error)(buffer.as_mut_ptr(), buffer.len()) }.min(buffer.len());
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    }
}

#[derive(Clone, Copy)]
enum Role {
    Provider,
    Origin(usize),
    Socks(usize),
}

struct FixtureState {
    proxy_mode: AtomicU8,
    rule_mode: AtomicU8,
    proxy_requests: AtomicUsize,
    rule_requests: AtomicUsize,
    node_dials: [AtomicUsize; 2],
    origin_requests: [AtomicUsize; 2],
    origin_ports: [u16; 2],
    socks_ports: [u16; 2],
    held_entered: Semaphore,
    held_release: Semaphore,
    held_finished: Semaphore,
}
impl FixtureState {
    fn proxy_body(&self, revision: u8) -> Vec<u8> {
        if revision == 2 {
            return b"proxies: [{name: rejected, type: unknown-protocol}]\n".to_vec();
        }
        let index = usize::from(revision == 1);
        format!(
            "proxies: [{{name: provider-{}, type: socks5, server: 127.0.0.1, port: {}}}]\n",
            if index == 0 { "A" } else { "B" },
            self.socks_ports[index]
        )
        .into_bytes()
    }
    fn rule_body(&self, revision: u8) -> Vec<u8> {
        match revision {
            0 => format!("payload: ['DST-PORT,{}']\n", self.origin_ports[0]).into_bytes(),
            1 => format!(
                "payload: ['DST-PORT,{}', 'DOMAIN,rule-b.fixture.invalid']\n",
                self.origin_ports[1]
            )
            .into_bytes(),
            _ => b"payload: ['UNKNOWN,fixture.invalid']\n".to_vec(),
        }
    }
}

struct Fixtures {
    state: Arc<FixtureState>,
    provider_port: u16,
    dns_port: u16,
    tasks: Vec<JoinHandle<()>>,
}
impl Fixtures {
    async fn start() -> Self {
        let mut listeners = Vec::new();
        for _ in 0..5 {
            listeners.push(TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        }
        let ports = listeners
            .iter()
            .map(|listener| listener.local_addr().unwrap().port())
            .collect::<Vec<_>>();
        let state = Arc::new(FixtureState {
            proxy_mode: AtomicU8::new(0),
            rule_mode: AtomicU8::new(0),
            proxy_requests: AtomicUsize::new(0),
            rule_requests: AtomicUsize::new(0),
            node_dials: [AtomicUsize::new(0), AtomicUsize::new(0)],
            origin_requests: [AtomicUsize::new(0), AtomicUsize::new(0)],
            origin_ports: [ports[1], ports[2]],
            socks_ports: [ports[3], ports[4]],
            held_entered: Semaphore::new(0),
            held_release: Semaphore::new(0),
            held_finished: Semaphore::new(0),
        });
        let mut tasks = Vec::new();
        for (listener, role) in listeners.into_iter().zip([
            Role::Provider,
            Role::Origin(0),
            Role::Origin(1),
            Role::Socks(0),
            Role::Socks(1),
        ]) {
            let state = state.clone();
            tasks.push(tokio::spawn(async move {
                let mut requests = JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { break };
                            if requests.len() >= 32 { drop(stream); continue; }
                            let state = state.clone();
                            requests.spawn(async move {
                                let _ = tokio::time::timeout(Duration::from_secs(8), Self::serve(stream, state, role)).await;
                            });
                        },
                        _ = requests.join_next(), if !requests.is_empty() => {}
                    }
                }
            }));
        }
        let dns = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let dns_port = dns.local_addr().unwrap().port();
        tasks.push(tokio::spawn(async move {
            let mut buffer = [0u8; 512];
            while let Ok((length, peer)) = dns.recv_from(&mut buffer).await {
                // Explicit fixture resolver, never a system/public DNS guess.
                // No name/address/packet body is logged or persisted.
                if length < 12 || buffer[4..6] != [0, 1] {
                    continue;
                }
                let mut reply = buffer[..length].to_vec();
                reply[2..4].copy_from_slice(&[0x81, 0x85]); // bounded REFUSED
                reply[6..12].fill(0);
                let _ = dns.send_to(&reply, peer).await;
            }
        }));
        Self {
            state,
            provider_port: ports[0],
            dns_port,
            tasks,
        }
    }

    async fn headers(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        while bytes.len() < 8192 {
            bytes.push(stream.read_u8().await?);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(bytes);
            }
        }
        Err(std::io::Error::other(
            "bounded synthetic HTTP header exceeded",
        ))
    }

    async fn response(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
        stream
            .write_all(
                format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        stream.write_all(body).await?;
        stream.shutdown().await
    }

    async fn serve(
        mut stream: TcpStream,
        state: Arc<FixtureState>,
        role: Role,
    ) -> std::io::Result<()> {
        if let Role::Socks(node) = role {
            let mut greeting = [0; 2];
            stream.read_exact(&mut greeting).await?;
            if greeting[0] != 5 || greeting[1] > 8 {
                return Ok(());
            }
            let mut methods = vec![0; usize::from(greeting[1])];
            stream.read_exact(&mut methods).await?;
            if !methods.contains(&0) {
                stream.write_all(&[5, 255]).await?;
                return Ok(());
            }
            stream.write_all(&[5, 0]).await?;
            let mut request = [0; 4];
            stream.read_exact(&mut request).await?;
            let localhost = match request[3] {
                1 => {
                    let mut address = [0; 4];
                    stream.read_exact(&mut address).await?;
                    address == [127, 0, 0, 1]
                }
                3 => {
                    let size = stream.read_u8().await?;
                    let mut name = vec![0; usize::from(size)];
                    stream.read_exact(&mut name).await?;
                    name == b"127.0.0.1"
                }
                _ => return Ok(()),
            };
            let port = stream.read_u16().await?;
            if request[..3] != [5, 1, 0] || !localhost || !state.origin_ports.contains(&port) {
                stream.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                return Ok(());
            }
            let mut upstream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
            state.node_dials[node].fetch_add(1, Ordering::SeqCst);
            stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await?;
            return Ok(());
        }
        let headers = Self::headers(&mut stream).await?;
        let text = std::str::from_utf8(&headers).unwrap_or("");
        let route = text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");
        if let Role::Origin(index) = role {
            let prefix = format!("http://127.0.0.1:{}", state.origin_ports[index]);
            let route = route.strip_prefix(&prefix).unwrap_or(route);
            if !route.starts_with("/route?") {
                return Self::response(&mut stream, 404, b"unknown fixture\n").await;
            }
            state.origin_requests[index].fetch_add(1, Ordering::SeqCst);
            return Self::response(
                &mut stream,
                200,
                if index == 0 {
                    b"paws-native-origin:A\n"
                } else {
                    b"paws-native-origin:B\n"
                },
            )
            .await;
        }
        let (mode, body) = if route == "/proxy.yaml" {
            state.proxy_requests.fetch_add(1, Ordering::SeqCst);
            let mode = state.proxy_mode.load(Ordering::SeqCst);
            (mode, state.proxy_body(mode))
        } else if route == "/rule.yaml" {
            state.rule_requests.fetch_add(1, Ordering::SeqCst);
            let mode = state.rule_mode.load(Ordering::SeqCst);
            (mode, state.rule_body(mode))
        } else {
            return Self::response(&mut stream, 404, b"unknown fixture\n").await;
        };
        if mode == 3 {
            state.held_entered.add_permits(1);
            state.held_release.acquire().await.unwrap().forget();
        }
        let result = Self::response(&mut stream, 200, &body).await;
        if mode == 3 {
            state.held_finished.add_permits(1);
        }
        result
    }
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "paws-native-provider-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct RunningOwner {
    library: &'static NativeLibrary,
    running: bool,
}
impl Drop for RunningOwner {
    fn drop(&mut self) {
        if self.running {
            let _ = unsafe { (self.library.stop)() };
        }
    }
}

async fn route_request(
    client: &reqwest::Client,
    port: u16,
    case: &str,
) -> Result<String, reqwest::Error> {
    client
        .get(format!("http://127.0.0.1:{port}/route?case={case}"))
        .header(reqwest::header::CONNECTION, "close")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Root-built pinned Go host c-shared and PAWS_SING_HOST_LIBRARY; actual sockets"]
async fn native_c_shared_provider_transactions_use_actual_rust_compiler_and_data_plane() {
    let library = NativeLibrary::get();
    library.register_real_compiler();
    let root = TestRoot::new();
    let fixtures = Fixtures::start().await;
    let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mixed_port = reservation.local_addr().unwrap().port();
    let public_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let public_port = public_reservation.local_addr().unwrap().port();
    drop(reservation);
    drop(public_reservation);
    let mut services = EngineNetworkServices::default();
    services.ports.mixed_enabled = true;
    services.ports.mixed_port = mixed_port;
    services.ports.controller_enabled = true;
    services.ports.controller_port = public_port;
    services.access.allow_lan = false;
    services.access.secret = Some("synthetic-public-controller-only".into());
    let dns = format!("udp://127.0.0.1:{}", fixtures.dns_port);
    let yaml = json!({
        "authentication":["qa-user:qa-password"], "skip-auth-prefixes":[],
        "proxies":[], "proxy-groups":[{"name":"QA-PROVIDED","type":"select","use":["QA-PROVIDER"]}],
        "proxy-providers":{"QA-PROVIDER":{"type":"http","url":format!("http://127.0.0.1:{}/proxy.yaml",fixtures.provider_port),"path":root.0.join("proxy.yaml"),"interval":0}},
        "rule-providers":{"QA-RULES":{"type":"http","url":format!("http://127.0.0.1:{}/rule.yaml",fixtures.provider_port),"path":root.0.join("rule.yaml"),"behavior":"classical","format":"yaml","interval":0}},
        "dns":{"enable":true,"ipv6":false,"nameserver":[dns.clone()],"fallback":[],"nameserver-policy":{}},
        "rules":["RULE-SET,QA-RULES,QA-PROVIDED","MATCH,REJECT"]
    }).to_string();
    let options = VpnOptions {
        engine: VpnEngine::SingBox,
        dns_hijacking: false,
        dns_servers: vec![dns],
        dns_fallbacks: Vec::new(),
        dns_nameserver_policy: BTreeMap::new(),
        ..Default::default()
    };
    let rendered = crate::config::render_sing_box_runtime_json(
        &yaml,
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &options,
        &services,
    )
    .unwrap();
    // Exact production renderer/security path; only host_probe removes TUN.
    let config = crate::config::secure_controller_config(
        VpnEngine::SingBox,
        &rendered.config_json,
        "synthetic-native-provider-owner",
    )
    .unwrap();
    let endpoint = crate::config::controller_endpoint(VpnEngine::SingBox, &config).unwrap();
    assert!(endpoint.address.ip().is_loopback());
    let private_endpoint = endpoint.clone();
    let directory = root.0.as_os_str().as_bytes();
    let started = unsafe {
        (library.start)(
            config.as_ptr(),
            config.len(),
            directory.as_ptr(),
            directory.len(),
        )
    };
    assert_eq!(
        started,
        0,
        "actual native provider initial load failed: {}",
        library.last_error()
    );
    let mut owner = RunningOwner {
        library,
        running: true,
    };
    let controller = Arc::new(crate::ControllerClient::new(VpnEngine::SingBox, endpoint).unwrap());
    let applied = controller.applied_services().await.unwrap();
    assert!(
        applied.mixed_ready,
        "actual mixed listener must be acknowledged"
    );
    assert!(applied.controller_endpoint.as_ref() == Some(&private_endpoint));
    assert_eq!(
        applied.public_controller_addr,
        Some((Ipv4Addr::LOCALHOST, public_port).into())
    );
    for proxy in [&applied.app_http_proxy, &applied.benchmark_proxy] {
        let proxy = proxy.as_ref().expect("actual private benchmark listener");
        assert_eq!(proxy.port, 18492);
        assert!(
            Some(&proxy.token) == private_endpoint.secret.as_ref(),
            "private proxy credential must come from the captured owner, not the acknowledgment DTO"
        );
        assert!(Some(&proxy.token) != services.access.secret.as_ref());
    }
    assert!(applied.warnings.is_empty());
    assert!(fixtures.state.proxy_requests.load(Ordering::SeqCst) > 0);
    assert!(fixtures.state.rule_requests.load(Ordering::SeqCst) > 0);
    let initial = controller
        .provider_state(EngineProviderKind::Proxy, "QA-PROVIDER")
        .await
        .unwrap();
    let initial_members = initial.members.unwrap();
    assert_eq!(initial_members[0].name, "provider-A");
    assert_eq!(
        initial_members[0].alive, None,
        "unprobed native member is unknown, not failed"
    );
    assert_eq!(initial_members[0].delay_ms, None);
    assert_eq!(
        controller
            .provider_state(EngineProviderKind::Rule, "QA-RULES")
            .await
            .unwrap()
            .rule_count,
        Some(1)
    );
    let proxy = reqwest::Proxy::http(format!("http://127.0.0.1:{mixed_port}"))
        .unwrap()
        .basic_auth("qa-user", "qa-password");
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    assert_eq!(
        route_request(&client, fixtures.state.origin_ports[0], "initial-A")
            .await
            .unwrap(),
        "paws-native-origin:A\n"
    );
    assert_eq!(fixtures.state.node_dials[0].load(Ordering::SeqCst), 1);
    assert_eq!(fixtures.state.node_dials[1].load(Ordering::SeqCst), 0);

    fixtures.state.proxy_mode.store(1, Ordering::SeqCst);
    controller
        .refresh_provider("proxy", "QA-PROVIDER")
        .await
        .unwrap();
    let members = controller.provider_members("QA-PROVIDER").await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, "provider-B");
    assert_eq!(
        members[0].alive, None,
        "refreshed member has no measured health yet"
    );
    assert_eq!(members[0].delay_ms, None);
    controller
        .select_proxy("QA-PROVIDED", Some("provider-B"))
        .await
        .unwrap();
    assert_eq!(
        controller
            .proxy_selection("QA-PROVIDED")
            .await
            .unwrap()
            .selected
            .as_deref(),
        Some("provider-B")
    );
    assert_eq!(
        controller.proxy_group("QA-PROVIDED").await.unwrap().proxies[0].name,
        "provider-B"
    );
    assert_eq!(
        route_request(&client, fixtures.state.origin_ports[0], "proxy-B")
            .await
            .unwrap(),
        "paws-native-origin:A\n"
    );
    assert_eq!(fixtures.state.node_dials[0].load(Ordering::SeqCst), 1);
    assert_eq!(fixtures.state.node_dials[1].load(Ordering::SeqCst), 1);

    fixtures.state.rule_mode.store(1, Ordering::SeqCst);
    controller
        .refresh_provider("rule", "QA-RULES")
        .await
        .unwrap();
    assert_eq!(
        controller
            .provider_state(EngineProviderKind::Rule, "QA-RULES")
            .await
            .unwrap()
            .rule_count,
        Some(2)
    );
    assert!(route_request(
        &client,
        fixtures.state.origin_ports[0],
        "old-rule-must-block"
    )
    .await
    .is_err());
    assert_eq!(
        route_request(&client, fixtures.state.origin_ports[1], "new-rule-B")
            .await
            .unwrap(),
        "paws-native-origin:B\n"
    );
    let proxy_cache = std::fs::read(root.0.join("proxy.yaml")).unwrap();
    let rule_cache = std::fs::read(root.0.join("rule.yaml")).unwrap();
    assert_eq!(proxy_cache, fixtures.state.proxy_body(1));
    assert_eq!(rule_cache, fixtures.state.rule_body(1));
    fixtures.state.proxy_mode.store(2, Ordering::SeqCst);
    fixtures.state.rule_mode.store(2, Ordering::SeqCst);
    assert!(controller
        .refresh_provider("proxy", "QA-PROVIDER")
        .await
        .is_err());
    assert!(controller
        .refresh_provider("rule", "QA-RULES")
        .await
        .is_err());
    assert_eq!(
        controller
            .provider_state(EngineProviderKind::Proxy, "QA-PROVIDER")
            .await
            .unwrap()
            .members
            .unwrap()[0]
            .name,
        "provider-B"
    );
    assert_eq!(
        controller
            .provider_state(EngineProviderKind::Rule, "QA-RULES")
            .await
            .unwrap()
            .rule_count,
        Some(2)
    );
    assert_eq!(
        std::fs::read(root.0.join("proxy.yaml")).unwrap(),
        proxy_cache
    );
    assert_eq!(std::fs::read(root.0.join("rule.yaml")).unwrap(), rule_cache);
    assert_eq!(
        route_request(
            &client,
            fixtures.state.origin_ports[1],
            "rollback-retains-B"
        )
        .await
        .unwrap(),
        "paws-native-origin:B\n"
    );

    fixtures.state.proxy_mode.store(3, Ordering::SeqCst);
    let pending_controller = controller.clone();
    let pending = tokio::spawn(async move {
        pending_controller
            .refresh_provider("proxy", "QA-PROVIDER")
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(2),
        fixtures.state.held_entered.acquire(),
    )
    .await
    .unwrap()
    .unwrap()
    .forget();
    owner.running = false; // one stop caller; never wait twice if a broken stop hangs
    let stop = library.stop;
    let stopped = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || unsafe { stop() }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        stopped,
        0,
        "actual native stop failed: {}",
        library.last_error()
    );
    fixtures.state.held_release.add_permits(1);
    tokio::time::timeout(
        Duration::from_secs(2),
        fixtures.state.held_finished.acquire(),
    )
    .await
    .unwrap()
    .unwrap()
    .forget();
    assert!(tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(controller
        .provider_state(EngineProviderKind::Proxy, "QA-PROVIDER")
        .await
        .is_err());
    assert_eq!(
        std::fs::read(root.0.join("proxy.yaml")).unwrap(),
        proxy_cache,
        "late retired refresh changed provider cache"
    );
    assert_eq!(std::fs::read(root.0.join("rule.yaml")).unwrap(), rule_cache);
    assert!(fixtures.state.origin_requests[0].load(Ordering::SeqCst) >= 2);
    assert!(fixtures.state.origin_requests[1].load(Ordering::SeqCst) >= 2);
    let mut released = Vec::new();
    for port in [
        private_endpoint.address.port(),
        public_port,
        mixed_port,
        18492,
    ] {
        released.push(
            TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .await
                .expect("stop must release actual private/public/mixed/benchmark listener"),
        );
    }
    eprintln!(
        "PASS actual native provider C ABI: initial/refresh/proxy+rule rollback/real SOCKS A+B routes/retired late response/actual services ack/four ports released"
    );
}

fn real_geo_callback(kind: u32, category: &str, source: &[u8]) -> (serde_json::Value, usize, u128) {
    let mut input = u32::try_from(category.len())
        .unwrap()
        .to_le_bytes()
        .to_vec();
    input.extend_from_slice(category.as_bytes());
    input.extend_from_slice(source);
    let mut output_length = 0;
    let started = std::time::Instant::now();
    let output = unsafe {
        crate::provider_bridge::compile_provider(
            kind,
            0,
            3,
            input.as_ptr(),
            input.len(),
            &mut output_length,
        )
    };
    let elapsed = started.elapsed().as_micros();
    assert!(
        !output.is_null(),
        "real Geo compiler returned no ABI buffer"
    );
    let decoded = serde_json::from_slice::<serde_json::Value>(unsafe {
        std::slice::from_raw_parts(output.cast::<u8>(), output_length)
    });
    unsafe { libc::free(output) };
    let decoded = decoded.unwrap();
    assert_eq!(
        decoded["ok"], true,
        "real bundled resource failed compilation: {:?}",
        decoded["error"]
    );
    (decoded["data"].clone(), output_length, elapsed)
}

// This evaluates coverage represented by the serialized native matcher DTO.
// It is an oracle comparison, not a replacement Go matcher or device result.
fn serialized_domain_match(rule: &serde_json::Value, domain: &str) -> bool {
    if rule.get("type").and_then(serde_json::Value::as_str) == Some("logical") {
        let rules = rule["rules"].as_array().unwrap();
        return match rule["mode"].as_str().unwrap() {
            "and" => rules
                .iter()
                .all(|rule| serialized_domain_match(rule, domain)),
            "or" => rules
                .iter()
                .any(|rule| serialized_domain_match(rule, domain)),
            _ => panic!("unexpected logical domain matcher"),
        };
    }
    let mut matched = false;
    for (key, values) in rule.as_object().unwrap() {
        for value in values
            .as_array()
            .expect("domain matcher fields must be arrays")
        {
            let value = value.as_str().unwrap();
            matched |= match key.as_str() {
                "domain" => domain == value,
                "domain_suffix" => {
                    domain == value
                        || domain
                            .strip_suffix(value)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                }
                "domain_keyword" => domain.contains(value),
                "domain_regex" => regex::Regex::new(value).unwrap().is_match(domain),
                _ => panic!("unexpected serialized domain matcher field"),
            };
        }
    }
    matched
}

fn serialized_cidr_contains(cidr: &str, address: std::net::IpAddr) -> bool {
    let (network, bits) = cidr.split_once('/').unwrap();
    let network = network.parse::<std::net::IpAddr>().unwrap();
    let bits = bits.parse::<u32>().unwrap();
    match (network, address) {
        (std::net::IpAddr::V4(network), std::net::IpAddr::V4(address)) => {
            assert!(bits <= 32);
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            u32::from(network) & mask == u32::from(address) & mask
        }
        (std::net::IpAddr::V6(network), std::net::IpAddr::V6(address)) => {
            assert!(bits <= 128);
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            u128::from(network) & mask == u128::from(address) & mask
        }
        _ => false,
    }
}

#[test]
#[ignore = "explicit bundled-resource admission: PAWS_ENGINE_GEO_ACCEPTANCE=1; no native library required"]
fn bundled_geo_resources_compile_through_real_callback_and_match_source_oracles() {
    assert_eq!(
        std::env::var("PAWS_ENGINE_GEO_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../entry/src/main/resources/rawfile/geodata")
        .canonicalize()
        .unwrap();
    let site_source = std::fs::read(root.join("geosite.dat")).unwrap();
    let allowed = std::collections::HashSet::from(["google".to_owned()]);
    let oracle = meow_rules::geosite::GeositeDB::from_bytes(&site_source, Some(&allowed)).unwrap();
    let (site, output_bytes, micros) = real_geo_callback(3, "google", &site_source);
    let rules = site["rules"].as_array().unwrap();
    assert!(!rules.is_empty());
    let mut positives = 0;
    for domain in [
        "google.com",
        "maps.google.com",
        "foo.google.com",
        "gstatic.com",
        "youtube.com",
        "example.invalid",
        "baidu.com",
        "google.com.fixture.invalid",
        "evilgoogle.com",
    ] {
        let expected = oracle.lookup("google", domain);
        positives += usize::from(expected);
        assert_eq!(
            rules
                .iter()
                .any(|rule| serialized_domain_match(rule, domain)),
            expected,
            "compiled matcher coverage disagrees with the actual GeositeDB for {domain}"
        );
    }
    assert!(positives > 0 && positives < 9);
    eprintln!(
        "GEO_CALLBACK {}",
        json!({"resource":"geosite.dat","category":"google","inputBytes":site_source.len(),"outputBytes":output_bytes,"compileMicros":micros,"samples":9,"scope":"serialized matcher coverage vs actual GeositeDB; not Go/native routing"})
    );
    assert_eq!(
        std::fs::read(root.join("geosite.dat")).unwrap(),
        site_source
    );

    for (kind, file, category) in [(4, "Country.mmdb", "CN"), (5, "GeoLite2-ASN.mmdb", "13335")] {
        let source = std::fs::read(root.join(file)).unwrap();
        let reader = maxminddb::Reader::from_source(source.as_slice()).unwrap();
        let (compiled, output_bytes, micros) = real_geo_callback(kind, category, &source);
        let rules = compiled["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        let networks = rules[0]["ip_cidr"].as_array().unwrap();
        assert!(!networks.is_empty());
        let mut positives = 0;
        let samples = [
            "1.1.1.1",
            "1.0.0.1",
            "8.8.8.8",
            "223.5.5.5",
            "119.29.29.29",
            "114.114.114.114",
            "10.0.0.1",
            "2001:4860:4860::8888",
            "2400:3200::1",
            "2606:4700:4700::1111",
        ];
        for sample in samples {
            let address = sample.parse().unwrap();
            let record = reader.lookup(address).unwrap();
            let expected = if kind == 4 {
                record
                    .decode_path::<&str>(&[
                        maxminddb::PathElement::Key("country"),
                        maxminddb::PathElement::Key("iso_code"),
                    ])
                    .unwrap()
                    .is_some_and(|country| country.eq_ignore_ascii_case(category))
            } else {
                record
                    .decode_path::<u32>(&[maxminddb::PathElement::Key("autonomous_system_number")])
                    .unwrap()
                    == Some(13335)
            };
            positives += usize::from(expected);
            let actual = networks
                .iter()
                .any(|cidr| serialized_cidr_contains(cidr.as_str().unwrap(), address));
            assert_eq!(
                actual, expected,
                "compiled {file} CIDR coverage disagrees with direct MMDB lookup for {sample}"
            );
        }
        assert!(positives > 0 && positives < samples.len());
        eprintln!(
            "GEO_CALLBACK {}",
            json!({"resource":file,"category":category,"inputBytes":source.len(),"outputBytes":output_bytes,"compileMicros":micros,"networks":networks.len(),"samples":samples.len(),"scope":"serialized CIDR coverage vs direct actual MMDB; not Go/native routing"})
        );
        assert_eq!(std::fs::read(root.join(file)).unwrap(), source);
    }
}
