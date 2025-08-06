//! Errors raised by [`WireframeServer`] operations.

use std::io;

use thiserror::Error;

/// Errors that may occur while running the server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Accepting a connection failed.
    #[error("accept error: {0}")]
    Accept(#[from] io::Error),
}
