use std::net::SocketAddr;
use std::sync::Arc;

/// Request context passed to extractors.
///
/// This type contains metadata about the current connection and provides
/// access to application state registered with [`WireframeApp`].
#[derive(Default)]
pub struct MessageRequest {
    /// Address of the peer that sent the current message.
    pub peer_addr: Option<SocketAddr>,
}

/// Raw payload buffer handed to extractors.
#[derive(Default)]
pub struct Payload<'a> {
    /// Incoming bytes not yet processed.
    pub data: &'a [u8],
}

impl Payload<'_> {
    /// Advances the payload by `count` bytes.
    ///
    /// Consumes up to `count` bytes from the front of the slice, ensuring we
    /// never slice beyond the available buffer.
    pub fn advance(&mut self, count: usize) {
        let n = count.min(self.data.len());
        self.data = &self.data[n..];
    }

    /// Returns the number of bytes remaining.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len()
    }
}

/// Trait for extracting data from a [`MessageRequest`].
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

/// Shared application state accessible to handlers.
#[derive(Clone)]
pub struct SharedState<T: Send + Sync>(Arc<T>);

impl<T: Send + Sync> SharedState<T> {
    /// Construct a new [`SharedState`].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use wireframe::extractor::SharedState;
    ///
    /// let data = Arc::new(5u32);
    /// let state = SharedState::new(Arc::clone(&data));
    /// assert_eq!(*state, 5);
    /// ```
    #[must_use]
    pub fn new(inner: Arc<T>) -> Self {
        Self(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_consumes_bytes() {
        let mut payload = Payload { data: b"hello" };
        payload.advance(2);
        assert_eq!(payload.data, b"llo");
        payload.advance(10);
        assert!(payload.data.is_empty());
    }

    #[test]
    fn remaining_reports_length() {
        let mut payload = Payload { data: b"abc" };
        assert_eq!(payload.remaining(), 3);
        payload.advance(1);
        assert_eq!(payload.remaining(), 2);
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
    /// ```
    /// use std::sync::Arc;
    /// use wireframe::extractor::SharedState;
    /// let state = Arc::new(42);
    /// let shared = SharedState::new(state.clone());
    /// assert_eq!(*shared, 42);
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
