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

/// Errors emitted by [`crate::client::WireframeClient`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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
    /// This error is returned by
    /// [`crate::client::WireframeClient::call_correlated`]
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
    #[rustfmt::skip]
    #[error("correlation ID mismatch in streaming response: expected {expected:?}, received {received:?}")]
    StreamCorrelationMismatch {
        /// The correlation ID sent with the request.
        expected: Option<u64>,
        /// The correlation ID received in the response frame.
        received: Option<u64>,
    },
    /// The pool already has the configured number of queued acquisitions.
    #[error("client pool waiter limit exceeded ({limit})")]
    PoolSaturated {
        /// Maximum number of acquisitions admitted to the waiter queues.
        limit: usize,
    },
    /// The pool has been closed and will not grant further leases.
    ///
    /// Queued acquisitions are resolved with this error when the pool closes,
    /// so a caller waiting for capacity learns promptly that the pool is gone.
    #[error("client pool is closed")]
    PoolClosed,
    /// The pool's scheduler task terminated abnormally.
    ///
    /// This indicates an internal fault rather than an orderly shutdown. The
    /// pool is not restarted; callers should discard it and reconnect.
    #[error("client pool scheduler failed")]
    SchedulerFailed,
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

    /// Build the typed error returned when the pool has no waiter capacity.
    #[cfg(feature = "pool")]
    pub(crate) const fn pool_saturated(limit: usize) -> Self { Self::PoolSaturated { limit } }

    /// Build the typed error returned once the pool has closed.
    #[cfg(feature = "pool")]
    pub(crate) const fn pool_closed() -> Self { Self::PoolClosed }

    /// Build the typed error returned when the pool scheduler failed.
    #[cfg(feature = "pool")]
    pub(crate) const fn scheduler_failed() -> Self { Self::SchedulerFailed }

    /// Return `true` when this error leaves the underlying connection unsafe
    /// to reuse in the pool.
    ///
    /// Pool lifecycle errors describe the pool, not the connection: a lease
    /// refused because the pool closed says nothing about any socket's
    /// protocol state, so none of them require recycling.
    #[must_use]
    pub const fn should_recycle_connection(&self) -> bool {
        match self {
            Self::Wireframe(_)
            | Self::PreambleWrite(_)
            | Self::PreambleRead(_)
            | Self::PreambleTimeout
            | Self::CorrelationMismatch { .. }
            | Self::StreamCorrelationMismatch { .. } => true,
            Self::Serialize(_)
            | Self::PreambleEncode(_)
            | Self::PoolSaturated { .. }
            | Self::PoolClosed
            | Self::SchedulerFailed => false,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(value: io::Error) -> Self { Self::Wireframe(ClientWireframeError::from_io(value)) }
}

#[cfg(test)]
mod tests {
    //! Coverage for client-side error mapping and connection reuse rules.

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

    #[test]
    fn should_recycle_connection_returns_true_for_transport_and_protocol_failures() {
        let io_error = ClientError::from(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "transport failure",
        ));
        let decode_error = ClientError::decode(Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "decode failure",
        )));
        let correlation_error = ClientError::CorrelationMismatch {
            expected: Some(1),
            received: Some(2),
        };

        assert!(
            io_error.should_recycle_connection(),
            "assertion failed: io_error.should_recycle_connection()"
        );
        assert!(
            decode_error.should_recycle_connection(),
            "assertion failed: decode_error.should_recycle_connection()"
        );
        assert!(
            correlation_error.should_recycle_connection(),
            "assertion failed: correlation_error.should_recycle_connection()"
        );
    }

    #[test]
    fn should_recycle_connection_returns_false_for_serialize_failures() {
        let error = ClientError::Serialize(Box::new(io::Error::other("serialize failure")));

        assert!(
            !error.should_recycle_connection(),
            "assertion failed: !error.should_recycle_connection()"
        );
    }

    #[rstest]
    #[case(1)]
    #[case(16)]
    fn pool_saturation_does_not_recycle_a_connection(#[case] limit: usize) {
        let error = ClientError::PoolSaturated { limit };

        assert!(
            matches!(error, ClientError::PoolSaturated { limit: actual } if actual == limit),
            "assertion failed: matches!(error, ClientError::PoolSaturated {{ limit: actual }} if \
             actual == limit)"
        );
        assert!(
            !error.should_recycle_connection(),
            "assertion failed: !error.should_recycle_connection()"
        );
    }

    #[rstest]
    #[case(ClientError::PoolClosed, "client pool is closed")]
    #[case(ClientError::SchedulerFailed, "client pool scheduler failed")]
    fn pool_lifecycle_errors_do_not_recycle_a_connection(
        #[case] error: ClientError,
        #[case] expected_message: &str,
    ) {
        assert_eq!(
            error.to_string(),
            expected_message,
            "error.to_string() should equal expected_message"
        );
        assert!(
            !error.should_recycle_connection(),
            "a pool lifecycle error must not discard a healthy socket"
        );
    }
}
