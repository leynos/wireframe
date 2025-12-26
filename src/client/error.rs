//! Error types for wireframe client operations.

use std::io;

/// Errors emitted by [`crate::WireframeClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Transport or codec error.
    #[error("transport error: {0}")]
    Io(#[from] io::Error),
    /// Failed to serialize an outbound message.
    #[error("failed to serialize message")]
    Serialize(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Failed to deserialize an inbound message.
    #[error("failed to deserialize message")]
    Deserialize(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The peer closed the connection before a response arrived.
    #[error("connection closed by peer")]
    Disconnected,
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
}
