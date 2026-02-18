//! Error types for application setup and messaging.

use std::io;

use thiserror::Error;

use crate::codec::CodecError;

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
pub type Result<T> = crate::Result<T>;
