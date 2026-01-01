//! Application builder configuring routes and middleware.
//! [`WireframeApp`] is an Actix-inspired builder for managing connection
//! state, routing, and middleware in a `WireframeServer`. It exposes
//! convenience methods to register handlers and lifecycle hooks, and
//! serializes messages using a configurable serializer.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use tokio::sync::{OnceCell, mpsc};

use super::{
    builder_defaults::{MAX_READ_TIMEOUT_MS, MIN_READ_TIMEOUT_MS, default_fragmentation},
    envelope::{Envelope, Packet},
    error::{Result, WireframeError},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    middleware_types::{Handler, Middleware},
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec, clamp_frame_length},
    fragment::FragmentationConfig,
    hooks::{ProtocolHooks, WireframeProtocol},
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

/// Fields preserved across type-transitioning builder methods.
///
/// Used by [`WireframeApp::with_codec`] and [`WireframeApp::serializer`] to
/// avoid duplicating the struct reconstruction logic.
struct PreservedFields<C, E>
where
    C: Send + 'static,
    E: Packet,
{
    handlers: HashMap<u32, Handler<E>>,
    middleware: Vec<Box<dyn Middleware<E>>>,
    app_data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    on_connect: Option<Arc<ConnectionSetup<C>>>,
    on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    read_timeout_ms: u64,
}

impl<C, E> PreservedFields<C, E>
where
    C: Send + 'static,
    E: Packet,
{
    /// Reconstruct a [`WireframeApp`] from preserved fields plus new components.
    #[expect(
        clippy::too_many_arguments,
        reason = "internal helper grouping fields for type-transitioning builders"
    )]
    fn into_app<S, F>(
        self,
        serializer: S,
        codec: F,
        protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
        fragmentation: Option<FragmentationConfig>,
    ) -> WireframeApp<S, C, E, F>
    where
        S: Serializer + Send + Sync,
        F: FrameCodec,
    {
        WireframeApp {
            handlers: self.handlers,
            routes: OnceCell::new(),
            middleware: self.middleware,
            serializer,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol,
            push_dlq: self.push_dlq,
            codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation,
        }
    }
}

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes and middleware without enforcing an
/// ordering. Methods return [`Result<Self>`] so registrations can be chained
/// ergonomically.
pub struct WireframeApp<
    S: Serializer + Send + Sync = BincodeSerializer,
    C: Send + 'static = (),
    E: Packet = Envelope,
    F: FrameCodec = LengthDelimitedFrameCodec,
> {
    pub(super) handlers: HashMap<u32, Handler<E>>,
    pub(super) routes: OnceCell<Arc<HashMap<u32, HandlerService<E>>>>,
    pub(super) middleware: Vec<Box<dyn Middleware<E>>>,
    pub(super) serializer: S,
    pub(super) app_data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(super) on_connect: Option<Arc<ConnectionSetup<C>>>,
    pub(super) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    pub(super) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    pub(super) codec: F,
    pub(super) read_timeout_ms: u64,
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
}

impl<S, C, E, F> Default for WireframeApp<S, C, E, F>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec + Default,
{
    /// Initializes empty routes, middleware, and application data with the
    /// default serializer and no lifecycle hooks.
    fn default() -> Self {
        let codec = F::default();
        let max_frame_length = codec.max_frame_length();
        Self {
            handlers: HashMap::new(),
            routes: OnceCell::new(),
            middleware: Vec::new(),
            serializer: S::default(),
            app_data: HashMap::new(),
            on_connect: None,
            on_disconnect: None,
            protocol: None,
            push_dlq: None,
            codec,
            read_timeout_ms: 100,
            fragmentation: default_fragmentation(max_frame_length),
        }
    }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec + Default,
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
    /// use wireframe::app::WireframeApp;
    /// WireframeApp::<_, _, wireframe::app::Envelope>::new().expect("failed to initialize app");
    /// ```
    pub fn new() -> Result<Self> { Ok(Self::default()) }

    /// Construct a new application builder using the provided serializer.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses [`Result`] for
    /// forward compatibility.
    pub fn with_serializer(serializer: S) -> Result<Self> {
        Ok(Self {
            serializer,
            ..Self::default()
        })
    }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Extract fields preserved across type-transitioning builder methods.
    fn into_preserved(self) -> PreservedFields<C, E> {
        PreservedFields {
            handlers: self.handlers,
            middleware: self.middleware,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            push_dlq: self.push_dlq,
            read_timeout_ms: self.read_timeout_ms,
        }
    }

    /// Replace the frame codec used for framing I/O.
    ///
    /// This resets any installed protocol hooks because the frame type may
    /// change across codecs. Fragmentation configuration is reset to the
    /// codec-derived default.
    #[must_use]
    pub fn with_codec<F2: FrameCodec>(mut self, codec: F2) -> WireframeApp<S, C, E, F2>
    where
        S: Default,
    {
        let fragmentation = default_fragmentation(codec.max_frame_length());
        let serializer = std::mem::take(&mut self.serializer);
        self.into_preserved()
            .into_app(serializer, codec, None, fragmentation)
    }

    /// Register a route that maps `id` to `handler`.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id`
    /// has already been registered.
    pub fn route(mut self, id: u32, handler: Handler<E>) -> Result<Self> {
        if self.handlers.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.handlers.insert(id, handler);
        self.routes = OnceCell::new();
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
        M: Middleware<E> + 'static,
    {
        self.middleware.push(Box::new(mw));
        self.routes = OnceCell::new();
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
    pub fn on_connection_setup<SetupFn, Fut, C2>(
        self,
        f: SetupFn,
    ) -> Result<WireframeApp<S, C2, E, F>>
    where
        SetupFn: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = C2> + Send + 'static,
        C2: Send + 'static,
    {
        Ok(WireframeApp {
            handlers: self.handlers,
            routes: OnceCell::new(),
            middleware: self.middleware,
            serializer: self.serializer,
            app_data: self.app_data,
            on_connect: Some(Arc::new(move || Box::pin(f()))),
            on_disconnect: None,
            protocol: self.protocol,
            push_dlq: self.push_dlq,
            codec: self.codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation: self.fragmentation,
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
    pub fn on_connection_teardown<TeardownFn, Fut>(mut self, f: TeardownFn) -> Result<Self>
    where
        TeardownFn: Fn(C) -> Fut + Send + Sync + 'static,
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
    pub fn with_protocol<P>(self, protocol: P) -> Self
    where
        P: WireframeProtocol<Frame = F::Frame, ProtocolError = ()> + 'static,
    {
        WireframeApp {
            protocol: Some(Arc::new(protocol)),
            ..self
        }
    }

    /// Configure a Dead Letter Queue for dropped push frames.
    ///
    /// ```rust,no_run
    /// use tokio::sync::mpsc;
    /// use wireframe::app::WireframeApp;
    ///
    /// # fn build() -> WireframeApp {
    /// #     WireframeApp::new().expect("builder creation should not fail")
    /// # }
    /// # fn main() {
    /// let (tx, _rx) = mpsc::channel(16);
    /// let app = build().with_push_dlq(tx);
    /// # let _ = app;
    /// # }
    /// ```
    #[must_use]
    pub fn with_push_dlq(self, dlq: mpsc::Sender<Vec<u8>>) -> Self {
        WireframeApp {
            push_dlq: Some(dlq),
            ..self
        }
    }

    /// Get a clone of the configured protocol, if any.
    ///
    /// Returns `None` if no protocol was installed via [`with_protocol`](Self::with_protocol).
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>> {
        self.protocol.clone()
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// If no protocol is installed, returns default (no-op) hooks.
    #[must_use]
    pub fn protocol_hooks(&self) -> ProtocolHooks<F::Frame, ()> {
        self.protocol
            .as_ref()
            .map(ProtocolHooks::from_protocol)
            .unwrap_or_default()
    }

    /// Replace the serializer used for messages.
    #[must_use]
    pub fn serializer<Ser>(mut self, serializer: Ser) -> WireframeApp<Ser, C, E, F>
    where
        Ser: Serializer + Send + Sync,
        F: Default,
    {
        let codec = std::mem::take(&mut self.codec);
        let protocol = self.protocol.take();
        let fragmentation = self.fragmentation.take();
        self.into_preserved()
            .into_app(serializer, codec, protocol, fragmentation)
    }

    /// Configure the read timeout in milliseconds.
    /// Clamped between 1 and 86 400 000 milliseconds (24 h).
    #[must_use]
    pub fn read_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.read_timeout_ms = timeout_ms.clamp(MIN_READ_TIMEOUT_MS, MAX_READ_TIMEOUT_MS);
        self
    }

    /// Override the fragmentation configuration.
    ///
    /// Provide `None` to disable fragmentation entirely.
    #[must_use]
    pub fn fragmentation(mut self, config: Option<FragmentationConfig>) -> Self {
        self.fragmentation = config;
        self
    }
}

impl<S, C, E> WireframeApp<S, C, E, LengthDelimitedFrameCodec>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Set the initial buffer capacity for framed reads.
    /// Clamped between 64 bytes and 16 MiB.
    #[must_use]
    pub fn buffer_capacity(mut self, capacity: usize) -> Self {
        let capacity = clamp_frame_length(capacity);
        self.codec = LengthDelimitedFrameCodec::new(capacity);
        self.fragmentation = default_fragmentation(capacity);
        self
    }
}
