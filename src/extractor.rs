use async_trait::async_trait;
use std::sync::Arc;

/// Request context passed to extractors.
///
/// This type contains metadata about the current connection and provides
/// access to application state registered with [`WireframeApp`].
#[derive(Default)]
pub struct MessageRequest {
    // TODO: populate with connection info and state storage
}

/// Raw payload buffer handed to extractors.
#[derive(Default)]
pub struct Payload<'a> {
    /// Incoming bytes not yet processed.
    pub data: &'a [u8],
}

/// Asynchronous extractor trait.
#[async_trait]
pub trait FromMessageRequest: Sized {
    /// Error type returned when extraction fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Perform extraction from the request and payload.
    async fn from_message_request(
        req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error>;
}

/// Shared application state accessible to handlers.
#[derive(Clone)]
pub struct SharedState<T>(Arc<T>);

impl<T> SharedState<T> {
    /// Construct a new [`SharedState`].
    #[must_use]
    pub fn new(inner: Arc<T>) -> Self {
        Self(inner)
    }
}

impl<T> std::ops::Deref for SharedState<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
