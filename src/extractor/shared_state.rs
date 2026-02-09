//! Shared application state extractor.

use std::sync::Arc;

use super::{ExtractError, FromMessageRequest, MessageRequest, Payload};

/// Shared application state accessible to handlers.
#[derive(Clone)]
pub struct SharedState<T: Send + Sync>(Arc<T>);

impl<T: Send + Sync> std::fmt::Debug for SharedState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState").finish_non_exhaustive()
    }
}

impl<T: Send + Sync> From<Arc<T>> for SharedState<T> {
    fn from(inner: Arc<T>) -> Self { Self(inner) }
}

impl<T: Send + Sync> From<T> for SharedState<T> {
    fn from(inner: T) -> Self { Self(Arc::new(inner)) }
}

impl<T> FromMessageRequest for SharedState<T>
where
    T: Send + Sync + 'static,
{
    type Error = ExtractError;

    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        req.state::<T>()
            .ok_or(ExtractError::MissingState(std::any::type_name::<T>()))
    }
}

impl<T: Send + Sync> std::ops::Deref for SharedState<T> {
    type Target = T;

    /// Returns a reference to the inner shared state value.
    ///
    /// This allows transparent access to the underlying state managed by `SharedState`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use wireframe::extractor::SharedState;
    ///
    /// let state = Arc::new(42);
    /// let shared: SharedState<u32> = state.clone().into();
    /// assert_eq!(*shared, 42);
    /// ```
    fn deref(&self) -> &Self::Target { &self.0 }
}
