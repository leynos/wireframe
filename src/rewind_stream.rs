use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A stream adapter that replays buffered bytes before reading
/// from the underlying stream.
pub struct RewindStream<S> {
    leftover: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S> RewindStream<S> {
    /// Create a new `RewindStream` that will yield `leftover` before
    /// delegating to `inner`.
    pub fn new(leftover: Vec<u8>, inner: S) -> Self {
        Self {
            leftover,
            pos: 0,
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for RewindStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos < self.leftover.len() {
            let remaining = self.leftover.len() - self.pos;
            let to_copy = remaining.min(buf.remaining());
            let start = self.pos;
            let end = start + to_copy;
            buf.put_slice(&self.leftover[start..end]);
            self.pos += to_copy;
            if self.pos < self.leftover.len() || to_copy > 0 {
                return Poll::Ready(Ok(()));
            }
        }

        if self.pos >= self.leftover.len() && !self.leftover.is_empty() {
            self.leftover.clear();
        }

        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for RewindStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S: Unpin> Unpin for RewindStream<S> {}
