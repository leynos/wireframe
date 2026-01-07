//! Error types for application setup and messaging.

use std::io;

use thiserror::Error;

use crate::codec::CodecError;

/// Top-level error type for application setup and codec operations.
///
/// This enum covers errors that occur during application configuration (such as
/// duplicate route registration) and codec-layer errors that occur during
/// connection processing.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WireframeError {
    /// A route with the provided identifier was already registered.
    #[error("route id {0} was already registered")]
    DuplicateRoute(u32),
    /// A codec-layer error occurred during connection processing.
    ///
    /// Codec errors are categorised by their origin: framing errors (wire-level
    /// frame boundary issues), protocol errors (semantic violations), I/O
    /// errors, or EOF conditions. See [`CodecError`] for details.
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
}

/// Errors produced when sending a handler response over a stream.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SendError {
    /// Serialisation failed.
    #[error("serialisation error: {0}")]
    Serialize(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Writing to the stream failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A codec-layer error occurred.
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
}

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;
