use std::env;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

const VLESS_UUID: [u8; 16] = [
    0xb8, 0x31, 0x38, 0x1d, 0x63, 0x24, 0x4d, 0x53, 0xad, 0x4f, 0x8c, 0xda, 0x48, 0xb3, 0x08, 0x11,
];
const TROJAN_PASSWORD: &str = "test-trojan-password";
const SS_PASSWORD: &str = "test-shadowsocks-password";
const HTTP_AUTH_HEADER: &str = "Proxy-Authorization: Basic YWxpY2U6czNjcjN0";
const SOCKS5_USER: &[u8] = b"bob";
const SOCKS5_PASS: &[u8] = b"hunter2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Direct,
    Http,
    HttpAuth,
    HttpBadAuth,
    HttpDown,
    Socks5,
    Socks5Auth,
    Socks5BadAuth,
    Shadowsocks,
    ShadowsocksBadPassword,
    Trojan,
    TrojanBadPassword,
    Vless,
    VlessBadUuid,
}

impl Mode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "http" => Some(Self::Http),
            "http-auth" => Some(Self::HttpAuth),
            "http-bad-auth" => Some(Self::HttpBadAuth),
            "http-down" => Some(Self::HttpDown),
            "socks5" => Some(Self::Socks5),
            "socks5-auth" => Some(Self::Socks5Auth),
            "socks5-bad-auth" => Some(Self::Socks5BadAuth),
            "ss" => Some(Self::Shadowsocks),
            "ss-bad-password" => Some(Self::ShadowsocksBadPassword),
            "trojan" => Some(Self::Trojan),
            "trojan-bad-password" => Some(Self::TrojanBadPassword),
            "vless" => Some(Self::Vless),
            "vless-bad-uuid" => Some(Self::VlessBadUuid),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Http => "http",
            Self::HttpAuth => "http-auth",
            Self::HttpBadAuth => "http-bad-auth",
            Self::HttpDown => "http-down",
            Self::Socks5 => "socks5",
            Self::Socks5Auth => "socks5-auth",
            Self::Socks5BadAuth => "socks5-bad-auth",
            Self::Shadowsocks => "ss",
            Self::ShadowsocksBadPassword => "ss-bad-password",
            Self::Trojan => "trojan",
            Self::TrojanBadPassword => "trojan-bad-password",
            Self::Vless => "vless",
            Self::VlessBadUuid => "vless-bad-uuid",
        }
    }

    fn proxy_name(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Http => "HTTP-MOCK",
            Self::HttpAuth => "HTTP-AUTH-MOCK",
            Self::HttpBadAuth => "HTTP-BAD-AUTH-MOCK",
            Self::HttpDown => "HTTP-DOWN-MOCK",
            Self::Socks5 => "SOCKS5-MOCK",
            Self::Socks5Auth => "SOCKS5-AUTH-MOCK",
            Self::Socks5BadAuth => "SOCKS5-BAD-AUTH-MOCK",
            Self::Shadowsocks => "SS-MOCK",
            Self::ShadowsocksBadPassword => "SS-BAD-PASSWORD-MOCK",
            Self::Trojan => "TROJAN-MOCK",
            Self::TrojanBadPassword => "TROJAN-BAD-PASSWORD-MOCK",
            Self::Vless => "VLESS-MOCK",
            Self::VlessBadUuid => "VLESS-BAD-UUID-MOCK",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse()?;
    let echo_addr = start_tcp_echo_server(args.bind).await?;
    let proxy_addr = match args.mode {
        Mode::Direct => None,
        Mode::Http => Some(start_http_connect_proxy(args.bind, None).await?),
        Mode::HttpAuth | Mode::HttpBadAuth => {
            Some(start_http_connect_proxy(args.bind, Some(HTTP_AUTH_HEADER)).await?)
        }
        Mode::HttpDown => Some(reserve_unbound_addr(args.bind).await?),
        Mode::Socks5 => Some(start_socks5_proxy(args.bind, None).await?),
        Mode::Socks5Auth | Mode::Socks5BadAuth => {
            Some(start_socks5_proxy(args.bind, Some((SOCKS5_USER, SOCKS5_PASS))).await?)
        }
        Mode::Shadowsocks | Mode::ShadowsocksBadPassword => {
            Some(start_shadowsocks_proxy(args.bind).await?)
        }
        Mode::Trojan | Mode::TrojanBadPassword => Some(start_trojan_proxy(args.bind).await?),
        Mode::Vless | Mode::VlessBadUuid => Some(start_vless_proxy(args.bind).await?),
    };
    let echo_target = HostPort {
        host: args.advertise_host.clone(),
        port: echo_addr.port(),
    };
    let proxy_target = proxy_addr.map(|addr| HostPort {
        host: args.advertise_host.clone(),
        port: addr.port(),
    });

    let profile_path = write_profile(
        args.mode,
        &echo_target,
        proxy_target.as_ref(),
        args.output.as_deref(),
    )?;
    println!("mode: {}", args.mode);
    println!("proxy name: {}", args.mode.proxy_name());
    println!("bind address: {}", args.bind);
    println!("echo target: {}", echo_target);
    if let Some(proxy_target) = &proxy_target {
        println!("mock proxy: {}", proxy_target);
    }
    println!("generated profile: {}", profile_path.display());
    println!();
    println!("Import that profile into the app while this process stays running.");
    println!(
        "For native delay checks use: proxy={}, url=http://{}",
        args.mode.proxy_name(),
        echo_target
    );
    if let Some(ms) = args.keepalive_ms {
        println!("Auto-stop after {ms} ms.");
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        return Ok(());
    }

    println!("Press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    output: Option<PathBuf>,
    keepalive_ms: Option<u64>,
    bind: IpAddr,
    advertise_host: String,
}

impl Args {
    fn parse() -> io::Result<Self> {
        let mut raw = env::args().skip(1);
        let Some(mode_text) = raw.next() else {
            print_usage();
            return Err(invalid_input("missing mode"));
        };
        let Some(mode) = Mode::parse(&mode_text) else {
            print_usage();
            return Err(invalid_input(format!("unknown mode: {mode_text}")));
        };

        let mut output = None;
        let mut keepalive_ms = None;
        let mut bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut advertise_host = None;
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--profile-out" | "-o" => {
                    let Some(path) = raw.next() else {
                        return Err(invalid_input("--profile-out requires a path"));
                    };
                    output = Some(PathBuf::from(path));
                }
                "--keepalive-ms" => {
                    let Some(value) = raw.next() else {
                        return Err(invalid_input("--keepalive-ms requires a value"));
                    };
                    keepalive_ms = Some(value.parse::<u64>().map_err(|err| {
                        invalid_input(format!("invalid --keepalive-ms value: {err}"))
                    })?);
                }
                "--bind" => {
                    let Some(value) = raw.next() else {
                        return Err(invalid_input("--bind requires an IP address"));
                    };
                    bind = value
                        .parse::<IpAddr>()
                        .map_err(|err| invalid_input(format!("invalid --bind value: {err}")))?;
                }
                "--advertise-host" => {
                    let Some(value) = raw.next() else {
                        return Err(invalid_input("--advertise-host requires a host"));
                    };
                    advertise_host = Some(validate_advertise_host(&value)?);
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => return Err(invalid_input(format!("unknown argument: {other}"))),
            }
        }

        Ok(Self {
            mode,
            output,
            keepalive_ms,
            bind,
            advertise_host: advertise_host.unwrap_or_else(|| bind.to_string()),
        })
    }
}

fn print_usage() {
    eprintln!(
        "usage: cargo run --manifest-path local-protocol-tests/Cargo.toml -- <direct|http|http-auth|http-bad-auth|http-down|socks5|socks5-auth|socks5-bad-auth|ss|ss-bad-password|trojan|trojan-bad-password|vless|vless-bad-uuid> [--profile-out path] [--keepalive-ms ms] [--bind ip] [--advertise-host host]"
    );
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn validate_advertise_host(raw: &str) -> io::Result<String> {
    let host = raw.trim();
    if host.is_empty() {
        return Err(invalid_input("--advertise-host cannot be empty"));
    }
    if host.chars().any(char::is_whitespace) {
        return Err(invalid_input(
            "--advertise-host must be one IP address or hostname; the supplied value contains whitespace (check that the command did not also capture the IPv6 status line)",
        ));
    }
    if host.parse::<IpAddr>().is_ok() {
        return Ok(host.to_owned());
    }
    let valid_hostname = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !valid_hostname {
        return Err(invalid_input(format!(
            "invalid --advertise-host value: {host:?}"
        )));
    }
    Ok(host.to_owned())
}

async fn start_tcp_echo_server(bind: IpAddr) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = echo_tcp(stream).await;
            });
        }
    });
    Ok(addr)
}

async fn reserve_unbound_addr(bind: IpAddr) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

async fn echo_tcp(mut stream: TcpStream) -> io::Result<()> {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            return Ok(());
        }
        stream.write_all(&buffer[..n]).await?;
    }
}

async fn start_http_connect_proxy(
    bind: IpAddr,
    required_auth_header: Option<&'static str>,
) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = handle_http_connect(stream, required_auth_header).await;
            });
        }
    });
    Ok(addr)
}

async fn handle_http_connect(
    mut inbound: TcpStream,
    required_auth_header: Option<&str>,
) -> io::Result<()> {
    let request = read_until_double_crlf(&mut inbound).await?;
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let authority = parts.next().unwrap_or_default();
    if !method.eq_ignore_ascii_case("CONNECT") {
        inbound
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            .await?;
        return Ok(());
    }
    if let Some(required) = required_auth_header {
        if !request.contains(required) {
            inbound
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await?;
            return Ok(());
        }
        if !request.contains("X-Paws-Test: local-protocol") {
            inbound
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await?;
            return Ok(());
        }
    }
    let target = parse_host_port(authority, 443)?;
    let mut outbound = TcpStream::connect(target).await?;
    inbound
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    Ok(())
}

async fn read_until_double_crlf(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(1024);
    let mut one = [0_u8; 1];
    while bytes.len() < 16 * 1024 {
        let n = stream.read(&mut one).await?;
        if n == 0 {
            break;
        }
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn start_socks5_proxy(
    bind: IpAddr,
    required_auth: Option<(&'static [u8], &'static [u8])>,
) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = handle_socks5(stream, required_auth).await;
            });
        }
    });
    Ok(addr)
}

async fn handle_socks5(
    mut inbound: TcpStream,
    required_auth: Option<(&[u8], &[u8])>,
) -> io::Result<()> {
    let mut greeting = [0_u8; 2];
    inbound.read_exact(&mut greeting).await?;
    if greeting[0] != 0x05 {
        return Ok(());
    }
    let mut methods = vec![0_u8; greeting[1] as usize];
    inbound.read_exact(&mut methods).await?;
    let method = if required_auth.is_some() {
        0x02
    } else if methods.contains(&0x00) {
        0x00
    } else {
        0xff
    };
    inbound.write_all(&[0x05, method]).await?;
    if method == 0xff {
        return Ok(());
    }

    if let Some((expected_user, expected_pass)) = required_auth {
        let mut auth_hdr = [0_u8; 2];
        inbound.read_exact(&mut auth_hdr).await?;
        if auth_hdr[0] != 0x01 {
            return Ok(());
        }
        let mut user = vec![0_u8; auth_hdr[1] as usize];
        inbound.read_exact(&mut user).await?;
        let mut pass_len = [0_u8; 1];
        inbound.read_exact(&mut pass_len).await?;
        let mut pass = vec![0_u8; pass_len[0] as usize];
        inbound.read_exact(&mut pass).await?;
        let ok = user == expected_user && pass == expected_pass;
        inbound
            .write_all(&[0x01, if ok { 0x00 } else { 0x01 }])
            .await?;
        if !ok {
            return Ok(());
        }
    }

    let mut header = [0_u8; 4];
    inbound.read_exact(&mut header).await?;
    if header[0] != 0x05 || header[1] != 0x01 {
        return Ok(());
    }
    let target = read_socks5_addr(&mut inbound, header[3]).await?;
    let mut outbound = TcpStream::connect(target).await?;
    inbound
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    Ok(())
}

async fn read_socks5_addr(stream: &mut TcpStream, atyp: u8) -> io::Result<SocketAddr> {
    let ip = match atyp {
        0x01 => {
            let mut octets = [0_u8; 4];
            stream.read_exact(&mut octets).await?;
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        0x03 => {
            let mut len = [0_u8; 1];
            stream.read_exact(&mut len).await?;
            let mut host = vec![0_u8; len[0] as usize];
            stream.read_exact(&mut host).await?;
            let host = String::from_utf8_lossy(&host);
            resolve_test_host(&host)?
        }
        0x04 => {
            let mut octets = [0_u8; 16];
            stream.read_exact(&mut octets).await?;
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => return Err(invalid_input(format!("unknown SOCKS5 ATYP: {atyp:#04x}"))),
    };
    let mut port = [0_u8; 2];
    stream.read_exact(&mut port).await?;
    Ok(SocketAddr::new(ip, u16::from_be_bytes(port)))
}

async fn start_vless_proxy(bind: IpAddr) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = handle_vless(stream).await;
            });
        }
    });
    Ok(addr)
}

async fn start_shadowsocks_proxy(bind: IpAddr) -> io::Result<SocketAddr> {
    use shadowsocks::config::{ServerConfig, ServerType};
    use shadowsocks::context::Context;
    use shadowsocks::crypto::CipherKind;
    use shadowsocks::relay::socks5::Address;
    use shadowsocks::ProxyListener;

    let config = ServerConfig::new(
        SocketAddr::new(bind, 0),
        SS_PASSWORD,
        CipherKind::AES_128_GCM,
    )
    .map_err(|err| invalid_input(format!("failed to build Shadowsocks config: {err}")))?;
    let context = Context::new_shared(ServerType::Server);
    let listener = ProxyListener::bind(context, &config).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let target = match inbound.handshake().await {
                    Ok(Address::SocketAddress(addr)) => addr,
                    Ok(Address::DomainNameAddress(host, port)) => {
                        match resolve_test_host(&host).map(|ip| SocketAddr::new(ip, port)) {
                            Ok(addr) => addr,
                            Err(_) => return,
                        }
                    }
                    Err(_) => return,
                };
                let Ok(mut outbound) = TcpStream::connect(target).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    Ok(addr)
}

async fn start_trojan_proxy(bind: IpAddr) -> io::Result<SocketAddr> {
    install_crypto_provider();
    let (cert, key) = generate_self_signed_cert()?;
    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|err| invalid_input(format!("failed to build TLS config: {err}")))?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let listener = TcpListener::bind(SocketAddr::new(bind, 0)).await?;
    let addr = listener.local_addr()?;
    let expected_hex = trojan_hex_password(TROJAN_PASSWORD);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let expected_hex = expected_hex.clone();
            tokio::spawn(async move {
                let _ = handle_trojan(stream, acceptor, expected_hex).await;
            });
        }
    });
    Ok(addr)
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn generate_self_signed_cert() -> io::Result<(
    rustls::pki_types::CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .map_err(|err| invalid_input(format!("failed to generate cert: {err}")))?;
    let cert = rustls::pki_types::CertificateDer::from(generated.cert.der().to_vec());
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        generated.key_pair.serialize_der(),
    ));
    Ok((cert, key))
}

fn trojan_hex_password(password: &str) -> String {
    use sha2::{Digest, Sha224};
    let mut hasher = Sha224::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

async fn handle_trojan(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    expected_hex: String,
) -> io::Result<()> {
    let mut inbound = acceptor
        .accept(stream)
        .await
        .map_err(|err| invalid_input(format!("trojan TLS accept failed: {err}")))?;

    let mut password = [0_u8; 56];
    inbound.read_exact(&mut password).await?;
    if String::from_utf8_lossy(&password) != expected_hex {
        return Ok(());
    }

    let mut crlf = [0_u8; 2];
    inbound.read_exact(&mut crlf).await?;
    if crlf != *b"\r\n" {
        return Ok(());
    }

    let mut cmd = [0_u8; 1];
    inbound.read_exact(&mut cmd).await?;
    if cmd[0] != 0x01 {
        return Ok(());
    }

    let target = read_trojan_addr(&mut inbound).await?;

    inbound.read_exact(&mut crlf).await?;
    if crlf != *b"\r\n" {
        return Ok(());
    }

    let mut outbound = TcpStream::connect(target).await?;
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    Ok(())
}

async fn read_trojan_addr<S>(stream: &mut S) -> io::Result<SocketAddr>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut atyp = [0_u8; 1];
    stream.read_exact(&mut atyp).await?;
    let ip = match atyp[0] {
        0x01 => {
            let mut octets = [0_u8; 4];
            stream.read_exact(&mut octets).await?;
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        0x03 => {
            let mut len = [0_u8; 1];
            stream.read_exact(&mut len).await?;
            let mut host = vec![0_u8; len[0] as usize];
            stream.read_exact(&mut host).await?;
            let host = String::from_utf8_lossy(&host);
            resolve_test_host(&host)?
        }
        0x04 => {
            let mut octets = [0_u8; 16];
            stream.read_exact(&mut octets).await?;
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        other => return Err(invalid_input(format!("unknown Trojan ATYP: {other:#04x}"))),
    };
    let mut port = [0_u8; 2];
    stream.read_exact(&mut port).await?;
    Ok(SocketAddr::new(ip, u16::from_be_bytes(port)))
}

async fn handle_vless(mut inbound: TcpStream) -> io::Result<()> {
    let (uuid, cmd, target) = read_vless_header(&mut inbound).await?;
    if uuid != VLESS_UUID || cmd != 0x01 {
        return Ok(());
    }
    inbound.write_all(&[0x00, 0x00]).await?;
    let mut outbound = TcpStream::connect(target).await?;
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    Ok(())
}

async fn read_vless_header(stream: &mut TcpStream) -> io::Result<([u8; 16], u8, SocketAddr)> {
    let mut version = [0_u8; 1];
    stream.read_exact(&mut version).await?;
    if version[0] != 0x00 {
        return Err(invalid_input("unexpected VLESS version"));
    }

    let mut uuid = [0_u8; 16];
    stream.read_exact(&mut uuid).await?;

    let mut addon_len = [0_u8; 1];
    stream.read_exact(&mut addon_len).await?;
    if addon_len[0] > 0 {
        let mut addon = vec![0_u8; addon_len[0] as usize];
        stream.read_exact(&mut addon).await?;
    }

    let mut cmd = [0_u8; 1];
    stream.read_exact(&mut cmd).await?;

    let mut port = [0_u8; 2];
    stream.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);

    let mut atyp = [0_u8; 1];
    stream.read_exact(&mut atyp).await?;
    let ip = match atyp[0] {
        0x01 => {
            let mut octets = [0_u8; 4];
            stream.read_exact(&mut octets).await?;
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        0x02 => {
            let mut len = [0_u8; 1];
            stream.read_exact(&mut len).await?;
            let mut host = vec![0_u8; len[0] as usize];
            stream.read_exact(&mut host).await?;
            let host = String::from_utf8_lossy(&host);
            resolve_test_host(&host)?
        }
        0x03 => {
            let mut octets = [0_u8; 16];
            stream.read_exact(&mut octets).await?;
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        other => return Err(invalid_input(format!("unknown VLESS ATYP: {other:#04x}"))),
    };

    Ok((uuid, cmd[0], SocketAddr::new(ip, port)))
}

fn resolve_test_host(host: &str) -> io::Result<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }
    match host {
        "localhost" | "127.0.0.1" => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        "::1" => Ok(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        other => Err(invalid_input(format!(
            "mock server only resolves IP literals and localhost, got: {other}"
        ))),
    }
}

fn parse_host_port(authority: &str, default_port: u16) -> io::Result<SocketAddr> {
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|err| invalid_input(format!("invalid port in {authority}: {err}")))?;
        (host, port)
    } else {
        (authority, default_port)
    };
    let ip = resolve_test_host(host)?;
    Ok(SocketAddr::new(ip, port))
}

struct HostPort {
    host: String,
    port: u16,
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') && !self.host.starts_with('[') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

fn write_profile(
    mode: Mode,
    echo_target: &HostPort,
    proxy_target: Option<&HostPort>,
    output: Option<&Path>,
) -> io::Result<PathBuf> {
    let content = render_profile(mode, echo_target, proxy_target)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = output.map(PathBuf::from).unwrap_or_else(|| {
        root.join("generated")
            .join(format!("{}.yaml", mode.as_str()))
    });
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, content)?;
    Ok(output_path)
}

fn render_profile(
    mode: Mode,
    echo_target: &HostPort,
    proxy_target: Option<&HostPort>,
) -> io::Result<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let template_path = root
        .join("profiles")
        .join(format!("{}.yaml.in", mode.as_str()));
    let template = std::fs::read_to_string(&template_path)?;
    let proxy_port = proxy_target.map(|addr| addr.port).unwrap_or(0);
    let content = template
        .replace("{{HOST}}", &echo_target.host)
        .replace("{{ECHO_PORT}}", &echo_target.port.to_string())
        .replace("{{PROXY_PORT}}", &proxy_port.to_string());
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(|error| {
        invalid_data(format!(
            "generated {} profile is invalid YAML: {error}",
            mode.as_str()
        ))
    })?;
    if !parsed.is_mapping() {
        return Err(invalid_data(format!(
            "generated {} profile root is not a YAML map",
            mode.as_str()
        )));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_host_rejects_accidentally_captured_ipv6_status_line() {
        let error = validate_advertise_host("192.168.3.131\naddress:").unwrap_err();
        assert!(error.to_string().contains("contains whitespace"));
    }

    #[test]
    fn advertise_host_accepts_ip_addresses_and_dns_names() {
        assert_eq!(
            validate_advertise_host(" 192.168.3.131 ").unwrap(),
            "192.168.3.131"
        );
        assert_eq!(
            validate_advertise_host("paws-test.local").unwrap(),
            "paws-test.local"
        );
        assert_eq!(validate_advertise_host("::1").unwrap(), "::1");
    }

    #[test]
    fn every_profile_template_renders_as_yaml() {
        let modes = [
            Mode::Direct,
            Mode::Http,
            Mode::HttpAuth,
            Mode::HttpBadAuth,
            Mode::HttpDown,
            Mode::Socks5,
            Mode::Socks5Auth,
            Mode::Socks5BadAuth,
            Mode::Shadowsocks,
            Mode::ShadowsocksBadPassword,
            Mode::Trojan,
            Mode::TrojanBadPassword,
            Mode::Vless,
            Mode::VlessBadUuid,
        ];
        for mode in modes {
            let echo = HostPort {
                host: "2001:db8::7".to_owned(),
                port: 18080,
            };
            let proxy = HostPort {
                host: echo.host.clone(),
                port: 18081,
            };
            render_profile(mode, &echo, Some(&proxy))
                .unwrap_or_else(|error| panic!("{} template failed: {error}", mode.as_str()));
        }
    }
}
