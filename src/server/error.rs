//! Errors raised by [`super::WireframeServer`] operations.

use std::{error::Error, io, sync::Arc};

use static_assertions::assert_impl_all;
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

    /// The server supervisor task terminated unexpectedly.
    #[error("server supervisor terminated abnormally: {message}")]
    AbnormalTermination {
        /// Diagnostic captured from the supervisor failure or cancellation.
        message: String,
        /// Original supervisor error, when termination returned an error.
        #[source]
        source: Option<Arc<Self>>,
    },
}

// Shared through Arc and across tasks; a non-Send or non-Sync payload would break callers.
assert_impl_all!(ServerError: Send, Sync);
