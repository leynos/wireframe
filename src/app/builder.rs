//! Application builder configuring routes and middleware.
//!
//! This module defines [`WireframeApp`], an Actix-inspired builder for
//! managing connection state, routing, and middleware in a `WireframeServer`.
//! It exposes convenience methods to register handlers and lifecycle hooks.

use std::{
    any::{Any, TypeId},
    boxed::Box,
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
};

use tokio::{io, sync::mpsc};

use super::{
    envelope::{Envelope, Packet},
    error::{Result, WireframeError},
};
use crate::{
    frame::{FrameProcessor, LengthFormat, LengthPrefixedProcessor},
    hooks::{ProtocolHooks, WireframeProtocol},
    middleware::{HandlerService, Transform},
    serializer::{BincodeSerializer, Serializer},
};

type BoxedFrameProcessor =
    Box<dyn FrameProcessor<Frame = Vec<u8>, Error = io::Error> + Send + Sync>;

/// Callback invoked when a connection is established.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use wireframe::app::ConnectionSetup;
///
/// let setup: Arc<ConnectionSetup<String>> = Arc::new(|| {
///     Box::pin(async {
///         // Perform authentication and return connection state
///         String::from("hello")
///     })
/// });
/// ```
pub type ConnectionSetup<C> = dyn Fn() -> Pin<Box<dyn Future<Output = C> + Send>> + Send + Sync;

/// Callback invoked when a connection is closed.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use wireframe::app::ConnectionTeardown;
///
/// let teardown: Arc<ConnectionTeardown<String>> = Arc::new(|state| {
///     Box::pin(async move {
///         println!("Dropping {state}");
///     })
/// });
/// ```
pub type ConnectionTeardown<C> =
    dyn Fn(C) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes, services, and middleware
/// without enforcing an ordering. Methods return [`Result<Self>`] so
/// registrations can be chained ergonomically.
pub struct WireframeApp<
    S: Serializer + Send + Sync = BincodeSerializer,
    C: Send + 'static = (),
    E: Packet = Envelope,
> {
    pub(super) routes: HashMap<u32, Handler<E>>,
    pub(super) services: Vec<Handler<E>>,
    pub(super) middleware: Vec<Box<dyn Transform<HandlerService<E>, Output = HandlerService<E>>>>,
    pub(super) frame_processor: BoxedFrameProcessor,
    pub(super) serializer: S,
    pub(super) app_data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(super) on_connect: Option<Arc<ConnectionSetup<C>>>,
    pub(super) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = Vec<u8>, ProtocolError = ()>>>,
    pub(super) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
}

/// Alias for asynchronous route handlers.
///
/// A `Handler` is an `Arc` to a function returning a [`Future`], enabling
/// asynchronous execution of message handlers.
pub type Handler<E> = Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Trait representing middleware components.
pub trait Middleware<E: Packet>:
    Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

impl<E: Packet, T> Middleware<E> for T where
    T: Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

impl<S, C, E> Default for WireframeApp<S, C, E>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    ///
    /// Initializes empty routes, services, middleware, and application data.
    /// Sets the default frame processor and serializer, with no connection
    /// lifecycle hooks.
    fn default() -> Self {
        Self {
            routes: HashMap::new(),
            services: Vec::new(),
            middleware: Vec::new(),
            frame_processor: Box::new(LengthPrefixedProcessor::new(LengthFormat::default())),
            serializer: S::default(),
            app_data: HashMap::new(),
            on_connect: None,
            on_disconnect: None,
            protocol: None,
            push_dlq: None,
        }
    }
}

impl<E> WireframeApp<BincodeSerializer, (), E>
where
    E: Packet,
{
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses [`Result`] for
    /// forward compatibility.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::{Packet, PacketParts, WireframeApp};
    ///
    /// #[derive(bincode::Encode, bincode::BorrowDecode)]
    /// struct MyEnv {
    ///     id: u32,
    ///     correlation_id: Option<u64>,
    ///     data: Vec<u8>,
    /// }
    ///
    /// impl Packet for MyEnv {
    ///     fn id(&self) -> u32 { self.id }
    ///     fn correlation_id(&self) -> Option<u64> { self.correlation_id }
    ///     fn into_parts(self) -> PacketParts {
    ///         PacketParts::new(self.id, self.correlation_id, self.data)
    ///     }
    ///     fn from_parts(parts: PacketParts) -> Self {
    ///         Self {
    ///             id: parts.id(),
    ///             correlation_id: parts.correlation_id(),
    ///             data: parts.payload(),
    ///         }
    ///     }
    /// }
    ///
    /// let app = WireframeApp::<_, _, MyEnv>::new().expect("failed to create app");
    /// ```
    pub fn new() -> Result<Self> { Ok(Self::default()) }

    /// Construct a new application builder using a custom envelope type.
    ///
    /// Deprecated: call [`WireframeApp::new`] with explicit envelope type
    /// parameters.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses [`Result`] for
    /// forward compatibility.
    #[deprecated(note = "use `WireframeApp::<_, _, E>::new()` instead")]
    pub fn new_with_envelope() -> Result<Self> { Self::new() }
}

impl<S, C, E> WireframeApp<S, C, E>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Register a route that maps `id` to `handler`.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id`
    /// has already been registered.
    pub fn route(mut self, id: u32, handler: Handler<E>) -> Result<Self> {
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
    pub fn service(mut self, handler: Handler<E>) -> Result<Self> {
        self.services.push(handler);
        Ok(self)
    }

    /// Store a shared state value accessible to request extractors.
    ///
    /// The value can later be retrieved using [`crate::extractor::SharedState`]. Registering
    /// another value of the same type overwrites the previous one.
    #[must_use]
    pub fn app_data<T>(mut self, state: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(
            TypeId::of::<T>(),
            Arc::new(state) as Arc<dyn Any + Send + Sync>,
        );
        self
    }

    /// Add a middleware component to the processing pipeline.
    ///
    /// # Errors
    ///
    /// This function currently always succeeds.
    pub fn wrap<M>(mut self, mw: M) -> Result<Self>
    where
        M: Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync + 'static,
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
    pub fn on_connection_setup<F, Fut, C2>(self, f: F) -> Result<WireframeApp<S, C2, E>>
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
            app_data: self.app_data,
            on_connect: Some(Arc::new(move || Box::pin(f()))),
            on_disconnect: None,
            protocol: self.protocol,
            push_dlq: self.push_dlq,
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

    /// Install a [`WireframeProtocol`] implementation.
    ///
    /// The protocol defines hooks for connection setup, frame modification, and
    /// command completion. It is wrapped in an [`Arc`] and stored for later use
    /// by the connection actor.
    #[must_use]
    pub fn with_protocol<P>(mut self, protocol: P) -> Self
    where
        P: WireframeProtocol<Frame = Vec<u8>, ProtocolError = ()> + 'static,
    {
        self.protocol = Some(Arc::new(protocol));
        self
    }

    /// Configure a Dead Letter Queue for dropped push frames.
    ///
    /// ```rust,no_run
    /// use tokio::sync::mpsc;
    /// use wireframe::app::WireframeApp;
    ///
    /// # fn build() -> WireframeApp { WireframeApp::new().unwrap() }
    /// # fn main() {
    /// let (tx, _rx) = mpsc::channel(16);
    /// let app = build().with_push_dlq(tx);
    /// # let _ = app;
    /// # }
    /// ```
    #[must_use]
    pub fn with_push_dlq(mut self, dlq: mpsc::Sender<Vec<u8>>) -> Self {
        self.push_dlq = Some(dlq);
        self
    }

    /// Get a clone of the configured protocol, if any.
    ///
    /// Returns `None` if no protocol was installed via [`with_protocol`](Self::with_protocol).
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = Vec<u8>, ProtocolError = ()>>> {
        self.protocol.as_ref().map(Arc::clone)
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// If no protocol is installed, returns default (no-op) hooks.
    #[must_use]
    pub fn protocol_hooks(&self) -> ProtocolHooks<Vec<u8>, ()> {
        self.protocol
            .as_ref()
            .map(|p| ProtocolHooks::from_protocol(&Arc::clone(p)))
            .unwrap_or_default()
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
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeApp<Ser, C, E>
    where
        Ser: Serializer + Send + Sync,
    {
        WireframeApp {
            routes: self.routes,
            services: self.services,
            middleware: self.middleware,
            frame_processor: self.frame_processor,
            serializer,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: self.protocol,
            push_dlq: self.push_dlq,
        }
    }
}
