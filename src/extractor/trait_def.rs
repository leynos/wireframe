//! Trait definition for extractor types.

use super::{MessageRequest, Payload};

/// Trait for extracting data from a [`MessageRequest`].
///
/// Types implementing this trait can be used as parameters to handler
/// functions. When invoked, `wireframe` passes the current request metadata and
/// message payload, allowing the extractor to parse bytes or inspect
/// connection information. This makes it easy to share common parsing and
/// validation logic across handlers.
pub trait FromMessageRequest: Sized {
    /// Error type returned when extraction fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Perform extraction from the request and payload.
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails.
    fn from_message_request(
        req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error>;
}
