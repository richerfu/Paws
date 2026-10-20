//! TLS-record I/O machinery shared by the record-framed SIP003
//! transports (`shadow_tls`, `restls`, `jls`) — incremental record assembly,
//! outbox draining, and VecDeque→ReadBuf serving. One copy keeps the
//! framing rules (header-then-payload reads, reads capped at the record
//! boundary, EOF-at-boundary vs mid-record, write-zero) identical across
//! transports.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// TLS record header size.
pub(crate) const RECORD_HDR: usize = 5;

/// Incremental TLS-record assembler.
pub(crate) struct RecordAssembler {
    /// Bytes accumulated so far for the record under construction.
    pub(crate) rec: Vec<u8>,
    /// Byte count that completes the record: `RECORD_HDR` until the
    /// 5-byte header lands, then `RECORD_HDR + declared_len`. 0 = ready
    /// for a new record.
    want: usize,
}

impl RecordAssembler {
    pub(crate) fn new() -> Self {
        Self {
            rec: Vec::new(),
            want: 0,
        }
    }

    /// Read one complete record into `rec`.
    ///
    /// `inbox` bytes (handshake read-ahead) are consumed before touching
    /// `inner`.  `header_gate` runs once per record as soon as the header
    /// completes — *before* the payload is read — so a caller can reject
    /// a bad record type early, exactly like upstream.
    ///
    /// Returns `Some(())` with the record buffered in `rec`, `None` on
    /// clean EOF at a record boundary (only when `boundary_eof_ok`),
    /// otherwise `UnexpectedEof`.  `label` prefixes error text.
    pub(crate) fn poll_fill<R: AsyncRead + Unpin + ?Sized>(
        &mut self,
        inner: &mut R,
        cx: &mut Context<'_>,
        mut inbox: Option<&mut VecDeque<u8>>,
        boundary_eof_ok: bool,
        header_gate: &mut dyn FnMut(&[u8; RECORD_HDR]) -> io::Result<()>,
        label: &'static str,
    ) -> Poll<io::Result<Option<()>>> {
        loop {
            if self.want == 0 {
                self.want = RECORD_HDR;
                self.rec.clear();
            }
            let mut tmp = [0u8; 8192];
            while self.rec.len() < self.want {
                if let Some(inbox) = inbox.as_deref_mut() {
                    if !inbox.is_empty() {
                        let take = (self.want - self.rec.len()).min(inbox.len());
                        let (a, b) = inbox.as_slices();
                        let from_a = take.min(a.len());
                        self.rec.extend_from_slice(&a[..from_a]);
                        if from_a < take {
                            self.rec.extend_from_slice(&b[..take - from_a]);
                        }
                        inbox.drain(..take);
                        continue;
                    }
                }
                // Cap the read at the record boundary — a bigger slice
                // would glue head bytes of the *next* record onto this
                // one and corrupt both parsing and the record MAC.
                let need = (self.want - self.rec.len()).min(8192);
                let mut rb = ReadBuf::new(&mut tmp[..need]);
                match Pin::new(&mut *inner).poll_read(cx, &mut rb) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Ready(Ok(())) => {
                        let filled = rb.filled();
                        if filled.is_empty() {
                            if boundary_eof_ok && self.rec.is_empty() && self.want == RECORD_HDR {
                                return Poll::Ready(Ok(None));
                            }
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                format!("{label}: EOF mid-record"),
                            )));
                        }
                        self.rec.extend_from_slice(filled);
                    }
                }
            }
            if self.want == RECORD_HDR {
                let mut hdr = [0u8; RECORD_HDR];
                hdr.copy_from_slice(&self.rec);
                header_gate(&hdr)?;
                let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
                self.want = RECORD_HDR + len;
                if self.rec.len() < self.want {
                    continue;
                }
                // len == 0 — the header was gated above; the record is
                // already complete.
            }
            self.want = 0;
            return Poll::Ready(Ok(Some(())));
        }
    }

    /// Serve the assembled record's payload (from `*rec_serve`) into
    /// `buf`; clears `rec` once fully consumed.  Returns false when no
    /// payload remains — the caller should assemble the next record.
    #[cfg(feature = "shadow-tls")]
    pub(crate) fn serve(&mut self, rec_serve: &mut usize, buf: &mut ReadBuf<'_>) -> bool {
        if *rec_serve == 0 {
            return false;
        }
        if *rec_serve >= self.rec.len() {
            // A zero-payload record left the cursor armed with nothing
            // to serve — disarm it, or bytes of the *next* record would
            // be served as payload as it assembles.
            *rec_serve = 0;
            return false;
        }
        let n = (self.rec.len() - *rec_serve).min(buf.remaining());
        buf.put_slice(&self.rec[*rec_serve..*rec_serve + n]);
        *rec_serve += n;
        if *rec_serve == self.rec.len() {
            self.rec.clear();
            *rec_serve = 0;
        }
        true
    }
}

/// Drain `outbox` into `inner` — partial writes, `WriteZero` and
/// `Pending` handled identically for every transport.  `label`
/// prefixes error text.
pub(crate) fn poll_drain_outbox<W: AsyncWrite + Unpin + ?Sized>(
    inner: &mut W,
    outbox: &mut VecDeque<u8>,
    cx: &mut Context<'_>,
    label: &'static str,
) -> Poll<io::Result<()>> {
    while !outbox.is_empty() {
        let slice = outbox.make_contiguous();
        match Pin::new(&mut *inner).poll_write(cx, slice) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(0)) => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("{label}: write zero"),
                )));
            }
            Poll::Ready(Ok(n)) => outbox.drain(..n),
        };
    }
    Poll::Ready(Ok(()))
}

/// Move up to `buf.remaining()` bytes off `pending` into the read buffer.
pub(crate) fn serve_pending(pending: &mut VecDeque<u8>, buf: &mut ReadBuf<'_>) {
    let n = pending.len().min(buf.remaining());
    let (a, b) = pending.as_slices();
    let from_a = n.min(a.len());
    buf.put_slice(&a[..from_a]);
    if from_a < n {
        buf.put_slice(&b[..n - from_a]);
    }
    pending.drain(..n);
}
