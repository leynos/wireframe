//! Extractor and request context definitions.
//!
//! [`MessageRequest`] carries connection metadata and shared application
//! state. Implement [`FromMessageRequest`] for custom extractors to parse
//! payload bytes or inspect connection info before your handler runs.

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
/// access to application state registered with [`crate::app::WireframeApp`].
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::{
    ///     app::WireframeApp,
    ///     extractor::{MessageRequest, SharedState},
    /// };
    ///
    /// let _app = WireframeApp::new().unwrap().app_data(5u32);
    /// // The framework populates the request with application data.
    /// # let mut req = MessageRequest::default();
    /// # req.insert_state(5u32);
    /// let val: Option<SharedState<u32>> = req.state();
    /// assert_eq!(*val.unwrap(), 5);
    /// ```
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

    /// Insert shared state of type `T` into the request.
    ///
    /// This replaces any existing value of the same type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::extractor::{MessageRequest, SharedState};
    ///
    /// let mut req = MessageRequest::default();
    /// req.insert_state(5u32);
    /// let val: Option<SharedState<u32>> = req.state();
    /// assert_eq!(*val.unwrap(), 5);
    /// ```
    pub fn insert_state<T>(&mut self, state: T)
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(
            TypeId::of::<T>(),
            Arc::new(state) as Arc<dyn Any + Send + Sync>,
        );
    }
}

/// Raw payload buffer handed to extractors.
///
/// Create a `Payload` from a slice using [`Payload::new`] or `into`:
///
/// ```rust
/// use wireframe::extractor::Payload;
///
/// let p1 = Payload::new(b"abc");
/// let p2: Payload<'_> = b"xyz".as_slice().into();
/// assert_eq!(p1.as_ref(), b"abc" as &[u8]);
/// assert_eq!(p2.as_ref(), b"xyz" as &[u8]);
/// ```
#[derive(Default)]
pub struct Payload<'a> {
    /// Incoming bytes not yet processed.
    data: &'a [u8],
}

impl<'a> Payload<'a> {
    /// Creates a new `Payload` from the provided byte slice.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::extractor::Payload;
    ///
    /// let payload = Payload::new(b"data");
    /// assert_eq!(payload.as_ref(), b"data" as &[u8]);
    /// ```
    #[must_use]
    #[inline]
    pub fn new(data: &'a [u8]) -> Self { Self { data } }
}

impl<'a> From<&'a [u8]> for Payload<'a> {
    #[inline]
    fn from(data: &'a [u8]) -> Self { Self { data } }
}

impl AsRef<[u8]> for Payload<'_> {
    fn as_ref(&self) -> &[u8] { self.data }
}

impl Payload<'_> {
    /// Advances the payload by `count` bytes.
    ///
    /// Consumes up to `count` bytes from the front of the slice, ensuring we
    /// never slice beyond the available buffer.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::extractor::Payload;
    ///
    /// let mut payload = Payload::new(b"abcd");
    /// payload.advance(2);
    /// assert_eq!(payload.as_ref(), b"cd" as &[u8]);
    /// ```
    pub fn advance(&mut self, count: usize) {
        let n = count.min(self.data.len());
        self.data = self.data.get(n..).unwrap_or_default();
    }

    /// Returns the number of bytes remaining.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::extractor::Payload;
    ///
    /// let mut payload = Payload::new(b"bytes");
    /// assert_eq!(payload.remaining(), 5);
    /// payload.advance(2);
    /// assert_eq!(payload.remaining(), 3);
    /// ```
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
    /// Formats the `ExtractError` for display purposes.
    ///
    /// Displays a descriptive message for missing shared state or payload decoding errors.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingState(ty) => write!(f, "no shared state registered for {ty}"),
            Self::InvalidPayload(e) => write!(f, "failed to decode payload: {e}"),
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

/// Extractor that deserializes the message payload into `T`.
#[derive(Debug, Clone)]
pub struct Message<T>(T);

impl<T> Message<T> {
    /// Consumes the extractor and returns the inner deserialised message value.
    #[must_use]
    pub fn into_inner(self) -> T { self.0 }
}

impl<T> std::ops::Deref for Message<T> {
    type Target = T;

    /// Returns a reference to the inner value.
    ///
    /// This enables transparent access to the wrapped type via dereferencing.
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T> FromMessageRequest for Message<T>
where
    T: WireMessage,
{
    type Error = ExtractError;

    /// Attempts to extract and deserialize a message of type `T` from the payload.
    ///
    /// Advances the payload by the number of bytes consumed during deserialization.
    /// Returns an error if the payload cannot be decoded into the target type.
    ///
    /// # Returns
    /// - `Ok(Self)`: The successfully extracted and deserialized message.
    /// - `Err(ExtractError::InvalidPayload)`: If deserialization fails.
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
    /// Returns the peer's socket address for the current connection, if available.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::extractor::{ConnectionInfo, FromMessageRequest, MessageRequest, Payload};
    ///
    /// let req = MessageRequest {
    ///     peer_addr: Some("127.0.0.1:8080".parse::<SocketAddr>().unwrap()),
    ///     ..Default::default()
    /// };
    /// let info = ConnectionInfo::from_message_request(&req, &mut Payload::default()).unwrap();
    /// assert_eq!(info.peer_addr(), req.peer_addr);
    /// ```
    #[must_use]
    pub fn peer_addr(&self) -> Option<SocketAddr> { self.peer_addr }
}

impl FromMessageRequest for ConnectionInfo {
    type Error = std::convert::Infallible;

    /// Extracts connection metadata from the message request.
    ///
    /// Returns a `ConnectionInfo` containing the peer's socket address, if available. This
    /// extraction is infallible.
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            peer_addr: req.peer_addr,
        })
    }
}
