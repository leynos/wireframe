//! Request metadata and streaming body types for handler consumption.
//!
//! These types allow handlers to consume request payloads incrementally
//! rather than waiting for full reassembly. See ADR 0002 for the design
//! rationale and composition with transport-level fragmentation.
//!
//! # Streaming request bodies
//!
//! Handlers can opt into streaming by accepting a [`RequestBodyStream`]
//! alongside [`RequestParts`]. The framework provides chunks via a bounded
//! channel, propagating back-pressure when the handler consumes slowly.
//!
//! ```
//! use bytes::Bytes;
//! use wireframe::request::{RequestBodyStream, RequestParts};
//!
//! async fn handle_upload(parts: RequestParts, body: RequestBodyStream) {
//!     // Process metadata immediately
//!     let id = parts.id();
//!     // Consume body chunks incrementally via StreamExt or AsyncRead adapter
//! }
//! ```

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use tokio::{io::AsyncRead, sync::mpsc};
use tokio_util::io::StreamReader;

/// Streaming request body.
///
/// Handlers can consume this stream incrementally rather than waiting for
/// full body reassembly. Each item yields either a chunk of bytes or an
/// I/O error.
///
/// This type is symmetric with [`crate::FrameStream`] for streaming responses.
/// See ADR 0002 for design rationale.
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use futures::StreamExt;
/// use wireframe::request::RequestBodyStream;
///
/// async fn consume_stream(mut body: RequestBodyStream) {
///     while let Some(result) = body.next().await {
///         match result {
///             Ok(chunk) => { /* process chunk */ }
///             Err(e) => { /* handle I/O error */ }
///         }
///     }
/// }
/// ```
pub type RequestBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send + 'static>>;

/// Default capacity for streaming request body channels.
///
/// This value balances memory usage against throughput; handlers consuming
/// slowly will cause back-pressure after this many chunks buffer.
///
/// # Rationale
///
/// The value 16 was chosen based on common workload characteristics:
///
/// - **Memory efficiency**: With typical chunk sizes of 4–16 KiB, 16 buffered chunks consume at
///   most ~256 KiB per connection, which is acceptable for most server deployments.
///
/// - **Throughput smoothing**: A small buffer absorbs transient processing delays in handlers,
///   preventing socket stalls on bursty workloads.
///
/// - **Back-pressure responsiveness**: The buffer is small enough that back-pressure engages
///   promptly when handlers fall behind, protecting the server from memory exhaustion.
///
/// - **Power-of-two alignment**: Matches common allocator bucket sizes, avoiding internal
///   fragmentation in the channel implementation.
///
/// Adjust via [`body_channel`] when your workload has different
/// characteristics (for example, very large chunks or strict memory limits).
pub const DEFAULT_BODY_CHANNEL_CAPACITY: usize = 16;

/// Create a bounded channel for streaming request bodies.
///
/// Returns a sender (for the connection to push chunks) and a
/// [`RequestBodyStream`] (for the handler to consume). Back-pressure
/// propagates when the channel fills: senders await capacity before
/// buffering additional chunks.
///
/// # Arguments
///
/// * `capacity` - Maximum number of chunks to buffer before applying back-pressure. Must be greater
///   than zero.
///
/// # Panics
///
/// Panics if `capacity` is zero, mirroring [`tokio::sync::mpsc::channel`].
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use wireframe::request::body_channel;
///
/// let (tx, rx) = body_channel(8);
/// // tx: send chunks from connection
/// // rx: consume in handler
/// ```
#[must_use]
pub fn body_channel(
    capacity: usize,
) -> (mpsc::Sender<Result<Bytes, io::Error>>, RequestBodyStream) {
    let (tx, rx) = mpsc::channel(capacity);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    (tx, Box::pin(stream))
}

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
    #[must_use]
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

/// Request metadata extracted outwith the request body.
///
/// `RequestParts` separates routing and protocol metadata from the request
/// payload, enabling handlers to begin processing before the full body
/// arrives. This struct is the counterpart to streaming response types
/// ([`crate::Response::Stream`]) for the inbound direction.
///
/// # Differences from [`crate::app::PacketParts`]
///
/// - [`crate::app::PacketParts`] carries the raw `payload` for packet routing and envelope
///   reconstruction.
/// - `RequestParts` carries protocol-defined `metadata` bytes (for example headers) that handlers
///   need to interpret the body, but excludes the body itself.
///
/// # Examples
///
/// ```
/// use wireframe::request::RequestParts;
///
/// let parts = RequestParts::new(42, Some(123), vec![0x01, 0x02]);
/// assert_eq!(parts.id(), 42);
/// assert_eq!(parts.correlation_id(), Some(123));
/// assert_eq!(parts.metadata(), &[0x01, 0x02]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestParts {
    /// Protocol-specific packet or message identifier used for routing.
    id: u32,
    /// Correlation identifier, if present.
    correlation_id: Option<u64>,
    /// Protocol-defined metadata bytes (for example headers) required to
    /// interpret the body.
    metadata: Vec<u8>,
}

impl RequestParts {
    /// Construct a new set of request parts.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![0xab, 0xcd]);
    /// assert_eq!(parts.id(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, metadata: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            metadata,
        }
    }

    /// Return the message identifier used to route this request.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(9, None, vec![]);
    /// assert_eq!(parts.id(), 9);
    /// ```
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }

    /// Retrieve the correlation identifier, if present.
    ///
    /// The correlation identifier ties this request to a logical session
    /// or prior exchange when the protocol supports request chaining.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, Some(42), vec![]);
    /// assert_eq!(parts.correlation_id(), Some(42));
    /// ```
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Return a reference to the protocol-defined metadata bytes.
    ///
    /// Metadata typically contains protocol headers, content-type markers,
    /// or other information required to interpret the streaming body.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![0x01, 0x02, 0x03]);
    /// assert_eq!(parts.metadata(), &[0x01, 0x02, 0x03]);
    /// ```
    #[must_use]
    pub fn metadata(&self) -> &[u8] { &self.metadata }

    /// Return a mutable reference to the protocol-defined metadata bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let mut parts = RequestParts::new(1, None, vec![0x01]);
    /// parts.metadata_mut().push(0x02);
    /// assert_eq!(parts.metadata(), &[0x01, 0x02]);
    /// ```
    pub fn metadata_mut(&mut self) -> &mut Vec<u8> { &mut self.metadata }

    /// Consume the parts and return the metadata bytes.
    ///
    /// Use this when ownership of the metadata is required without
    /// additional allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![7, 8]);
    /// let metadata = parts.into_metadata();
    /// assert_eq!(metadata, vec![7, 8]);
    /// ```
    #[must_use]
    pub fn into_metadata(self) -> Vec<u8> { self.metadata }

    /// Ensure a correlation identifier is present, inheriting from `source`
    /// if missing.
    ///
    /// This mirrors [`crate::app::PacketParts::inherit_correlation`] to ensure
    /// consistent correlation handling across request and response paths.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// // Inherit when missing
    /// let parts = RequestParts::new(1, None, vec![]).inherit_correlation(Some(42));
    /// assert_eq!(parts.correlation_id(), Some(42));
    ///
    /// // Overwrite mismatched value
    /// let parts = RequestParts::new(1, Some(7), vec![]).inherit_correlation(Some(8));
    /// assert_eq!(parts.correlation_id(), Some(8));
    /// ```
    #[must_use]
    pub fn inherit_correlation(mut self, source: Option<u64>) -> Self {
        let result = Self::select_correlation(self.correlation_id, source);
        if let CorrelationResult::Mismatch { found, expected } = result {
            log::warn!(
                "mismatched correlation id in request: id={}, expected={expected}, found={found}",
                self.id,
            );
        }
        self.correlation_id = result.value();
        self
    }

    #[inline]
    fn select_correlation(current: Option<u64>, source: Option<u64>) -> CorrelationResult {
        match (current, source) {
            (None, cid) => CorrelationResult::Inherited(cid),
            (Some(found), Some(expected)) if found != expected => {
                CorrelationResult::Mismatch { found, expected }
            }
            (curr, _) => CorrelationResult::Inherited(curr),
        }
    }
}

/// Result of correlation ID selection when inheriting from a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorrelationResult {
    /// The correlation ID was inherited (or remained unchanged).
    Inherited(Option<u64>),
    /// A mismatch was detected; the source value takes precedence.
    Mismatch { found: u64, expected: u64 },
}

impl CorrelationResult {
    /// Extract the final correlation ID value.
    const fn value(self) -> Option<u64> {
        match self {
            Self::Inherited(cid) => cid,
            Self::Mismatch { expected, .. } => Some(expected),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stream_tests;
