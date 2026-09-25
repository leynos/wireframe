//! Errors raised by [`super::WireframeServer`] operations.

use std::{error::Error, io};

use thiserror::Error;

use crate::app::PrepareError;

/// Errors that may occur while configuring or running the server.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ServerError {
    /// Binding or configuring the listener failed.
    #[error("bind error: {0}")]
    Bind(#[source] io::Error),

    /// Accepting a connection failed.
    #[error("accept error: {0}")]
    Accept(#[source] io::Error),

    /// Evaluating the application factory during startup failed.
    #[error("application factory error: {0}")]
    FactoryBuild(#[source] Box<dyn Error + Send + Sync>),

    /// Preparing the application during startup failed.
    #[error("application preparation error: {0}")]
    Prepare(#[source] PrepareError),
}
