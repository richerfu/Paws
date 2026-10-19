mod body;
mod conn;
pub mod header;
mod kdf;

use async_trait::async_trait;
use meow_common::{
    AdapterType, MeowError, Metadata, ProxyAdapter, ProxyConn, ProxyHealth, ProxyPacketConn, Result,
};
use smol_str::SmolStr;
use std::sync::Arc;
use tracing::debug;

use crate::transport_chain::TransportChain;
pub use header::Security;

pub struct VmessAdapter {
    name: SmolStr,
    server: SmolStr,
    port: u16,
    addr_str: SmolStr,
    cmd_key: [u8; 16],
    security: Security,
    udp: bool,
    transport: Arc<TransportChain>,
    health: ProxyHealth,
    /// sing-mux compatible connection multiplexing (optional).
    #[cfg(feature = "mux")]
    mux: Option<Arc<crate::mux::MuxClient>>,
}

impl VmessAdapter {
    pub fn new(
        name: &str,
        server: &str,
        port: u16,
        uuid_bytes: [u8; 16],
        security: Security,
        udp: bool,
        transport: TransportChain,
    ) -> Self {
        Self {
            name: SmolStr::from(name),
            server: SmolStr::from(server),
            port,
            addr_str: SmolStr::from(format!("{server}:{port}")),
            cmd_key: header::cmd_key(&uuid_bytes),
            security,
            udp,
            transport: Arc::new(transport),
            health: ProxyHealth::new(),
            #[cfg(feature = "mux")]
            mux: None,
        }
    }

    /// Enable connection multiplexing.  Two wire protocols share one
    /// connection pool (protocol picked by the `protocol` mux option):
    ///
    /// * sing-mux (smux/yamux/h2mux) — the session's VMess request targets
    ///   the reserved mux destination (sp.mux.sing-box.arpa:444) and a mux
    ///   request header follows; server must be sing-box / mihomo with
    ///   multiplex enabled on the VMess inbound.
    /// * muxcool — the session's VMess request itself is the signaling
    ///   (CommandMux 0x03, no address); server must be Xray, or sing-box /
    ///   mihomo (sing-vmess routes CommandMux to HandleMuxConnection, no
    ///   inbound config needed).
    #[cfg(feature = "mux")]
    pub fn with_mux(mut self, options: crate::mux::MuxOptions) -> Self {
        use crate::mux::{MuxClient, MUX_DESTINATION_FQDN, MUX_DESTINATION_PORT};
        use std::sync::Arc as StdArc;

        let transport = Arc::clone(&self.transport);
        let server = self.server.clone();
        let cmd_key = self.cmd_key;
        let security = self.security;
        let port = self.port;
        let protocol = options.protocol;

        let dial: crate::mux::DialFn = StdArc::new(move || {
            let transport = Arc::clone(&transport);
            let server = server.clone();
            Box::pin(async move {
                let sealed = match protocol {
                    crate::mux::Protocol::MuxCool => {
                        header::seal_mux_request_header(&cmd_key, security)
                    }
                    _ => {
                        let metadata = Metadata {
                            host: MUX_DESTINATION_FQDN.into(),
                            dst_port: MUX_DESTINATION_PORT,
                            ..Default::default()
                        };
                        header::seal_request_header(&cmd_key, security, &metadata, false)
                    }
                }
                .map_err(MeowError::Proxy)?;
                dial_vmess(&transport, &server, port, sealed, security).await
            })
        });
        self.mux = Some(MuxClient::new(dial, options));
        self
    }

    /// Dial a raw TCP + transport-chain stream to the VMess server, run the
    /// VMess request header exchange for the given destination, and return
    /// the encrypted duplex.
    async fn dial_to(&self, metadata: &Metadata) -> Result<Box<dyn ProxyConn>> {
        let sealed = header::seal_request_header(&self.cmd_key, self.security, metadata, false)
            .map_err(MeowError::Proxy)?;
        dial_vmess(
            &self.transport,
            &self.server,
            self.port,
            sealed,
            self.security,
        )
        .await
    }
}

/// Dial a raw TCP + transport-chain stream to the VMess server, run the
/// VMess request header exchange for the given destination, and return the
/// encrypted duplex.  Shared by the plain dial path and the sing-mux
/// session dialer (which targets the reserved mux destination).
async fn dial_vmess(
    transport: &TransportChain,
    server: &str,
    port: u16,
    sealed: header::SealedHeader,
    security: Security,
) -> Result<Box<dyn ProxyConn>> {
    use tokio::io::AsyncWriteExt;

    let tcp = meow_common::connect_tcp_host(server, port)
        .await
        .map_err(MeowError::Io)?;
    let _ = tcp.set_nodelay(true);
    let mut stream = transport
        .connect(Box::new(tcp))
        .await
        .map_err(|e| MeowError::Proxy(format!("vmess transport: {e}")))?;

    stream
        .write_all(&sealed.bytes)
        .await
        .map_err(MeowError::Io)?;

    let read_cipher =
        body::BodyCipher::new(security, &sealed.req_key, &sealed.req_iv, sealed.resp_v);
    let write_cipher =
        body::BodyCipher::new(security, &sealed.req_key, &sealed.req_iv, sealed.resp_v);

    let duplex = conn::spawn_vmess_relay(
        stream,
        read_cipher,
        write_cipher,
        sealed.req_key,
        sealed.req_iv,
        sealed.resp_v,
    );
    Ok(Box::new(crate::stream_conn::StreamConn(Box::new(duplex))))
}

#[async_trait]
impl ProxyAdapter for VmessAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn adapter_type(&self) -> AdapterType {
        AdapterType::Vmess
    }

    fn addr(&self) -> &str {
        &self.addr_str
    }

    fn support_udp(&self) -> bool {
        // With mux enabled, UDP rides the mux TCP session (unless
        // `only-tcp` forces the plain path) — mirrors mihomo's
        // SingMux.SupportUDP.
        self.udp || {
            #[cfg(feature = "mux")]
            {
                self.mux.as_ref().is_some_and(|mux| mux.supports_udp())
            }
            #[cfg(not(feature = "mux"))]
            {
                false
            }
        }
    }

    async fn dial_tcp(&self, metadata: &Metadata) -> Result<Box<dyn ProxyConn>> {
        debug!(
            "VMess connecting to {} via {}",
            metadata.remote_address(),
            self.addr_str
        );

        #[cfg(feature = "mux")]
        if let Some(mux) = &self.mux {
            let conn = mux.open_stream_for(metadata, "vmess").await?;
            return Ok(Box::new(conn));
        }

        self.dial_to(metadata).await
    }

    #[cfg_attr(not(feature = "mux"), allow(unused_variables))]
    async fn dial_udp(&self, metadata: &Metadata) -> Result<Box<dyn ProxyPacketConn>> {
        #[cfg(feature = "mux")]
        if let Some(mux) = &self.mux {
            if mux.supports_udp() {
                debug!(
                    "VMess mux UDP connecting to {} via {}",
                    metadata.remote_address(),
                    self.addr_str
                );
            }
            if let Some(conn) = mux.open_packet_stream_for(metadata, "vmess").await? {
                return Ok(conn);
            }
        }

        Err(MeowError::NotSupported(
            "vmess UDP relay not yet implemented".into(),
        ))
    }

    fn health(&self) -> &ProxyHealth {
        &self.health
    }
}
