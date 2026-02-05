//! Error types for built-in extractors.

/// Errors that can occur when extracting built-in types.
///
/// This enum is marked `#[non_exhaustive]` so more variants may be added in
/// the future without breaking changes.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExtractError {
    /// No shared state of the requested type was found.
    MissingState(&'static str),
    /// Failed to decode the message payload.
    InvalidPayload(bincode::error::DecodeError),
    /// No streaming body was available for this request.
    ///
    /// This occurs when:
    /// - The request was not configured for streaming consumption
    /// - The stream was already consumed by another extractor
    MissingBodyStream,
}

impl std::fmt::Display for ExtractError {
    /// Formats the `ExtractError` for display purposes.
    ///
    /// Displays a descriptive message for missing shared state or payload decoding errors.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingState(ty) => write!(f, "no shared state registered for {ty}"),
            Self::InvalidPayload(e) => write!(f, "failed to decode payload: {e}"),
            Self::MissingBodyStream => {
                write!(f, "no streaming body available for this request")
            }
        }
    }
}

impl std::error::Error for ExtractError {
    /// Returns the underlying error if this is an `InvalidPayload` variant.
    ///
    /// # Returns
    /// An optional reference to the underlying decode error, or `None` if not applicable.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPayload(e) => Some(e),
            _ => None,
        }
    }
}
