//! Errors raised by [`WireframeServer`] operations.

use std::io;

use thiserror::Error;

/// Errors that may occur while configuring or running the server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Binding or configuring the listener failed.
    #[error("bind error: {0}")]
    Bind(#[source] io::Error),

    /// Accepting a connection failed.
    #[error("accept error: {0}")]
    Accept(#[source] io::Error),
}
