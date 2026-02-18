//! Request context and payload buffer types for extractors.

use std::{net::SocketAddr, sync::Mutex};

use super::SharedState;
use crate::{app_data_store::AppDataStore, request::RequestBodyStream};

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
    /// Values are keyed by their concrete type. Registering additional
    /// state of the same type will replace the previous entry.
    pub(crate) app_data: AppDataStore,
    /// Optional streaming body for handlers that opt into streaming consumption.
    ///
    /// When present, the [`StreamingBody`](crate::extractor::StreamingBody)
    /// extractor can take ownership of this stream. Only one handler/extractor
    /// may consume the stream; subsequent extractions will receive
    /// [`ExtractError::MissingBodyStream`].
    body_stream: Option<Mutex<Option<RequestBodyStream>>>,
}

impl MessageRequest {
    /// Create a new empty message request.
    ///
    /// Use [`Self::with_peer_addr`] to configure connection metadata.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Set the peer address for this request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::extractor::MessageRequest;
    ///
    /// let req = MessageRequest::new().with_peer_addr(Some(
    ///     "127.0.0.1:8080".parse().expect("valid socket address"),
    /// ));
    /// assert!(req.peer_addr.is_some());
    /// ```
    #[must_use]
    pub fn with_peer_addr(mut self, addr: Option<SocketAddr>) -> Self {
        self.peer_addr = addr;
        self
    }

    /// Retrieve shared state of type `T` if available.
    ///
    /// Returns `None` when no value of type `T` was registered.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     app::WireframeApp,
    ///     extractor::{MessageRequest, SharedState},
    /// };
    ///
    /// let _app: WireframeApp = WireframeApp::new()
    ///     .expect("failed to initialize app")
    ///     .app_data(5u32);
    /// // The framework populates the request with application data.
    /// # let req = MessageRequest::default();
    /// # req.insert_state(5u32);
    /// let val: Option<SharedState<u32>> = req.state();
    /// assert_eq!(*val.expect("shared state missing"), 5);
    /// ```
    #[must_use]
    pub fn state<T>(&self) -> Option<SharedState<T>>
    where
        T: Send + Sync + 'static,
    {
        self.app_data.get::<T>().map(SharedState::from)
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
    /// let req = MessageRequest::default();
    /// req.insert_state(5u32);
    /// let val: Option<SharedState<u32>> = req.state();
    /// assert_eq!(*val.expect("shared state missing"), 5);
    /// ```
    pub fn insert_state<T>(&self, state: T)
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(state);
    }

    /// Set the streaming body for this request.
    ///
    /// The framework calls this when a handler opts into streaming consumption.
    /// The stream can later be taken by the
    /// [`StreamingBody`](crate::extractor::StreamingBody) extractor.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{extractor::MessageRequest, request::body_channel};
    ///
    /// let mut req = MessageRequest::default();
    /// let (_tx, stream) = body_channel(4);
    /// req.set_body_stream(stream);
    /// ```
    pub fn set_body_stream(&mut self, stream: RequestBodyStream) {
        self.body_stream = Some(Mutex::new(Some(stream)));
    }

    /// Take the streaming body from this request, if present.
    ///
    /// Returns `None` if no body stream was set or if it was already taken
    /// by a previous extractor. This ensures only one consumer receives the
    /// stream.
    ///
    /// # Mutex poisoning
    ///
    /// If the internal mutex is poisoned (for example, due to a panic in another
    /// thread while holding the lock), this method returns `None` rather than
    /// propagating the panic. This behaviour prioritizes availability over
    /// strict correctness: a poisoned mutex typically indicates a serious bug
    /// elsewhere, but crashing additional handlers would only compound the
    /// problem. The missing stream will surface as an
    /// [`ExtractError::MissingBodyStream`](crate::extractor::ExtractError::MissingBodyStream)
    /// in the handler, which can be logged and investigated.
    #[must_use]
    pub fn take_body_stream(&self) -> Option<RequestBodyStream> {
        self.body_stream
            .as_ref()
            .and_then(|mutex| mutex.lock().ok())
            .and_then(|mut guard| guard.take())
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
    pub(super) data: &'a [u8],
}

impl<'a> Payload<'a> {
    /// Creates a new `Payload` from the provided byte slice.
    ///
    /// # Examples
    ///
    /// ```rust
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
    /// ```rust
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
    /// ```rust
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
