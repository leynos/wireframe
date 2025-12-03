//! Async stream adapter that replays leftover bytes.
//!
//! `RewindStream` emits any bytes buffered from a preamble read before
//! delegating reads and writes to the underlying stream.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

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

#[cfg(test)]
impl<S> RewindStream<S> {
    pub(crate) fn set_pos_for_tests(&mut self, pos: usize) { self.pos = pos; }

    pub(crate) fn leftover_len_for_tests(&self) -> usize { self.leftover.len() }
}

impl<S: AsyncRead + Unpin> AsyncRead for RewindStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos > self.leftover.len() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "rewind buffer slice out of bounds",
            )));
        }
        if self.pos < self.leftover.len() {
            let remaining = self.leftover.len() - self.pos;
            let to_copy = remaining.min(buf.remaining());
            let start = self.pos;
            let end = start + to_copy;
            if let Some(slice) = self.leftover.get(start..end) {
                buf.put_slice(slice);
            } else {
                debug_assert!(false, "rewind slice bounds exceeded");
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "rewind buffer slice out of bounds",
                )));
            }
            self.pos += to_copy;
            if self.pos < self.leftover.len() || to_copy > 0 {
                return Poll::Ready(Ok(()));
            }
        }

        if self.pos >= self.leftover.len() && !self.leftover.is_empty() {
            self.leftover.clear();
            self.pos = 0;
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

#[cfg(test)]
mod tests {
    use std::{pin::Pin, task::Context};

    use futures::task::noop_waker_ref;
    use tokio::io::{AsyncRead, ReadBuf};

    use super::*;

    #[test]
    fn poll_read_returns_error_for_invalid_leftover_slice_bounds() {
        let mut stream = RewindStream::new(vec![1_u8, 2, 3], tokio::io::empty());
        stream.set_pos_for_tests(stream.leftover_len_for_tests() + 1);

        let waker = noop_waker_ref();
        let mut cx = Context::from_waker(waker);
        let mut buffer = [0_u8; 2];
        let mut read_buf = ReadBuf::new(&mut buffer);

        let mut pinned = Pin::new(&mut stream);
        let result = RewindStream::poll_read(Pin::as_mut(&mut pinned), &mut cx, &mut read_buf);

        match result {
            Poll::Ready(Err(err)) => assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("expected UnexpectedEof, got {other:?}"),
        }
    }
}
