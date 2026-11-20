//! Error types for application setup and messaging.

use std::io;

use thiserror::Error;

use crate::codec::CodecError;

/// Errors produced when sending a handler response over a stream.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SendError {
    /// Serialization failed.
    #[error("serialization error: {0}")]
    Serialize(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Writing to the stream failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A codec-layer error occurred.
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
}

/// Errors produced while preparing an application for connection handling.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PrepareError {
    /// A future middleware transform failed while preparing a route.
    #[error("route middleware transformation failed: {source}")]
    MiddlewareTransform {
        /// The transform failure that prevented preparation.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The stored push-queue template is not a usable configuration.
    ///
    /// Preparation rejects the template before any connection exists, so an
    /// invalid capacity or rate cannot surface later as a per-connection
    /// failure.
    #[error("invalid push queue configuration: {source}")]
    InvalidPushConfig {
        /// The capacity or rate problem reported by the push configuration.
        #[source]
        source: crate::push::PushConfigError,
    },
}

/// Result type used throughout the builder API.
pub type Result<T> = crate::Result<T>;
