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
    /// Bytes already read with the preamble that must precede socket reads.
    leftover: Vec<u8>,
    /// Cursor into `leftover`; reset when the buffered prefix is exhausted.
    pos: usize,
    /// Underlying stream used after buffered bytes have been replayed.
    inner: S,
}

impl<S> RewindStream<S> {
    /// Create a new `RewindStream` that will yield `leftover` before
    /// delegating to `inner`.
    pub const fn new(leftover: Vec<u8>, inner: S) -> Self {
        Self {
            leftover,
            pos: 0,
            inner,
        }
    }

    /// Return a reference to the underlying stream.
    pub const fn inner(&self) -> &S { &self.inner }

    /// Return a mutable reference to the underlying stream.
    pub const fn inner_mut(&mut self) -> &mut S { &mut self.inner }

    /// Consume the wrapper and return the underlying stream.
    ///
    /// Note: any unread leftover bytes will be discarded.
    pub fn into_inner(self) -> S { self.inner }
}

#[cfg(test)]
impl<S> RewindStream<S> {
    pub(crate) fn set_pos_for_tests(&mut self, pos: usize) { self.pos = pos; }

    pub(crate) fn pos_for_tests(&self) -> usize { self.pos }

    pub(crate) fn leftover_len_for_tests(&self) -> usize { self.leftover.len() }
}

impl<S: AsyncRead + Unpin> AsyncRead for RewindStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos != self.leftover.len() {
            let remaining = self.leftover.len().saturating_sub(self.pos);
            let to_copy = remaining.min(buf.remaining());
            let start = self.pos;
            let end = start + to_copy;
            if let Some(slice) = self.leftover.get(start..end) {
                buf.put_slice(slice);
            } else {
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
    //! Coverage for rewind-buffer bounds and poll sequencing.
    //!
    //! Two `poll_read` mutants are equivalent and deliberately left
    //! unannotated. Mutating the replay-completion guard's `<` to `<=` cannot
    //! change behaviour because `pos == len` with `to_copy == 0` is
    //! unreachable after a successful copy; the second disjunct already
    //! covers the completing copy. Mutating the cleanup guard's `&&` to `||`
    //! only clears an already empty vector, because the cleanup guard is
    //! reached only with `pos == len`.

    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::task::noop_waker_ref;
    use tokio::io::{AsyncRead, ReadBuf};

    use super::*;

    /// Inner stream that records how often `poll_read` reaches it.
    ///
    /// The reader is always ready: it yields `data` until exhausted and then
    /// reports end-of-file. Counting polls lets a test prove that
    /// `RewindStream` did not fall through to the inner stream while replay
    /// was still pending.
    struct CountingReader {
        data: &'static [u8],
        polls: usize,
    }

    impl CountingReader {
        const fn new(data: &'static [u8]) -> Self { Self { data, polls: 0 } }
    }

    impl AsyncRead for CountingReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.polls += 1;
            Pin::new(&mut self.data).poll_read(cx, buf)
        }
    }

    /// Poll `stream` once with a buffer of `capacity` bytes.
    ///
    /// Returns the bytes delivered by that single poll. Panics when the poll
    /// is not immediately ready with `Ok`, because these tests model an
    /// always-ready inner reader driven by a no-op waker.
    fn poll_once(stream: &mut RewindStream<CountingReader>, capacity: usize) -> Vec<u8> {
        let waker = noop_waker_ref();
        let mut cx = Context::from_waker(waker);
        let mut buffer = vec![0_u8; capacity];
        let mut read_buf = ReadBuf::new(&mut buffer);

        match RewindStream::poll_read(Pin::new(stream), &mut cx, &mut read_buf) {
            Poll::Ready(Ok(())) => read_buf.filled().to_vec(),
            other => panic!("expected a ready Ok poll, got {other:?}"),
        }
    }

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
            Poll::Ready(Err(err)) => assert_eq!(
                err.kind(),
                io::ErrorKind::UnexpectedEof,
                "err.kind() should equal io::ErrorKind::UnexpectedEof"
            ),
            other => panic!("expected UnexpectedEof, got {other:?}"),
        }
    }

    #[test]
    fn poll_read_replays_leftover_across_split_reads() {
        let mut stream = RewindStream::new(b"abcde".to_vec(), CountingReader::new(b"XYZ"));

        // Successive small buffers split the leftover into `ab`, `c` and `de`
        // without reaching the inner stream. Kills `||` -> `&&` on the
        // replay-completion guard, and `>=` -> `<` and `!` deletion on the
        // cleanup guard, each of which would either append inner bytes to the
        // completing read or retain the cleared leftover.
        assert_eq!(poll_once(&mut stream, 2).as_slice(), b"ab");
        assert_eq!(poll_once(&mut stream, 1).as_slice(), b"c");
        assert_eq!(poll_once(&mut stream, 2).as_slice(), b"de");
        assert_eq!(stream.inner().polls, 0);
        assert_eq!(stream.leftover_len_for_tests(), 5);
        assert_eq!(stream.pos_for_tests(), 5);

        // Replay is complete, so this poll clears it before reading the inner
        // stream. A retained leftover or stale cursor fails here.
        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"XYZ");
        assert_eq!(stream.inner().polls, 1);
        assert_eq!(stream.leftover_len_for_tests(), 0);
        assert_eq!(stream.pos_for_tests(), 0);

        // The inner reader is now exhausted; a further poll reports EOF.
        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"");
        assert_eq!(stream.inner().polls, 2);
    }

    #[test]
    fn poll_read_reports_completion_without_appending_inner_bytes() {
        let mut stream = RewindStream::new(b"abc".to_vec(), CountingReader::new(b"XYZ"));

        // A buffer larger than the leftover must deliver exactly the leftover.
        // Kills `>` -> `==` and `||` -> `&&` on the replay-completion guard:
        // both fall through when the copy exhausts the leftover, clearing it
        // and polling the inner stream within the same read.
        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"abc");
        assert_eq!(stream.inner().polls, 0);
        assert_eq!(stream.leftover_len_for_tests(), 3);
        assert_eq!(stream.pos_for_tests(), 3);

        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"XYZ");
        assert_eq!(stream.inner().polls, 1);
        assert_eq!(stream.leftover_len_for_tests(), 0);
    }

    #[test]
    fn poll_read_makes_no_progress_while_zero_capacity_replay_is_pending() {
        let mut stream = RewindStream::new(b"abc".to_vec(), CountingReader::new(b"XYZ"));

        // A zero-capacity poll cannot copy, so it must report readiness
        // without touching the inner stream or the cursor. Kills `<` -> `==`
        // and `<` -> `>` on the replay-completion guard, which would both fall
        // through to the inner stream.
        assert_eq!(poll_once(&mut stream, 0).as_slice(), b"");
        assert_eq!(stream.inner().polls, 0);
        assert_eq!(stream.pos_for_tests(), 0);
        assert_eq!(stream.leftover_len_for_tests(), 3);

        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"abc");
    }

    #[test]
    fn poll_read_clears_replayed_leftover_before_polling_inner() {
        let mut stream = RewindStream::new(b"abc".to_vec(), CountingReader::new(b"XYZ"));

        assert_eq!(poll_once(&mut stream, 3).as_slice(), b"abc");
        assert_eq!(stream.inner().polls, 0);
        assert_eq!(stream.leftover_len_for_tests(), 3);
        assert_eq!(stream.pos_for_tests(), 3);

        // Replay is complete, so the next poll clears it before delegating.
        // With no buffer space the inner read yields nothing, but it must be
        // reached exactly once. Kills `>=` -> `<` and `!` deletion on the
        // cleanup guard, which would retain the leftover and cursor.
        assert_eq!(poll_once(&mut stream, 0).as_slice(), b"");
        assert_eq!(stream.inner().polls, 1);
        assert_eq!(stream.leftover_len_for_tests(), 0);
        assert_eq!(stream.pos_for_tests(), 0);

        assert_eq!(poll_once(&mut stream, 8).as_slice(), b"XYZ");
        assert_eq!(stream.inner().polls, 2);
    }
}
