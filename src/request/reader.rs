//! `AsyncRead` adapter over a streaming request body.
//!
//! [`RequestBodyReader`] wraps a [`RequestBodyStream`] so a protocol crate can
//! feed streaming bytes to parsers that consume readers rather than streams.
//! [`RequestBodyReader::new`] takes ownership of the body and
//! [`RequestBodyReader::into_inner`] gives it back, discarding any bytes the
//! reader had buffered but not handed out. The [`AsyncRead`] implementation
//! forwards polls to the underlying stream, so chunk order and backpressure
//! carry through.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;

use super::RequestBodyStream;

/// Adapter wrapping a [`RequestBodyStream`] as [`AsyncRead`].
///
/// Protocol crates can use this to feed streaming bytes into parsers
/// that operate on readers rather than streams. The adapter consumes
/// chunks from the underlying stream and presents them as a contiguous
/// byte sequence.
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use tokio::io::AsyncReadExt;
/// use wireframe::request::{RequestBodyReader, RequestBodyStream};
///
/// async fn read_all(body: RequestBodyStream) -> std::io::Result<Vec<u8>> {
///     let mut reader = RequestBodyReader::new(body);
///     let mut buf = Vec::new();
///     reader.read_to_end(&mut buf).await?;
///     Ok(buf)
/// }
/// ```
pub struct RequestBodyReader {
    /// Owns ordered chunks so `AsyncRead` preserves backpressure and cancellation semantics.
    inner: StreamReader<RequestBodyStream, Bytes>,
}
impl RequestBodyReader {
    /// Create a new reader from a streaming body.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes::Bytes;
    /// use wireframe::request::{RequestBodyReader, RequestBodyStream};
    ///
    /// fn wrap_stream(stream: RequestBodyStream) -> RequestBodyReader {
    ///     RequestBodyReader::new(stream)
    /// }
    /// ```
    #[must_use]
    pub fn new(stream: RequestBodyStream) -> Self {
        Self {
            inner: StreamReader::new(stream),
        }
    }
    /// Consume the reader and return the underlying stream.
    ///
    /// # Data loss warning
    ///
    /// Any bytes that have been read from the stream but not yet consumed
    /// from the reader's internal buffer are **permanently discarded** when
    /// this method is called. This can occur when:
    ///
    /// - A partial read left buffered bytes (for example, `read(&mut [u8; 4])` when the chunk
    ///   contained more than 4 bytes)
    /// - The reader was used with `BufRead` methods that prefetch data
    ///
    /// **Safe to call when:** the reader has been fully drained (all reads
    /// returned zero bytes or an error), or when deliberately abandoning
    /// any remaining buffered content.
    pub fn into_inner(self) -> RequestBodyStream { self.inner.into_inner() }
}
impl AsyncRead for RequestBodyReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
