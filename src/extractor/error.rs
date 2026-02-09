//! Error types for built-in extractors.

use thiserror::Error;

/// Errors that can occur when extracting built-in types.
///
/// This enum is marked `#[non_exhaustive]` so more variants may be added in
/// the future without breaking changes.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExtractError {
    /// No shared state of the requested type was found.
    #[error("no shared state registered for {0}")]
    MissingState(&'static str),
    /// Failed to decode the message payload.
    #[error("failed to decode payload: {0}")]
    InvalidPayload(#[source] bincode::error::DecodeError),
    /// No streaming body was available for this request.
    ///
    /// This occurs when:
    /// - The request was not configured for streaming consumption
    /// - The stream was already consumed by another extractor
    #[error("no streaming body available for this request")]
    MissingBodyStream,
}
