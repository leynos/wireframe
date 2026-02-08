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
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

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
    pub(super) message_assembler: Option<Arc<dyn MessageAssembler>>,
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
            message_assembler: None,
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
    /// Helper to rebuild the app when changing type parameters.
    ///
    /// This centralises the field-by-field reconstruction required when
    /// transforming between different serializer or codec types.
    #[expect(
        clippy::too_many_arguments,
        reason = "internal helper grouping fields for type-transitioning builders"
    )]
    fn rebuild_with_params<S2, F2>(
        self,
        serializer: S2,
        codec: F2,
        protocol: Option<Arc<dyn WireframeProtocol<Frame = F2::Frame, ProtocolError = ()>>>,
        fragmentation: Option<FragmentationConfig>,
        message_assembler: Option<Arc<dyn MessageAssembler>>,
    ) -> WireframeApp<S2, C, E, F2>
    where
        S2: Serializer + Send + Sync,
        F2: FrameCodec,
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
            message_assembler,
        }
    }

    /// Helper to rebuild the app when changing the connection state type.
    pub(super) fn rebuild_with_connection_type<C2>(
        self,
        on_connect: Option<Arc<ConnectionSetup<C2>>>,
        on_disconnect: Option<Arc<ConnectionTeardown<C2>>>,
    ) -> WireframeApp<S, C2, E, F>
    where
        C2: Send + 'static,
    {
        WireframeApp {
            handlers: self.handlers,
            routes: OnceCell::new(),
            middleware: self.middleware,
            serializer: self.serializer,
            app_data: self.app_data,
            on_connect,
            on_disconnect,
            protocol: self.protocol,
            push_dlq: self.push_dlq,
            codec: self.codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation: self.fragmentation,
            message_assembler: self.message_assembler,
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
        let message_assembler = self.message_assembler.take();
        self.rebuild_with_params(serializer, codec, None, fragmentation, message_assembler)
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
        let message_assembler = self.message_assembler.take();
        self.rebuild_with_params(
            serializer,
            codec,
            protocol,
            fragmentation,
            message_assembler,
        )
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
