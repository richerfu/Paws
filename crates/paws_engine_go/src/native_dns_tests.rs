//! Opt-in real DNS transport/dispatcher/cache acceptance against the same
//! process-lifetime native image as the provider test. No fake DNS controller.

use super::{NativeLibrary, RunningOwner, TestRoot};
use paws_engine::{EngineManagement, EngineNetworkServices};
use paws_model::{RuntimeMode, VpnEngine, VpnOptions};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::task::JoinHandle;

const PACKET_LIMIT: usize = 4096;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const SUFFIX: &str = "fixture.invalid";

fn word(packet: &[u8], offset: usize) -> Result<u16, &'static str> {
    let bytes = packet.get(offset..offset + 2).ok_or("truncated DNS word")?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// Accept legal compression differences, but reject loops, reserved labels,
/// overlong names and out-of-packet pointers. `cursor` advances past the name's
/// encoded representation, not past the expanded pointer target.
fn name(packet: &[u8], cursor: &mut usize) -> Result<String, &'static str> {
    let mut position = *cursor;
    let mut jumped = false;
    let mut visited = Vec::new();
    let mut labels = Vec::new();
    let mut wire_length = 1usize;
    for _ in 0..128 {
        if visited.contains(&position) {
            return Err("DNS compression loop");
        }
        visited.push(position);
        let length = *packet.get(position).ok_or("truncated DNS name")?;
        if length & 0xc0 == 0xc0 {
            let next = *packet.get(position + 1).ok_or("truncated DNS pointer")?;
            if !jumped {
                *cursor = position + 2;
            }
            position = (usize::from(length & 0x3f) << 8) | usize::from(next);
            jumped = true;
            continue;
        }
        if length & 0xc0 != 0 {
            return Err("reserved DNS label");
        }
        position += 1;
        if length == 0 {
            if !jumped {
                *cursor = position;
            }
            return Ok(labels.join(".").to_ascii_lowercase());
        }
        wire_length += usize::from(length) + 1;
        if wire_length > 255 {
            return Err("DNS name exceeds wire limit");
        }
        let label = packet
            .get(position..position + usize::from(length))
            .ok_or("truncated DNS label")?;
        if !label.iter().all(u8::is_ascii) {
            return Err("fixture names must be ASCII");
        }
        labels.push(std::str::from_utf8(label).map_err(|_| "invalid DNS label")?);
        position += usize::from(length);
    }
    Err("DNS compression depth exceeded")
}

struct Question {
    name: String,
    kind: u16,
    end: usize,
}
fn question(packet: &[u8]) -> Result<Question, &'static str> {
    if packet.len() < 12 || packet.len() > PACKET_LIMIT || word(packet, 4)? != 1 {
        return Err("expected one bounded DNS question");
    }
    let mut cursor = 12;
    let name = name(packet, &mut cursor)?;
    let kind = word(packet, cursor)?;
    if word(packet, cursor + 2)? != 1 {
        return Err("fixture expects Internet class");
    }
    Ok(Question {
        name,
        kind,
        end: cursor + 4,
    })
}

fn query(id: u16, domain: &str, kind: u16) -> Vec<u8> {
    let mut packet = vec![0u8; 12];
    packet[..2].copy_from_slice(&id.to_be_bytes());
    packet[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    packet[4..6].copy_from_slice(&1u16.to_be_bytes());
    for label in domain.split('.') {
        assert!(!label.is_empty() && label.len() < 64 && label.is_ascii());
        packet.push(u8::try_from(label.len()).unwrap());
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&kind.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet
}

fn answers(packet: &[u8], id: u16, domain: &str, kind: u16) -> Vec<IpAddr> {
    assert_eq!(
        word(packet, 0).unwrap(),
        id,
        "native DNS transaction identity"
    );
    let flags = word(packet, 2).unwrap();
    assert_eq!(
        flags & 0x820f,
        0x8000,
        "native reply must be complete NOERROR"
    );
    let question = question(packet).unwrap();
    assert_eq!(question.name, domain);
    assert_eq!(question.kind, kind);
    let counts = [
        word(packet, 6).unwrap(),
        word(packet, 8).unwrap(),
        word(packet, 10).unwrap(),
    ];
    assert!(
        counts
            .iter()
            .map(|count| usize::from(*count))
            .sum::<usize>()
            <= 64
    );
    let mut cursor = question.end;
    let mut addresses = Vec::new();
    for (section, count) in counts.into_iter().enumerate() {
        for _ in 0..count {
            let owner = name(packet, &mut cursor).unwrap();
            let record_type = word(packet, cursor).unwrap();
            let class = word(packet, cursor + 2).unwrap();
            let length = usize::from(word(packet, cursor + 8).unwrap());
            cursor += 10;
            let data = packet
                .get(cursor..cursor + length)
                .expect("bounded DNS RDATA");
            if section == 0 && class == 1 {
                assert!(owner == domain || owner.ends_with(&format!(".{SUFFIX}")));
                match (record_type, length) {
                    (1, 4) => addresses.push(IpAddr::V4(Ipv4Addr::new(
                        data[0], data[1], data[2], data[3],
                    ))),
                    (28, 16) => addresses.push(IpAddr::V6(Ipv6Addr::from(
                        <[u8; 16]>::try_from(data).unwrap(),
                    ))),
                    (5, _) => {
                        let mut alias_cursor = cursor;
                        let alias = name(packet, &mut alias_cursor).unwrap();
                        assert_eq!(alias_cursor, cursor + length);
                        assert!(alias.ends_with(&format!(".{SUFFIX}")));
                    }
                    _ => panic!("unexpected fixture answer kind or length"),
                }
            }
            cursor += length;
        }
    }
    assert_eq!(
        cursor,
        packet.len(),
        "all DNS sections must be bounded and consumed"
    );
    assert!(
        !addresses.is_empty(),
        "native DNS must return actual fixture address"
    );
    addresses
}

struct Resolver {
    address: SocketAddr,
    hits: Arc<Mutex<BTreeMap<(String, u16), usize>>>,
    task: JoinHandle<()>,
}
impl Resolver {
    async fn start(index: u8) -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = socket.local_addr().unwrap();
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let recorded = hits.clone();
        let task = tokio::spawn(async move {
            let mut buffer = [0u8; PACKET_LIMIT];
            // Only local synthetic fixture queries, with a finite request cap.
            for _ in 0..128 {
                let Ok((length, peer)) = socket.recv_from(&mut buffer).await else {
                    break;
                };
                let packet = &buffer[..length];
                let Ok(question) = question(packet) else {
                    continue;
                };
                if packet[2] & 0x80 != 0 || !question.name.ends_with(&format!(".{SUFFIX}")) {
                    continue;
                }
                *recorded
                    .lock()
                    .unwrap()
                    .entry((question.name.clone(), question.kind))
                    .or_default() += 1;
                let fail = index == 1 && question.name == "fallback.fixture.invalid";
                let no_name = question.name == "nxdomain.fixture.invalid";
                let mut reply = packet[..question.end].to_vec();
                let flags: u16 = if fail {
                    0x8182
                } else if no_name {
                    0x8183
                } else {
                    0x8180
                };
                reply[2..4].copy_from_slice(&flags.to_be_bytes());
                reply[6..12].fill(0);
                if !fail && !no_name && matches!(question.kind, 1 | 28) {
                    reply[6..8].copy_from_slice(&1u16.to_be_bytes());
                    reply.extend_from_slice(&[0xc0, 0x0c]);
                    reply.extend_from_slice(&question.kind.to_be_bytes());
                    reply.extend_from_slice(&1u16.to_be_bytes());
                    reply.extend_from_slice(&30u32.to_be_bytes());
                    if question.kind == 1 {
                        reply.extend_from_slice(&4u16.to_be_bytes());
                        reply.extend_from_slice(&[203, 0, 113, 100 + index]);
                    } else {
                        reply.extend_from_slice(&16u16.to_be_bytes());
                        reply.extend_from_slice(
                            &Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, u16::from(100 + index))
                                .octets(),
                        );
                    }
                }
                let _ = socket.send_to(&reply, peer).await;
            }
        });
        Self {
            address,
            hits,
            task,
        }
    }
    fn count(&self, domain: &str, kind: u16) -> usize {
        self.hits
            .lock()
            .unwrap()
            .get(&(domain.to_owned(), kind))
            .copied()
            .unwrap_or(0)
    }
    fn total(&self) -> usize {
        self.hits.lock().unwrap().values().sum()
    }
    fn url(&self) -> String {
        format!("udp://{}", self.address)
    }
}
impl Drop for Resolver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct DnsClient {
    address: SocketAddr,
    next_id: u16,
}
impl DnsClient {
    async fn exchange(&mut self, domain: &str, kind: u16, tcp: bool) -> Vec<u8> {
        self.next_id = self.next_id.checked_add(1).unwrap();
        let packet = query(self.next_id, domain, kind);
        tokio::time::timeout(QUERY_TIMEOUT, async {
            if tcp {
                let mut stream = TcpStream::connect(self.address).await.unwrap();
                stream
                    .write_u16(u16::try_from(packet.len()).unwrap())
                    .await
                    .unwrap();
                stream.write_all(&packet).await.unwrap();
                let length = usize::from(stream.read_u16().await.unwrap());
                assert!((12..=PACKET_LIMIT).contains(&length));
                let mut reply = vec![0; length];
                stream.read_exact(&mut reply).await.unwrap();
                reply
            } else {
                let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
                socket.connect(self.address).await.unwrap();
                socket.send(&packet).await.unwrap();
                let mut reply = vec![0; PACKET_LIMIT];
                let length = socket.recv(&mut reply).await.unwrap();
                reply.truncate(length);
                reply
            }
        })
        .await
        .expect("actual native DNS exchange deadline")
    }
    async fn answer(&mut self, domain: &str, kind: u16, tcp: bool, expected: IpAddr) {
        let response = self.exchange(domain, kind, tcp).await;
        assert_eq!(
            answers(&response, self.next_id, domain, kind),
            vec![expected]
        );
    }
}

async fn reserve_listener() -> (SocketAddr, UdpSocket, TcpListener) {
    for _ in 0..16 {
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = udp.local_addr().unwrap();
        if let Ok(tcp) = TcpListener::bind(address).await {
            return (address, udp, tcp);
        }
    }
    panic!("could not reserve matching synthetic UDP/TCP DNS listener");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Root-built pinned Go host c-shared and PAWS_SING_HOST_LIBRARY; actual sockets"]
async fn native_dns_uses_real_resolvers_policy_hosts_cache_and_owner() {
    let library = NativeLibrary::get();
    library.register_real_compiler();
    let root = TestRoot::new();
    let a = Resolver::start(1).await;
    let b = Resolver::start(2).await;
    let (address, udp_reservation, tcp_reservation) = reserve_listener().await;
    let yaml = format!(
        r#"
proxies: []
proxy-groups: []
rules: ['MATCH,DIRECT']
hosts:
  host.fixture.invalid: [203.0.113.77, '2001:db8::77']
  '+.hosts.fixture.invalid': 203.0.113.78
  '+.specific.hosts.fixture.invalid': 203.0.113.79
  alias.fixture.invalid: canonical.fixture.invalid
dns:
  enable: true
  ipv6: true
  listen: '{address}'
  use-hosts: true
  nameserver: ['{primary}']
  fallback: ['{fallback}']
  default-nameserver: ['{primary}']
  proxy-server-nameserver: ['{primary}']
  nameserver-policy:
    '+.ordered.fixture.invalid': ['{fallback}']
    '*.ordered.fixture.invalid': ['{primary}']
    '*.star.fixture.invalid': ['{fallback}']
    '+.star.fixture.invalid': ['{primary}']
"#,
        primary = a.url(),
        fallback = b.url()
    );
    let options = VpnOptions {
        engine: VpnEngine::SingBox,
        ipv6: true,
        dns_hijacking: false,
        dns_servers: vec![a.url()],
        dns_fallbacks: Vec::new(),
        dns_nameserver_policy: BTreeMap::new(),
        ..Default::default()
    };
    let mut services = EngineNetworkServices::default();
    services.ports.mixed_enabled = false;
    services.ports.controller_enabled = false;
    let rendered = crate::config::render_sing_box_runtime_json(
        &yaml,
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &options,
        &services,
    )
    .unwrap();
    let config = crate::config::secure_controller_config(
        VpnEngine::SingBox,
        &rendered.config_json,
        "synthetic-native-dns-owner",
    )
    .unwrap();
    let endpoint = crate::config::controller_endpoint(VpnEngine::SingBox, &config).unwrap();
    let directory = root.0.as_os_str().as_bytes();
    drop(udp_reservation);
    drop(tcp_reservation);
    assert_eq!(
        unsafe {
            (library.start)(
                config.as_ptr(),
                config.len(),
                directory.as_ptr(),
                directory.len(),
            )
        },
        0,
        "actual native DNS start failed: {}",
        library.last_error()
    );
    let mut owner = RunningOwner {
        library,
        running: true,
    };
    let controller = crate::ControllerClient::new(VpnEngine::SingBox, endpoint).unwrap();
    let live = controller.dns_config().await.unwrap();
    assert_eq!(live.listen.as_deref(), Some(address.to_string().as_str()));
    assert_eq!(live.upstreams, vec![a.url()]);
    assert_eq!(live.fallbacks, vec![b.url()]);
    let before = controller.dns_statistics().await.unwrap();
    let mut client = DnsClient {
        address,
        next_id: 400,
    };
    let ip_a = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 101));
    let ip_b = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 102));

    client.answer("cache.fixture.invalid", 1, false, ip_a).await;
    assert_eq!(a.count("cache.fixture.invalid", 1), 1);
    assert_eq!(b.count("cache.fixture.invalid", 1), 0);
    let missed = controller.dns_statistics().await.unwrap();
    assert!(missed.cache_misses > before.cache_misses);
    client.answer("cache.fixture.invalid", 1, true, ip_a).await;
    assert_eq!(
        a.count("cache.fixture.invalid", 1),
        1,
        "TCP query must use the native UDP-query cache"
    );
    let cached = controller.dns_statistics().await.unwrap();
    assert!(cached.cache_hits > missed.cache_hits);
    assert!(cached.handled_packets >= before.handled_packets + 2);
    assert!(cached
        .recent_queries
        .iter()
        .any(|query| query.name == "cache.fixture.invalid"));
    controller.flush_dns(false).await.unwrap();
    client.answer("cache.fixture.invalid", 1, false, ip_a).await;
    assert_eq!(
        a.count("cache.fixture.invalid", 1),
        2,
        "real flush must force an actual upstream re-query"
    );

    client
        .answer("fallback.fixture.invalid", 1, false, ip_b)
        .await;
    assert_eq!(a.count("fallback.fixture.invalid", 1), 1);
    assert_eq!(b.count("fallback.fixture.invalid", 1), 1);
    // A valid NXDOMAIN is a result, not a transport failure to hide via fallback.
    let negative = client.exchange("nxdomain.fixture.invalid", 1, false).await;
    assert_eq!(word(&negative, 0).unwrap(), client.next_id);
    assert_eq!(word(&negative, 2).unwrap() & 0x800f, 0x8003);
    assert_eq!(b.count("nxdomain.fixture.invalid", 1), 0);

    client
        .answer("leaf.ordered.fixture.invalid", 1, false, ip_b)
        .await;
    assert_eq!(
        a.count("leaf.ordered.fixture.invalid", 1),
        0,
        "first matching YAML policy must win"
    );
    assert_eq!(b.count("leaf.ordered.fixture.invalid", 1), 1);
    client.answer("star.fixture.invalid", 1, false, ip_a).await;
    client
        .answer("leaf.star.fixture.invalid", 1, false, ip_b)
        .await;
    assert_eq!(
        a.count("star.fixture.invalid", 1),
        1,
        "*. must not match the suffix root"
    );
    assert_eq!(b.count("leaf.star.fixture.invalid", 1), 1);

    let upstream_before_hosts = a.total() + b.total();
    client
        .answer(
            "host.fixture.invalid",
            1,
            false,
            "203.0.113.77".parse().unwrap(),
        )
        .await;
    client
        .answer(
            "host.fixture.invalid",
            28,
            true,
            "2001:db8::77".parse().unwrap(),
        )
        .await;
    client
        .answer(
            "leaf.hosts.fixture.invalid",
            1,
            false,
            "203.0.113.78".parse().unwrap(),
        )
        .await;
    client
        .answer(
            "leaf.specific.hosts.fixture.invalid",
            1,
            false,
            "203.0.113.79".parse().unwrap(),
        )
        .await;
    assert_eq!(
        a.total() + b.total(),
        upstream_before_hosts,
        "literal hosts must not reach either resolver"
    );
    client.answer("alias.fixture.invalid", 1, false, ip_a).await;
    assert_eq!(a.count("canonical.fixture.invalid", 1), 1);
    assert_eq!(a.count("alias.fixture.invalid", 1), 0);
    client
        .answer(
            "aaaa.fixture.invalid",
            28,
            false,
            "2001:db8::65".parse().unwrap(),
        )
        .await;
    assert_eq!(a.count("aaaa.fixture.invalid", 28), 1);

    let final_stats = controller.dns_statistics().await.unwrap();
    owner.running = false;
    let stopped = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || unsafe { (library.stop)() }),
    )
    .await
    .expect("actual native DNS stop/drain deadline")
    .unwrap();
    assert_eq!(
        stopped,
        0,
        "native DNS stop failed: {}",
        library.last_error()
    );
    assert!(controller.dns_statistics().await.is_err());
    let _udp_released = UdpSocket::bind(address)
        .await
        .expect("stop must release actual DNS UDP socket");
    let _tcp_released = TcpListener::bind(address)
        .await
        .expect("stop must release actual DNS TCP socket");
    println!("NATIVE_DNS_ACCEPTANCE handled={} hits={} misses={} resolverA={} resolverB={} udpTcp=true policyHosts=true flush=true released=true",
        final_stats.handled_packets, final_stats.cache_hits, final_stats.cache_misses, a.total(), b.total());
}

async fn fake_ip_reverse(
    client: &reqwest::Client,
    endpoint: &paws_engine::EngineControllerEndpoint,
    address: IpAddr,
) -> Option<String> {
    let mut url = reqwest::Url::parse(&format!("http://{}/", endpoint.address)).unwrap();
    url.path_segments_mut().unwrap().extend([
        "paws",
        "observations",
        "fakeip",
        &address.to_string(),
    ]);
    let response = client
        .get(url)
        .bearer_auth(endpoint.secret.as_ref().expect("captured owner credential"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let value: serde_json::Value = response.json().await.unwrap();
    match value.get("domain") {
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(domain)) => Some(domain.clone()),
        _ => panic!("actual native fake-IP Store.Lookup response must contain domain|null"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires new Root-built native fake-IP host shared library; actual mapping and Store reset"]
async fn native_fake_ip_mapping_filter_and_reset_use_actual_store() {
    let library = NativeLibrary::get();
    library.register_real_compiler();
    let root = TestRoot::new();
    let a = Resolver::start(1).await;
    let b = Resolver::start(2).await;
    let (address, udp_reservation, tcp_reservation) = reserve_listener().await;
    // This is the production profile renderer, not a hand-written equivalent
    // native DNS graph. The compiled native child transport owns all mappings.
    let yaml = format!(
        r#"
proxies: []
proxy-groups: []
rules: ['MATCH,DIRECT']
hosts:
  host.fixture.invalid: 203.0.113.77
dns:
  enable: true
  ipv6: true
  listen: '{address}'
  use-hosts: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-range6: 'fc00::/64'
  fake-ip-filter-mode: blacklist
  fake-ip-filter: ['+.real.fixture.invalid', '+.policy.fixture.invalid']
  nameserver: ['{primary}']
  fallback: ['{fallback}']
  default-nameserver: ['{primary}']
  proxy-server-nameserver: ['{primary}']
  nameserver-policy:
    '+.policy.fixture.invalid': ['{fallback}']
"#,
        primary = a.url(),
        fallback = b.url()
    );
    let options = VpnOptions {
        engine: VpnEngine::SingBox,
        ipv6: true,
        dns_hijacking: false,
        dns_servers: vec![a.url()],
        dns_fallbacks: Vec::new(),
        dns_nameserver_policy: BTreeMap::new(),
        ..Default::default()
    };
    let mut services = EngineNetworkServices::default();
    services.ports.mixed_enabled = false;
    services.ports.controller_enabled = false;
    let rendered = crate::config::render_sing_box_runtime_json(
        &yaml,
        RuntimeMode::Rule,
        &BTreeMap::new(),
        &options,
        &services,
    )
    .unwrap();
    let config = crate::config::secure_controller_config(
        VpnEngine::SingBox,
        &rendered.config_json,
        "synthetic-native-fake-ip-owner",
    )
    .unwrap();
    let endpoint = crate::config::controller_endpoint(VpnEngine::SingBox, &config).unwrap();
    let directory = root.0.as_os_str().as_bytes();
    drop(udp_reservation);
    drop(tcp_reservation);
    assert_eq!(
        unsafe {
            (library.start)(
                config.as_ptr(),
                config.len(),
                directory.as_ptr(),
                directory.len(),
            )
        },
        0,
        "actual native fake-IP start failed: {}",
        library.last_error()
    );
    let mut owner = RunningOwner {
        library,
        running: true,
    };
    let controller = crate::ControllerClient::new(VpnEngine::SingBox, endpoint.clone()).unwrap();
    let dns_config = controller.dns_config().await.unwrap();
    assert_eq!(
        dns_config.model, "fake-ip",
        "DNS domain must expose the applied mode, not an engine-name template"
    );
    assert_eq!(
        dns_config.listen.as_deref(),
        Some(address.to_string().as_str())
    );
    assert_eq!(dns_config.upstreams, vec![a.url()]);
    assert_eq!(dns_config.fallbacks, vec![b.url()]);
    assert_eq!(
        dns_config.nameserver_policy.get("+.policy.fixture.invalid"),
        Some(&vec![b.url()])
    );
    let private = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(QUERY_TIMEOUT)
        .build()
        .unwrap();
    let mut client = DnsClient {
        address,
        next_id: 800,
    };
    let initial_upstream = a.total() + b.total();
    let first = "first.fake.fixture.invalid";
    let packet = client.exchange(first, 1, false).await;
    let ipv4 = answers(&packet, client.next_id, first, 1);
    assert_eq!(ipv4.len(), 1);
    let IpAddr::V4(ipv4_address) = ipv4[0] else {
        panic!("fake A returned non-IPv4")
    };
    assert_eq!(ipv4_address.octets()[..2], [198, 18]);
    client.answer(first, 1, true, ipv4[0]).await;
    let packet = client.exchange(first, 28, false).await;
    let ipv6 = answers(&packet, client.next_id, first, 28);
    assert_eq!(ipv6.len(), 1);
    let IpAddr::V6(ipv6_address) = ipv6[0] else {
        panic!("fake AAAA returned non-IPv6")
    };
    assert_eq!(ipv6_address.segments()[..4], [0xfc00, 0, 0, 0]);
    client.answer(first, 28, true, ipv6[0]).await;
    assert_eq!(
        a.total() + b.total(),
        initial_upstream,
        "fake mappings must not query either real resolver"
    );
    assert_eq!(
        fake_ip_reverse(&private, &endpoint, ipv4[0])
            .await
            .as_deref(),
        Some(first)
    );
    assert_eq!(
        fake_ip_reverse(&private, &endpoint, ipv6[0])
            .await
            .as_deref(),
        Some(first)
    );

    client
        .answer(
            "host.fixture.invalid",
            1,
            false,
            "203.0.113.77".parse().unwrap(),
        )
        .await;
    assert_eq!(
        a.total() + b.total(),
        initial_upstream,
        "hosts must take precedence over fake-IP"
    );
    client
        .answer(
            "leaf.real.fixture.invalid",
            1,
            false,
            "203.0.113.101".parse().unwrap(),
        )
        .await;
    assert_eq!(a.count("leaf.real.fixture.invalid", 1), 1);
    assert_eq!(b.count("leaf.real.fixture.invalid", 1), 0);
    client
        .answer(
            "leaf.policy.fixture.invalid",
            1,
            false,
            "203.0.113.102".parse().unwrap(),
        )
        .await;
    assert_eq!(a.count("leaf.policy.fixture.invalid", 1), 0);
    assert_eq!(b.count("leaf.policy.fixture.invalid", 1), 1);

    controller.flush_dns(true).await.unwrap();
    // Query Store.Lookup before allocating again: a reset pool may legitimately
    // reuse an IP. A changed answer alone would not prove reverse-map removal.
    assert!(fake_ip_reverse(&private, &endpoint, ipv4[0])
        .await
        .is_none());
    assert!(fake_ip_reverse(&private, &endpoint, ipv6[0])
        .await
        .is_none());
    let second = "second.fake.fixture.invalid";
    let packet = client.exchange(second, 1, false).await;
    let remapped = answers(&packet, client.next_id, second, 1);
    assert_eq!(remapped.len(), 1);
    let IpAddr::V4(remapped_address) = remapped[0] else {
        panic!("remapped A returned non-IPv4")
    };
    assert_eq!(remapped_address.octets()[..2], [198, 18]);
    assert_eq!(
        fake_ip_reverse(&private, &endpoint, remapped[0])
            .await
            .as_deref(),
        Some(second)
    );
    client.answer(second, 1, true, remapped[0]).await;
    assert_eq!(a.count(second, 1) + b.count(second, 1), 0);
    owner.running = false;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || unsafe { (library.stop)() })
        )
        .await
        .expect("actual native fake-IP stop deadline")
        .unwrap(),
        0
    );
    assert!(controller.dns_config().await.is_err());
    let _udp_released = UdpSocket::bind(address)
        .await
        .expect("fake-IP owner released UDP listener");
    let _tcp_released = TcpListener::bind(address)
        .await
        .expect("fake-IP owner released TCP listener");
    println!("NATIVE_FAKE_IP_ACCEPTANCE aRepeat=true aaaaRepeat=true reverseActual=true filterHostsPolicy=true oldReverseGone=true newMapping=true stop=true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Root-built Sing08 host shared library; actual wildcard DNS callback and sockets"]
async fn native_dns_catch_all_policy_and_fake_ip_filter_use_real_callback() {
    let library = NativeLibrary::get();
    library.register_real_compiler();
    // Separate owners prove the policy without fake-IP first, then prove that
    // the catch-all blacklist actually reaches that same real resolver path.
    // The old DNS/fake-IP cases and their upstream counters remain unchanged.
    for mode in ["redir-host", "fake-ip"] {
        let root = TestRoot::new();
        let a = Resolver::start(1).await;
        let b = Resolver::start(2).await;
        let (address, udp_reservation, tcp_reservation) = reserve_listener().await;
        let yaml = format!(
            r#"
proxies: []
proxy-groups: []
rules: ['MATCH,DIRECT']
dns:
  enable: true
  ipv6: true
  listen: '{address}'
  enhanced-mode: {mode}
  fake-ip-range: 198.18.0.1/16
  fake-ip-range6: 'fc00::/64'
  fake-ip-filter-mode: blacklist
  fake-ip-filter: ['*']
  nameserver: ['{primary}']
  fallback: []
  default-nameserver: ['{primary}']
  proxy-server-nameserver: ['{primary}']
  nameserver-policy:
    '+.specific.fixture.invalid': ['{primary}']
    '*': ['{catch_all}']
"#,
            primary = a.url(),
            catch_all = b.url()
        );
        let options = VpnOptions {
            engine: VpnEngine::SingBox,
            ipv6: true,
            dns_hijacking: false,
            dns_servers: vec![a.url()],
            dns_fallbacks: Vec::new(),
            dns_nameserver_policy: BTreeMap::new(),
            ..Default::default()
        };
        let mut services = EngineNetworkServices::default();
        services.ports.mixed_enabled = false;
        services.ports.controller_enabled = false;
        let rendered = crate::config::render_sing_box_runtime_json(
            &yaml,
            RuntimeMode::Rule,
            &BTreeMap::new(),
            &options,
            &services,
        )
        .unwrap();
        let config = crate::config::secure_controller_config(
            VpnEngine::SingBox,
            &rendered.config_json,
            "synthetic-native-dns-star-owner",
        )
        .unwrap();
        let endpoint = crate::config::controller_endpoint(VpnEngine::SingBox, &config).unwrap();
        let directory = root.0.as_os_str().as_bytes();
        drop(udp_reservation);
        drop(tcp_reservation);
        assert_eq!(
            unsafe {
                (library.start)(
                    config.as_ptr(),
                    config.len(),
                    directory.as_ptr(),
                    directory.len(),
                )
            },
            0,
            "actual native {mode} catch-all start failed: {}",
            library.last_error()
        );
        let mut owner = RunningOwner {
            library,
            running: true,
        };
        let controller =
            crate::ControllerClient::new(VpnEngine::SingBox, endpoint.clone()).unwrap();
        let live = controller.dns_config().await.unwrap();
        assert_eq!(live.model, mode);
        assert_eq!(live.listen.as_deref(), Some(address.to_string().as_str()));
        assert_eq!(live.nameserver_policy.get("*"), Some(&vec![b.url()]));
        let private = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(QUERY_TIMEOUT)
            .build()
            .unwrap();
        let mut client = DnsClient {
            address,
            next_id: 1200,
        };
        let probes = [
            (
                "catch-all.fixture.invalid",
                1,
                false,
                "203.0.113.102",
                false,
            ),
            ("catch-all.fixture.invalid", 28, true, "2001:db8::66", false),
            (
                "leaf.specific.fixture.invalid",
                1,
                true,
                "203.0.113.101",
                true,
            ),
            (
                "leaf.specific.fixture.invalid",
                28,
                false,
                "2001:db8::65",
                true,
            ),
        ];
        for (domain, kind, tcp, expected, specific) in probes {
            let expected = expected.parse::<IpAddr>().unwrap();
            client.answer(domain, kind, tcp, expected).await;
            assert_eq!(
                a.count(domain, kind),
                usize::from(specific),
                "{mode}: an earlier specific policy must precede the catch-all"
            );
            assert_eq!(
                b.count(domain, kind),
                usize::from(!specific),
                "{mode}: unmatched names must actually reach catch-all resolver B"
            );
            if mode == "fake-ip" {
                assert!(
                    fake_ip_reverse(&private, &endpoint, expected).await.is_none(),
                    "a real address returned through '*' must not acquire a fake-IP reverse mapping"
                );
            }
        }
        if mode == "fake-ip" {
            // These are the first actual allocation slots observed by the
            // separate positive mapping case. This is a bounded Store.Lookup
            // negative control, not an assertion that the whole pool was dumped.
            for address in ["198.18.0.3", "fc00::2"] {
                assert!(
                    fake_ip_reverse(&private, &endpoint, address.parse().unwrap())
                        .await
                        .is_none()
                );
            }
        }
        owner.running = false;
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(3),
                tokio::task::spawn_blocking(move || unsafe { (library.stop)() })
            )
            .await
            .expect("actual native catch-all DNS stop deadline")
            .unwrap(),
            0
        );
        assert!(controller.dns_config().await.is_err());
        let _udp_released = UdpSocket::bind(address)
            .await
            .expect("catch-all owner released UDP listener");
        let _tcp_released = TcpListener::bind(address)
            .await
            .expect("catch-all owner released TCP listener");
        println!("NATIVE_DNS_STAR_ACCEPTANCE mode={mode} specificBeforeStar=true resolverA={} resolverB={} udpTcp=true realAnswers=true stop=true",
            a.total(), b.total());
    }
}

#[test]
fn dns_wire_reader_accepts_compression_and_rejects_pointer_loops() {
    let request = query(7, "wire.fixture.invalid", 1);
    let mut compressed = request.clone();
    compressed.extend_from_slice(&[0xc0, 0x0c]);
    let mut cursor = request.len();
    assert_eq!(
        name(&compressed, &mut cursor).unwrap(),
        "wire.fixture.invalid"
    );
    assert_eq!(cursor, compressed.len());
    let mut cursor = 0;
    assert!(name(&[0xc0, 0x00], &mut cursor).is_err());
    let mut cursor = 0;
    assert!(name(&[0xc0, 0xff], &mut cursor).is_err());
}

#[test]
fn dns_wire_reader_accepts_uncompressed_answer_and_standard_additional() {
    let mut response = query(8, "wire.fixture.invalid", 1);
    let wire_name = response[12..response.len() - 4].to_vec();
    response[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
    response[6..8].copy_from_slice(&1u16.to_be_bytes());
    response[10..12].copy_from_slice(&1u16.to_be_bytes());
    response.extend_from_slice(&wire_name);
    response.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 203, 0, 113, 101]);
    // A standards-compliant additional OPT is not an extra fixture A answer.
    response.extend_from_slice(&[0, 0, 41, 16, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        answers(&response, 8, "wire.fixture.invalid", 1),
        vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 101))]
    );
}
