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
//!     // Consume body chunks incrementally via StreamExt or AsyncRead adaptor
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

/// Adaptor wrapping a [`RequestBodyStream`] as [`AsyncRead`].
///
/// Protocol crates can use this to feed streaming bytes into parsers
/// that operate on readers rather than streams. The adaptor consumes
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
    /// Any buffered bytes from partial reads are discarded.
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
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn new_stores_all_fields() {
        let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
        assert_eq!(parts.id(), 42);
        assert_eq!(parts.correlation_id(), Some(100));
        assert_eq!(parts.metadata(), &[0x01, 0x02]);
    }

    #[rstest]
    fn metadata_returns_reference() {
        let parts = RequestParts::new(1, None, vec![0xab, 0xcd, 0xef]);
        let meta = parts.metadata();
        assert_eq!(meta.len(), 3);
        assert_eq!(meta.first(), Some(&0xab));
    }

    #[rstest]
    fn metadata_mut_allows_modification() {
        let mut parts = RequestParts::new(1, None, vec![0x01]);
        parts.metadata_mut().push(0x02);
        assert_eq!(parts.metadata(), &[0x01, 0x02]);
    }

    #[rstest]
    fn into_metadata_transfers_ownership() {
        let parts = RequestParts::new(1, None, vec![7, 8, 9]);
        let owned = parts.into_metadata();
        assert_eq!(owned, vec![7, 8, 9]);
    }

    #[rstest]
    fn clone_produces_equal_instance() {
        let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
        let cloned = parts.clone();
        assert_eq!(parts, cloned);
    }

    #[rstest]
    #[case(RequestParts::new(1, None, vec![]), Some(42), Some(42))]
    #[case(RequestParts::new(1, Some(7), vec![]), None, Some(7))]
    #[case(RequestParts::new(1, None, vec![]), None, None)]
    #[case(RequestParts::new(1, Some(7), vec![]), Some(8), Some(8))]
    fn inherit_correlation_variants(
        #[case] start: RequestParts,
        #[case] source: Option<u64>,
        #[case] expected: Option<u64>,
    ) {
        let got = start.inherit_correlation(source);
        assert_eq!(got.correlation_id(), expected);
    }

    #[rstest]
    fn empty_metadata_is_valid() {
        let parts = RequestParts::new(1, None, vec![]);
        assert!(parts.metadata().is_empty());
    }
}

#[cfg(test)]
mod stream_tests {
    use futures::StreamExt;
    use rstest::rstest;
    use tokio::io::AsyncReadExt;

    use super::*;

    #[rstest]
    #[tokio::test]
    async fn request_body_stream_yields_chunks() {
        let chunks = vec![
            Ok(Bytes::from_static(b"hello")),
            Ok(Bytes::from_static(b" world")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

        let collected: Vec<_> = stream.collect().await;
        assert_eq!(collected.len(), 2);
        assert!(collected.iter().all(Result::is_ok));
    }

    #[rstest]
    #[tokio::test]
    async fn async_read_adaptor_reads_stream() {
        let chunks = vec![Ok(Bytes::from_static(b"test"))];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = [0u8; 4];
        let n = reader.read(&mut buf).await.expect("read should succeed");
        assert_eq!(n, 4);
        assert_eq!(&buf, b"test");
    }

    #[rstest]
    #[tokio::test]
    async fn reader_consumes_multiple_chunks() {
        let chunks = vec![
            Ok(Bytes::from_static(b"hello ")),
            Ok(Bytes::from_static(b"world")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .await
            .expect("read_to_end should succeed");
        assert_eq!(buf, b"hello world");
    }

    #[rstest]
    #[tokio::test]
    async fn stream_propagates_io_error() {
        let chunks = vec![
            Ok(Bytes::from_static(b"ok")),
            Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

        let results: Vec<_> = stream.collect().await;
        assert!(results.first().expect("first result").is_ok());
        assert!(results.get(1).expect("second result").is_err());
    }

    #[rstest]
    #[tokio::test]
    async fn reader_surfaces_stream_error() {
        let chunks = vec![Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "disconnected",
        ))];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = [0u8; 16];
        let result = reader.read(&mut buf).await;
        let err = result.expect_err("read should fail");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[rstest]
    #[tokio::test]
    async fn body_channel_delivers_chunks() {
        let (tx, stream) = body_channel(4);

        tx.send(Ok(Bytes::from_static(b"chunk1")))
            .await
            .expect("send should succeed");
        tx.send(Ok(Bytes::from_static(b"chunk2")))
            .await
            .expect("send should succeed");
        drop(tx);

        let chunks: Vec<_> = stream.collect().await;
        assert_eq!(chunks.len(), 2);
    }

    #[rstest]
    #[tokio::test]
    async fn body_channel_back_pressure() {
        let (tx, _rx) = body_channel(1);

        tx.send(Ok(Bytes::from_static(b"first")))
            .await
            .expect("first send should succeed");

        // Channel is full; try_send should fail
        let result = tx.try_send(Ok(Bytes::from_static(b"second")));
        assert!(result.is_err(), "try_send should fail when channel is full");
    }

    #[rstest]
    #[tokio::test]
    async fn body_channel_sender_detects_dropped_receiver() {
        let (tx, rx) = body_channel(1);
        drop(rx);

        let result = tx.send(Ok(Bytes::from_static(b"orphan"))).await;
        assert!(result.is_err(), "send should fail when receiver is dropped");
    }

    #[rstest]
    fn reader_into_inner_returns_stream() {
        let chunks = vec![Ok(Bytes::from_static(b"test"))];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let reader = RequestBodyReader::new(stream);

        let _recovered: RequestBodyStream = reader.into_inner();
        // Compiles and type-checks; stream is returned
    }
}
