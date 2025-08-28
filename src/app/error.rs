//! Error types for application setup and messaging.

use std::io;

use thiserror::Error;

/// Top-level error type for application setup.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireframeError {
    /// A route with the provided identifier was already registered.
    #[error("route id {0} was already registered")]
    DuplicateRoute(u32),
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
}

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;
