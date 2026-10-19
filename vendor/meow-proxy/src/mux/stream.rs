//! `MuxStreamConn`: ProxyConn wrapper around one smux stream.

use super::client::{MuxSession, MuxStream};
use meow_common::ProxyConn;
use std::io;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// One multiplexed logical connection.  Dropping it sends FIN (best effort)
/// and decrements the session's stream count.
pub struct MuxStreamConn {
    inner: MuxStream,
    session: Arc<MuxSession>,
}

impl MuxStreamConn {
    pub(crate) fn new(inner: MuxStream, session: Arc<MuxSession>) -> Self {
        Self { inner, session }
    }
}

impl Drop for MuxStreamConn {
    fn drop(&mut self) {
        self.session.streams.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ProxyConn for MuxStreamConn {}

impl AsyncRead for MuxStreamConn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for MuxStreamConn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
