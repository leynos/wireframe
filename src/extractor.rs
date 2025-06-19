//! Extractor and request context definitions.
//!
//! This module provides [`MessageRequest`], which carries connection
//! metadata and shared application state, along with a set of extractor
//! types. Implement [`FromMessageRequest`] for custom extractors to
//! parse payload bytes or inspect connection info before your handler
//! runs.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};

use crate::message::Message as WireMessage;

/// Request context passed to extractors.
///
/// This type contains metadata about the current connection and provides
/// access to application state registered with [`WireframeApp`].
#[derive(Default)]
pub struct MessageRequest {
    /// Address of the peer that sent the current message.
    pub peer_addr: Option<SocketAddr>,
    /// Shared state values registered with the application.
    ///
    /// Values are keyed by their [`TypeId`]. Registering additional
    /// state of the same type will replace the previous entry.
    pub app_data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl MessageRequest {
    /// Retrieve shared state of type `T` if available.
    ///
    /// Returns `None` when no value of type `T` was registered.
    #[must_use]
    pub fn state<T>(&self) -> Option<SharedState<T>>
    where
        T: Send + Sync + 'static,
    {
        self.app_data
            .get(&TypeId::of::<T>())
            .and_then(|data| data.clone().downcast::<T>().ok())
            .map(SharedState)
    }
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
    pub fn remaining(&self) -> usize { self.data.len() }
}

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

/// Shared application state accessible to handlers.
#[derive(Clone)]
pub struct SharedState<T: Send + Sync>(Arc<T>);

impl<T: Send + Sync> SharedState<T> {
    /// Construct a new [`SharedState`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use wireframe::extractor::SharedState;
    ///
    /// let data = Arc::new(5u32);
    /// let state: SharedState<u32> = Arc::clone(&data).into();
    /// assert_eq!(*state, 5);
    /// ```
    #[must_use]
    /// Creates a new `SharedState` instance wrapping the provided `Arc<T>`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use wireframe::extractor::SharedState;
    ///
    /// let state = Arc::new(42);
    /// let shared: SharedState<u32> = state.clone().into();
    /// assert_eq!(*shared, 42);
    /// ```
    #[deprecated(since = "0.2.0", note = "construct via `inner.into()` instead")]
    pub fn new(inner: Arc<T>) -> Self { Self(inner) }
}

impl<T: Send + Sync> From<Arc<T>> for SharedState<T> {
    fn from(inner: Arc<T>) -> Self { Self(inner) }
}

impl<T: Send + Sync> From<T> for SharedState<T> {
    fn from(inner: T) -> Self { Self(Arc::new(inner)) }
}

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
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingState(ty) => write!(f, "no shared state registered for {ty}"),
            Self::InvalidPayload(e) => write!(f, "failed to decode payload: {e}"),
        }
    }
}

impl std::error::Error for ExtractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPayload(e) => Some(e),
            _ => None,
        }
    }
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
    /// ```no_run
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

/// Extractor that deserializes the message payload into `T`.
#[derive(Debug, Clone)]
pub struct Message<T>(T);

impl<T> Message<T> {
    /// Consume the extractor, returning the inner message.
    #[must_use]
    pub fn into_inner(self) -> T { self.0 }
}

impl<T> std::ops::Deref for Message<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T> FromMessageRequest for Message<T>
where
    T: WireMessage,
{
    type Error = ExtractError;

    fn from_message_request(
        _req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        let (msg, consumed) = T::from_bytes(payload.data).map_err(ExtractError::InvalidPayload)?;
        payload.advance(consumed);
        Ok(Self(msg))
    }
}

/// Extractor providing peer connection metadata.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionInfo {
    peer_addr: Option<SocketAddr>,
}

impl ConnectionInfo {
    /// Returns the peer's socket address, if known.
    #[must_use]
    pub fn peer_addr(&self) -> Option<SocketAddr> { self.peer_addr }
}

impl FromMessageRequest for ConnectionInfo {
    type Error = std::convert::Infallible;

    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            peer_addr: req.peer_addr,
        })
    }
}
