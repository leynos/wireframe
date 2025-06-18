//! Application builder configuring routes and middleware.
//!
//! `WireframeApp` stores registered routes, services, and middleware
//! for a [`WireframeServer`]. Most builder methods return [`Result<Self>`]
//! so callers can chain registrations ergonomically.

use std::{boxed::Box, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use bytes::BytesMut;
use tokio::io::{self, AsyncWrite, AsyncWriteExt};

use crate::{
    frame::{FrameProcessor, LengthPrefixedProcessor},
    message::Message,
    serializer::{BincodeSerializer, Serializer},
};

type BoxedFrameProcessor =
    Box<dyn FrameProcessor<Frame = Vec<u8>, Error = io::Error> + Send + Sync>;

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes, services, and middleware
/// without enforcing an ordering. Methods return [`Result<Self>`] so
/// registrations can be chained ergonomically.
#[allow(clippy::type_complexity)]
pub struct WireframeApp<S: Serializer = BincodeSerializer, C: Send + 'static = ()> {
    routes: HashMap<u32, Service>,
    services: Vec<Service>,
    middleware: Vec<Box<dyn Middleware>>,
    frame_processor: BoxedFrameProcessor,
    serializer: S,
    on_connect: Option<Arc<dyn Fn() -> Pin<Box<dyn Future<Output = C> + Send>> + Send + Sync>>,
    on_disconnect: Option<Arc<dyn Fn(C) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>>,
}

/// Alias for boxed asynchronous handlers.
///
/// A `Service` is a boxed function returning a [`Future`], enabling
/// asynchronous execution of message handlers.
pub type Service = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Trait representing middleware components.
pub trait Middleware: Send + Sync {}

/// Top-level error type for application setup.
#[derive(Debug)]
pub enum WireframeError {
    /// A route with the provided identifier was already registered.
    DuplicateRoute(u32),
}

/// Errors produced when sending a handler response over a stream.
#[derive(Debug)]
pub enum SendError {
    /// Serialization failed.
    Serialize(Box<dyn std::error::Error + Send + Sync>),
    /// Writing to the stream failed.
    Io(io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Serialize(e) => write!(f, "serialization error: {e}"),
            SendError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::Serialize(e) => Some(&**e),
            SendError::Io(e) => Some(e),
        }
    }
}

impl From<io::Error> for SendError {
    fn from(e: io::Error) -> Self { SendError::Io(e) }
}

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;

impl<S, C> Default for WireframeApp<S, C>
where
    S: Serializer + Default,
    C: Send + 'static,
{
    fn default() -> Self {
        Self {
            routes: HashMap::new(),
            services: Vec::new(),
            middleware: Vec::new(),
            frame_processor: Box::new(LengthPrefixedProcessor),
            serializer: S::default(),
            on_connect: None,
            on_disconnect: None,
        }
    }
}

impl WireframeApp<BincodeSerializer, ()> {
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses the
    /// [`Result`] type for forward compatibility.
    pub fn new() -> Result<Self> { Ok(Self::default()) }
}

impl<S, C> WireframeApp<S, C>
where
    S: Serializer,
    C: Send + 'static,
{
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses the
    /// [`Result`] type for forward compatibility.
    ///
    /// Register a route that maps `id` to `handler`.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id`
    /// has already been registered.
    pub fn route(mut self, id: u32, handler: Service) -> Result<Self> {
        if self.routes.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.routes.insert(id, handler);
        Ok(self)
    }

    /// Register a handler discovered by attribute macros or other means.
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn service(mut self, handler: Service) -> Result<Self> {
        self.services.push(handler);
        Ok(self)
    }

    /// Add a middleware component to the processing pipeline.
    ///
    /// # Errors
    ///
    /// This function currently always succeeds.
    pub fn wrap<M>(mut self, mw: M) -> Result<Self>
    where
        M: Middleware + 'static,
    {
        self.middleware.push(Box::new(mw));
        Ok(self)
    }

    /// Register a callback invoked when a new connection is established.
    ///
    /// The callback can perform authentication or other setup tasks and
    /// returns connection-specific state stored for the connection's
    /// lifetime.
    ///
    /// # Type Parameters
    ///
    /// This method changes the connection state type parameter from `C` to `C2`.
    /// This means that any subsequent builder methods will operate on the new connection state type
    /// `C2`. Be aware of this type transition when chaining builder methods.
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn on_connection_setup<F, Fut, C2>(self, f: F) -> Result<WireframeApp<S, C2>>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = C2> + Send + 'static,
        C2: Send + 'static,
    {
        Ok(WireframeApp {
            routes: self.routes,
            services: self.services,
            middleware: self.middleware,
            frame_processor: self.frame_processor,
            serializer: self.serializer,
            on_connect: Some(Arc::new(move || Box::pin(f()))),
            on_disconnect: None,
        })
    }

    /// Register a callback invoked when a connection is closed.
    ///
    /// The callback receives the connection state produced by
    /// [`on_connection_setup`](Self::on_connection_setup).
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn on_connection_teardown<F, Fut>(mut self, f: F) -> Result<Self>
    where
        F: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.on_disconnect = Some(Arc::new(move |c| Box::pin(f(c))));
        Ok(self)
    }

    /// Set the frame processor used for encoding and decoding frames.
    #[must_use]
    pub fn frame_processor<P>(mut self, processor: P) -> Self
    where
        P: FrameProcessor<Frame = Vec<u8>, Error = io::Error> + Send + Sync + 'static,
    {
        self.frame_processor = Box::new(processor);
        self
    }

    /// Replace the serializer used for messages.
    #[must_use]
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeApp<Ser, C>
    where
        Ser: Serializer,
    {
        WireframeApp {
            routes: self.routes,
            services: self.services,
            middleware: self.middleware,
            frame_processor: self.frame_processor,
            serializer,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
        }
    }

    /// Serialize `msg` and write it to `stream` using the frame processor.
    ///
    /// # Errors
    ///
    /// Returns a [`SendError`] if serialization or writing fails.
    pub async fn send_response<W, M>(
        &self,
        stream: &mut W,
        msg: &M,
    ) -> std::result::Result<(), SendError>
    where
        W: AsyncWrite + Unpin,
        M: Message,
    {
        let bytes = self
            .serializer
            .serialize(msg)
            .map_err(SendError::Serialize)?;
        let mut framed = BytesMut::with_capacity(4 + bytes.len());
        self.frame_processor
            .encode(&bytes, &mut framed)
            .map_err(SendError::Io)?;
        stream.write_all(&framed).await.map_err(SendError::Io)?;
        stream.flush().await.map_err(SendError::Io)
    }

    /// Handle an accepted connection.
    ///
    /// This placeholder immediately closes the connection after the
    /// preamble phase. A warning is logged so tests and callers are
    /// aware of the current limitation.
    pub async fn handle_connection<W>(&self, _stream: W)
    where
        W: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        let state = if let Some(setup) = &self.on_connect {
            Some((setup)().await)
        } else {
            None
        };

        log::warn!(
            "`WireframeApp::handle_connection` called, but connection handling is not \
             implemented; closing stream"
        );
        tokio::task::yield_now().await;

        if let (Some(teardown), Some(state)) = (&self.on_disconnect, state) {
            teardown(state).await;
        }
    }
}
