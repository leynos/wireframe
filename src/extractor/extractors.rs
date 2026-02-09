//! Built-in extractor implementations for message, streaming body, and connection metadata.

use std::net::SocketAddr;

use super::{ExtractError, FromMessageRequest, MessageRequest, Payload};
use crate::message::Message as WireMessage;

/// Extractor that deserializes the message payload into `T`.
#[derive(Debug, Clone)]
pub struct Message<T>(T);

impl<T> Message<T> {
    /// Consumes the extractor and returns the inner deserialized message value.
    #[must_use]
    pub fn into_inner(self) -> T { self.0 }
}

impl<T> std::ops::Deref for Message<T> {
    type Target = T;

    /// Returns a reference to the inner value.
    ///
    /// This enables transparent access to the wrapped type via dereferencing.
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T> FromMessageRequest for Message<T>
where
    T: WireMessage,
{
    type Error = ExtractError;

    /// Attempts to extract and deserialize a message of type `T` from the payload.
    ///
    /// Advances the payload by the number of bytes consumed during deserialization.
    /// Returns an error if the payload cannot be decoded into the target type.
    ///
    /// # Returns
    /// - `Ok(Self)`: The successfully extracted and deserialized message.
    /// - `Err(ExtractError::InvalidPayload)`: If deserialization fails.
    fn from_message_request(
        _req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        let (msg, consumed) =
            T::from_bytes(payload.as_ref()).map_err(ExtractError::InvalidPayload)?;
        payload.advance(consumed);
        Ok(Self(msg))
    }
}

/// Extractor providing streaming access to the request body.
///
/// Unlike [`Payload`] which borrows buffered bytes, this extractor
/// takes ownership of a streaming body channel. Handlers opting into
/// streaming receive chunks incrementally via a [`RequestBodyStream`].
///
/// This type is the inbound counterpart to [`crate::Response::Stream`].
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

/// Extractor providing peer connection metadata.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionInfo {
    peer_addr: Option<SocketAddr>,
}

impl ConnectionInfo {
    /// Returns the peer's socket address for the current connection, if available.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::extractor::{ConnectionInfo, FromMessageRequest, MessageRequest, Payload};
    ///
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket address");
    /// let req = MessageRequest::new().with_peer_addr(Some(addr));
    /// let info = ConnectionInfo::from_message_request(&req, &mut Payload::default())
    ///     .expect("connection info extraction should succeed");
    /// assert_eq!(info.peer_addr(), Some(addr));
    /// ```
    #[must_use]
    pub fn peer_addr(&self) -> Option<SocketAddr> { self.peer_addr }
}

impl FromMessageRequest for ConnectionInfo {
    type Error = std::convert::Infallible;

    /// Extracts connection metadata from the message request.
    ///
    /// Returns a `ConnectionInfo` containing the peer's socket address, if available. This
    /// extraction is infallible.
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            peer_addr: req.peer_addr,
        })
    }
}
