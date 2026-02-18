//! Error types for wireframe client operations.

use std::io;

use crate::WireframeError;

/// Protocol-level failures surfaced by client request/response operations.
///
/// These errors are wrapped in [`WireframeError::Protocol`] when the client
/// pipeline can read a frame but cannot decode it into the requested type.
#[derive(Debug, thiserror::Error)]
pub enum ClientProtocolError {
    /// Failed to deserialize an inbound message payload.
    #[error("failed to deserialize message")]
    Deserialize(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Wireframe error type used by client request/response operations.
///
/// Transport failures are mapped to [`WireframeError::Io`]. Decode failures
/// are mapped to [`WireframeError::Protocol`], carrying
/// [`ClientProtocolError`].
pub type ClientWireframeError = WireframeError<ClientProtocolError>;

/// Errors emitted by [`crate::WireframeClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Request/response transport or protocol failure.
    #[error(transparent)]
    Wireframe(#[from] ClientWireframeError),
    /// Failed to serialize an outbound message.
    #[error("failed to serialize message")]
    Serialize(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Failed to encode the connection preamble.
    #[error("failed to encode preamble")]
    PreambleEncode(#[source] bincode::error::EncodeError),
    /// I/O error writing the connection preamble.
    #[error("failed to write preamble: {0}")]
    PreambleWrite(#[source] io::Error),
    /// I/O error reading the server's preamble response.
    #[error("failed to read preamble response: {0}")]
    PreambleRead(#[source] io::Error),
    /// Preamble exchange timed out.
    #[error("preamble exchange timed out")]
    PreambleTimeout,
    /// Response correlation ID does not match the request.
    ///
    /// This error is returned by [`crate::WireframeClient::call_correlated`]
    /// when the response envelope's correlation ID differs from the request's.
    #[error("correlation ID mismatch: expected {expected:?}, received {received:?}")]
    CorrelationMismatch {
        /// The correlation ID sent with the request.
        expected: Option<u64>,
        /// The correlation ID received in the response.
        received: Option<u64>,
    },
    /// A data frame within a streaming response carried a correlation ID that
    /// does not match the request.
    ///
    /// This error is returned by
    /// [`ResponseStream`](crate::client::ResponseStream) when a data frame
    /// arrives with an unexpected correlation identifier.
    #[error(
        "correlation ID mismatch in streaming response: expected {expected:?}, received \
         {received:?}"
    )]
    StreamCorrelationMismatch {
        /// The correlation ID sent with the request.
        expected: Option<u64>,
        /// The correlation ID received in the response frame.
        received: Option<u64>,
    },
}

impl ClientError {
    /// Build a protocol decode error for the request/response pipeline.
    pub(crate) fn decode(source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Wireframe(ClientWireframeError::Protocol(
            ClientProtocolError::Deserialize(source),
        ))
    }

    /// Build a transport error for the request/response pipeline.
    pub(crate) fn disconnected() -> Self {
        Self::from(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed by peer",
        ))
    }
}

impl From<io::Error> for ClientError {
    fn from(value: io::Error) -> Self { Self::Wireframe(ClientWireframeError::from_io(value)) }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(io::ErrorKind::BrokenPipe)]
    #[case(io::ErrorKind::UnexpectedEof)]
    fn io_errors_map_to_wireframe_transport_variant(#[case] kind: io::ErrorKind) {
        let err = ClientError::from(io::Error::new(kind, "transport failure"));

        assert!(
            matches!(err, ClientError::Wireframe(ClientWireframeError::Io(_))),
            "I/O errors should map to ClientError::Wireframe(WireframeError::Io(_))"
        );
    }

    #[test]
    fn decode_helper_maps_to_wireframe_protocol_variant() {
        let err = ClientError::decode(Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "decode failure",
        )));

        assert!(
            matches!(
                err,
                ClientError::Wireframe(ClientWireframeError::Protocol(
                    ClientProtocolError::Deserialize(_)
                ))
            ),
            "decode errors should map to ClientError::Wireframe(WireframeError::Protocol(_))"
        );
    }
}
