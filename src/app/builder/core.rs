//! Core builder types for `WireframeApp`.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use tokio::sync::{OnceCell, mpsc};

use crate::{
    app::{
        builder_defaults::default_fragmentation,
        envelope::{Envelope, Packet},
        error::Result,
        lifecycle::{ConnectionSetup, ConnectionTeardown},
        middleware_types::{Handler, Middleware},
    },
    codec::{FrameCodec, LengthDelimitedFrameCodec},
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
    pub(in crate::app) handlers: HashMap<u32, Handler<E>>,
    pub(in crate::app) routes: OnceCell<Arc<HashMap<u32, HandlerService<E>>>>,
    pub(in crate::app) middleware: Vec<Box<dyn Middleware<E>>>,
    pub(in crate::app) serializer: S,
    pub(in crate::app) app_data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    pub(in crate::app) codec: F,
    pub(in crate::app) read_timeout_ms: u64,
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
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
    pub(super) fn rebuild_with_params<S2, F2>(
        self,
        serializer: S2,
        codec: F2,
        protocol: Option<Arc<dyn WireframeProtocol<Frame = F2::Frame, ProtocolError = ()>>>,
        fragmentation: Option<crate::fragment::FragmentationConfig>,
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
}
