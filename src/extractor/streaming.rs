//! Streaming body extractor.

use super::{ExtractError, FromMessageRequest, MessageRequest, Payload};

/// Extractor providing streaming access to the request body.
///
/// Unlike [`Payload`] which borrows buffered bytes, this extractor
/// takes ownership of a streaming body channel. Handlers opting into
/// streaming receive chunks incrementally via a [`RequestBodyStream`].
///
/// This type is the inbound counterpart to
/// [`crate::response::Response::Stream`].
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use tokio::io::AsyncReadExt;
/// use wireframe::{extractor::StreamingBody, request::body_channel};
///
/// # #[tokio::main]
/// # async fn main() {
/// let (tx, stream) = body_channel(4);
///
/// tokio::spawn(async move {
///     let _ = tx.send(Ok(Bytes::from_static(b"payload"))).await;
/// });
///
/// let body = StreamingBody::new(stream);
/// let mut reader = body.into_reader();
/// let mut buf = Vec::new();
/// reader.read_to_end(&mut buf).await.expect("read body");
/// assert_eq!(buf, b"payload");
/// # }
/// ```
///
/// [`RequestBodyStream`]: crate::request::RequestBodyStream
pub struct StreamingBody {
    stream: crate::request::RequestBodyStream,
}

impl StreamingBody {
    /// Create a streaming body from the given stream.
    ///
    /// Typically constructed by the framework when a handler opts into
    /// streaming request consumption.
    #[must_use]
    pub fn new(stream: crate::request::RequestBodyStream) -> Self { Self { stream } }

    /// Consume the extractor and return the underlying stream.
    ///
    /// Use this when you need direct access to the stream for custom
    /// processing with [`futures::StreamExt`] methods.
    #[must_use]
    pub fn into_stream(self) -> crate::request::RequestBodyStream { self.stream }

    /// Convert to an [`AsyncRead`] adapter.
    ///
    /// Protocol crates can use this to feed streaming bytes into parsers
    /// that operate on readers rather than streams.
    ///
    /// [`AsyncRead`]: tokio::io::AsyncRead
    #[must_use]
    pub fn into_reader(self) -> crate::request::RequestBodyReader {
        crate::request::RequestBodyReader::new(self.stream)
    }
}

impl std::fmt::Debug for StreamingBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingBody").finish_non_exhaustive()
    }
}

impl FromMessageRequest for StreamingBody {
    type Error = ExtractError;

    /// Extract the streaming body from the request.
    ///
    /// # Errors
    ///
    /// Returns [`ExtractError::MissingBodyStream`] if:
    /// - The request was not configured for streaming consumption
    /// - The stream was already consumed by another extractor
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        req.take_body_stream()
            .map(Self::new)
            .ok_or(ExtractError::MissingBodyStream)
    }
}
