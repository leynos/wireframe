//! Application builder configuring routes and middleware.
//!
//! `WireframeApp` stores registered routes, services, and middleware
//! for a [`WireframeServer`]. Methods return [`Result<Self>`] so callers
//! can chain registrations ergonomically.

use std::{boxed::Box, collections::HashMap, future::Future, pin::Pin};

use bytes::BytesMut;
use tokio::io::{self, AsyncWrite, AsyncWriteExt};

use crate::{
    config::SerializationFormat,
    frame::{FrameProcessor, LengthPrefixedProcessor},
    message::Message,
};

type BoxedFrameProcessor =
    Box<dyn FrameProcessor<Frame = Vec<u8>, Error = io::Error> + Send + Sync>;

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes, services, and middleware
/// without enforcing an ordering. Methods return [`Result<Self>`] so
/// registrations can be chained ergonomically.
pub struct WireframeApp {
    routes: HashMap<u32, Service>,
    services: Vec<Service>,
    middleware: Vec<Box<dyn Middleware>>,
    frame_processor: BoxedFrameProcessor,
    serializer: SerializationFormat,
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

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;

impl Default for WireframeApp {
    fn default() -> Self {
        Self {
            routes: HashMap::new(),
            services: Vec::new(),
            middleware: Vec::new(),
            frame_processor: Box::new(LengthPrefixedProcessor),
            serializer: SerializationFormat::Bincode,
        }
    }
}

impl WireframeApp {
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses the
    /// [`Result`] type for forward compatibility.
    pub fn new() -> Result<Self> { Ok(Self::default()) }

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

    /// Set the frame processor used for encoding and decoding frames.
    ///
    /// # Errors
    ///
    /// Currently never returns an error but retains `Result` for future
    /// configurability.
    pub fn frame_processor<P>(mut self, processor: P) -> Result<Self>
    where
        P: FrameProcessor<Frame = Vec<u8>, Error = io::Error> + Send + Sync + 'static,
    {
        self.frame_processor = Box::new(processor);
        Ok(self)
    }

    /// Choose the serialization format for messages.
    ///
    /// # Errors
    ///
    /// This function currently never fails.
    pub fn serialization_format(mut self, format: SerializationFormat) -> Result<Self> {
        self.serializer = format;
        Ok(self)
    }

    /// Serialize `msg` and write it to `stream` using the frame processor.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if serialization or writing fails.
    pub async fn send_response<S, M>(&mut self, stream: &mut S, msg: &M) -> io::Result<()>
    where
        S: AsyncWrite + Unpin,
        M: Message,
    {
        let bytes = self.serializer.serialize(msg).map_err(io::Error::other)?;
        let mut framed = BytesMut::new();
        self.frame_processor.encode(&bytes, &mut framed).await?;
        stream.write_all(&framed).await?;
        stream.flush().await
    }

    /// Handle an accepted connection.
    ///
    /// This placeholder immediately closes the connection after the
    /// preamble phase. A warning is logged so tests and callers are
    /// aware of the current limitation.
    pub async fn handle_connection<S>(&self, _stream: S)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        log::warn!(
            "`WireframeApp::handle_connection` called, but connection handling is not \
             implemented; closing stream"
        );
        tokio::task::yield_now().await;
    }
}
