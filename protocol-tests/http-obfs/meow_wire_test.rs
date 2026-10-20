//! Standalone local fixture. The temporary Cargo manifest supplies dependencies;
//! this path includes the actual production HTTP envelope without modifying it.
use meow_transport::simple_obfs::client as simple_obfs;

use shadowsocks::{
    config::{ServerConfig, ServerType},
    context::Context,
    crypto::CipherKind,
    relay::socks5::Address,
    ProxyClientStream,
};
use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

struct Wire {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    max_write: usize,
    yield_next: bool,
}
impl AsyncRead for Wire {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Wire {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let s = self.get_mut();
        if s.yield_next {
            s.yield_next = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let n = bytes.len().min(s.max_write);
        if n > 0 {
            s.writes.lock().unwrap().push(bytes[..n].to_vec());
        }
        s.yield_next = s.max_write != usize::MAX;
        Poll::Ready(Ok(n))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("local ClientHello fixture path");
    let hello = std::fs::read(path)?;
    assert!(hello.len() > 5 && hello[0] == 22);
    assert_eq!(
        u16::from_be_bytes([hello[3], hello[4]]) as usize + 5,
        hello.len()
    );
    let mut reports = Vec::new();
    for chunk_size in [hello.len(), 7] {
        for max_write in [usize::MAX, 7] {
            let writes = Arc::new(Mutex::new(Vec::new()));
            let raw = Wire {
                writes: writes.clone(),
                max_write,
                yield_next: false,
            };
            let transport = simple_obfs::HttpObfs::new(raw, "cover.example".into(), 443)
                .map_err(io::Error::other)?;
            let config = ServerConfig::new(
                "127.0.0.1:8388".parse::<std::net::SocketAddr>().unwrap(),
                "local-test-only",
                CipherKind::AES_128_GCM,
            )
            .unwrap();
            let mut conn = ProxyClientStream::from_stream(
                Context::new_shared(ServerType::Local),
                transport,
                &config,
                Address::DomainNameAddress("target.example".into(), 443),
            );
            // Match SsCore::dial_tcp_stream: empty first write emits destination.
            assert_eq!(conn.write(&[]).await?, 0);
            conn.flush().await?;
            let eager_writes = writes.lock().unwrap().len();
            for chunk in hello.chunks(chunk_size) {
                conn.write_all(chunk).await?;
            }
            conn.flush().await?;
            let raw_writes: Vec<String> = writes.lock().unwrap().iter().map(|w| hex(w)).collect();
            reports.push(serde_json::json!({
                "name": format!("meow/chunk-{chunk_size}/{}", if max_write == 7 { "partial7-pending" } else { "full" }),
                "helloHex": hex(&hello), "writesHex": raw_writes,
                "eagerWrites": eager_writes, "chunkSize": chunk_size,
            }));
        }
    }
    println!("{}", serde_json::to_string(&reports).unwrap());
    Ok(())
}
