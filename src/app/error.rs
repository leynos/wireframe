//! Error types for application setup and messaging.

use tokio::io;

/// Top-level error type for application setup.
#[derive(Debug)]
pub enum WireframeError {
    /// A route with the provided identifier was already registered.
    DuplicateRoute(u32),
}

/// Errors produced when sending a handler response over a stream.
#[derive(Debug)]
pub enum SendError {
    /// Serialization failed.
    Serialize(Box<dyn std::error::Error + Send + Sync>),
    /// Writing to the stream failed.
    Io(io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Serialize(e) => write!(f, "serialization error: {e}"),
            SendError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::Serialize(e) => Some(&**e),
            SendError::Io(e) => Some(e),
        }
    }
}

impl From<io::Error> for SendError {
    fn from(e: io::Error) -> Self { SendError::Io(e) }
}

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;
