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
    /// Creates a new `SharedState` instance wrapping the provided `Arc<T>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// let state = Arc::new(42);
    /// let shared = SharedState::new(state.clone());
    /// assert_eq!(*shared, 42);
    /// ```
    pub fn new(inner: Arc<T>) -> Self {
        Self(inner)
    }
}

impl<T> std::ops::Deref for SharedState<T> {
    type Target = T;

    /// Returns a reference to the inner shared state value.
    ///
    /// This allows transparent access to the underlying state managed by `SharedState`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// let state = Arc::new(42);
    /// let shared = SharedState::new(state.clone());
    /// assert_eq!(*shared, 42);
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
