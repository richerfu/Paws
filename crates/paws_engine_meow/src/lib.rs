//! meow-rs translation at the packet-stack boundary.
mod memory;
mod observation;
#[cfg(test)]
mod tests;
use async_trait::async_trait;
use meow_common::sniffer::{sniff_http, sniff_tls};
use meow_common::{Metadata, ProxyConn, ProxyPacketConn};
use meow_listener::SnifferRuntime;
use meow_trie::DomainTrie;
use meow_tunnel::Tunnel;
use paws_engine::{
    EngineError, EngineTunnel, FlowMetadata, Network, PacketConnection, ProxyStream, SniffProtocol,
    SnifferConfig,
};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct MeowTunnel {
    tunnel: Tunnel,
    config: meow_common::sniffer::SnifferConfig,
    sniffer: SnifferRuntime,
    force_domains: DomainTrie<()>,
}
impl MeowTunnel {
    pub fn new(tunnel: Tunnel, config: meow_common::sniffer::SnifferConfig) -> Self {
        let mut force_domains = DomainTrie::new();
        for domain in &config.force_domain {
            force_domains.insert(domain, ());
        }
        Self {
            tunnel,
            sniffer: SnifferRuntime::new(config.clone()),
            config,
            force_domains,
        }
    }
}
fn metadata(value: &FlowMetadata) -> Metadata {
    Metadata {
        network: match value.network {
            Network::Tcp => meow_common::Network::Tcp,
            Network::Udp => meow_common::Network::Udp,
        },
        conn_type: meow_common::ConnType::Inner,
        src_ip: value.src_ip,
        src_port: value.src_port,
        dst_ip: value.dst_ip,
        dst_port: value.dst_port,
        host: value.host.as_str().into(),
        sniff_host: value.sniff_host.as_str().into(),
        in_name: value.in_name.as_str().into(),
        in_port: value.in_port,
        ..Metadata::default()
    }
}
struct Stream(Box<dyn ProxyStream>);
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buffer)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buffer)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
impl ProxyConn for Stream {}
fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> EngineError {
    EngineError::Backend {
        engine: paws_engine::EngineConfig::Meow.engine(),
        operation,
        message: error.to_string(),
    }
}
struct Packet(Box<dyn ProxyPacketConn>);
#[async_trait]
impl PacketConnection for Packet {
    async fn read_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), EngineError> {
        self.0
            .read_packet(buffer)
            .await
            .map_err(|error| backend_error("read UDP", error))
    }
    async fn write_packet(
        &self,
        buffer: &[u8],
        address: &SocketAddr,
    ) -> Result<usize, EngineError> {
        self.0
            .write_packet(buffer, address)
            .await
            .map_err(|error| backend_error("write UDP", error))
    }
    fn local_addr(&self) -> Result<SocketAddr, EngineError> {
        self.0
            .local_addr()
            .map_err(|error| backend_error("UDP address", error))
    }
    fn close(&self) -> Result<(), EngineError> {
        self.0
            .close()
            .map_err(|error| backend_error("close UDP", error))
    }
}
#[async_trait]
impl EngineTunnel for MeowTunnel {
    async fn handle_tcp(&self, stream: Box<dyn ProxyStream>, flow: FlowMetadata) {
        meow_tunnel::tcp::handle_tcp(
            &self.tunnel.inner().clone(),
            Box::new(Stream(stream)),
            metadata(&flow),
        )
        .await;
    }
    async fn open_udp(
        &self,
        flow: &mut FlowMetadata,
    ) -> Result<Arc<dyn PacketConnection>, EngineError> {
        let inner = self.tunnel.inner().clone();
        let mut meta = metadata(flow);
        inner.pre_resolve(&mut meta).await;
        flow.dst_ip = meta.dst_ip;
        let target = inner
            .resolve_proxy(&meta)
            .await
            .ok_or_else(|| backend_error("route UDP", "no route target"))?;
        let packet = target
            .adapter
            .dial_udp(&meta)
            .await
            .map_err(|error| backend_error("dial UDP", error))?;
        Ok(Arc::new(Packet(packet)))
    }
    async fn dns_query(&self, query: &[u8]) -> Result<Vec<u8>, EngineError> {
        meow_dns::DnsServer::handle_query(query, &self.tunnel.resolver())
            .await
            .map_err(|error| backend_error("DNS query", error))
    }
    fn sniffer_config(&self) -> SnifferConfig {
        SnifferConfig {
            enable: self.config.enable,
            override_destination: self.config.override_destination,
            parse_pure_ip: self.config.parse_pure_ip,
            tls_ports: self.config.tls_ports.clone(),
            http_ports: self.config.http_ports.clone(),
            force_domain: self
                .config
                .force_domain
                .iter()
                .map(ToString::to_string)
                .collect(),
            skip_domain: self
                .config
                .skip_domain
                .iter()
                .map(ToString::to_string)
                .collect(),
            timeout: self.config.timeout,
        }
    }
    fn sniff(&self, protocol: SniffProtocol, payload: &[u8]) -> Option<String> {
        match protocol {
            SniffProtocol::Tls => sniff_tls(payload),
            SniffProtocol::Http => sniff_http(payload),
        }
        .map(|host| host.to_string())
    }
    fn should_sniff(&self, flow: &FlowMetadata) -> bool {
        !self.config.parse_pure_ip
            || flow.host.is_empty()
            || flow.host.parse::<IpAddr>().is_ok()
            || self.force_domains.search(&flow.host).is_some()
    }
    fn apply_sniff(&self, host: &str, flow: &mut FlowMetadata) {
        let mut meta = metadata(flow);
        self.sniffer.maybe_apply_sniff(host, &mut meta);
        flow.host = meta.host.to_string();
        flow.sniff_host = meta.sniff_host.to_string();
    }
}

mod session;
pub use session::{MeowEngineSession, MeowRuntimeConfig};

mod app_http;
pub use app_http::AppHttpProxyRuntime;
mod services;

mod management;
pub use management::{connection_snapshot, proxy_groups_snapshot};
mod controller;
