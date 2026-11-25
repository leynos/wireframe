//! Core builder types for `WireframeApp`.
//!
//! [`WireframeApp`] holds the routes, streaming handlers, middleware and
//! configuration the builder modules populate; see [`super::routing`].

use std::{collections::HashMap, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    app::{
        Route,
        builder_defaults::DEFAULT_READ_TIMEOUT_MS,
        envelope::{Envelope, Packet},
        error::Result,
        lifecycle::{ConnectionSetup, ConnectionTeardown},
        memory_budgets::MemoryBudgets,
        middleware_types::{Middleware, StreamingHandler},
    },
    codec::{FrameCodec, LengthDelimitedFrameCodec, RecoveryConfig, RecoveryPolicyHook},
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    push::PushQueuesBuilder,
    serializer::{BincodeSerializer, Serializer},
    session::SessionRegistry,
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
    /// Registered routes keyed by the protocol message identifier.
    pub(in crate::app) handlers: HashMap<u32, Route<E>>,
    /// Middleware applied in registration order around each handler.
    pub(in crate::app) middleware: Vec<Box<dyn Middleware<E>>>,
    /// Streaming handlers keyed by the protocol message identifier.
    pub(in crate::app) streaming_handlers: HashMap<u32, StreamingHandler>,
    /// Serializer retained by every connection built from this application.
    ///
    /// The builder owns the serializer outright. Preparation moves it into
    /// [`PreparedApp`](crate::app::PreparedApp), whose shared root keeps it
    /// alive, so a connection borrows `&S` instead of carrying its own
    /// reference count.
    pub(in crate::app) serializer: Box<S>,
    /// Optional asynchronous hook that creates per-connection state.
    ///
    /// Owned directly rather than through an `Arc`: the prepared root already
    /// keeps the callback definition alive for the connection that borrows it.
    pub(in crate::app) on_connect: Option<Box<ConnectionSetup<C>>>,
    /// Optional asynchronous hook that releases state after processing once
    /// setup has produced it. It runs for clean completion, returned
    /// stream-processing errors, a caught handler panic, and cooperative
    /// cancellation observed at a read boundary; see
    /// [`ConnectionTeardown`](crate::app::ConnectionTeardown) for the gaps.
    pub(in crate::app) on_disconnect: Option<Box<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Legacy dead-letter sink for pushes that cannot be delivered.
    ///
    /// Superseded by [`Self::push_queues`], whose dead-letter queue carries
    /// codec frames rather than serialized payloads (ADR 009).
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    /// Template cloned to build each connection's push queues.
    ///
    /// `None` means every connection receives the default queue
    /// configuration. A stored template is validated during preparation so a
    /// misconfigured application fails before accepting connections.
    pub(in crate::app) push_queues: Option<PushQueuesBuilder<F::Frame>>,
    /// Registry shared by every connection built from this application.
    pub(in crate::app) session_registry: Arc<SessionRegistry<F::Frame>>,
    /// Frame codec used by all connections from this application.
    pub(in crate::app) codec: F,
    /// Maximum interval to wait for the next inbound frame.
    pub(in crate::app) read_timeout_ms: u64,
    /// Optional limits and timeout for transparent frame fragmentation.
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Optional assembler for protocol messages spread across several frames.
    ///
    /// Boxed and owned by the application; a connection borrows
    /// `&dyn MessageAssembler` and keeps only its own assembly state.
    pub(in crate::app) message_assembler: Option<Box<dyn MessageAssembler>>,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
    /// Optional hook consulted when a decoder fails on a live connection.
    pub(in crate::app) recovery_hook: Option<Arc<dyn RecoveryPolicyHook>>,
    /// Global recovery settings such as the consecutive-drop limit.
    pub(in crate::app) recovery_config: RecoveryConfig,
}

impl<S, C, E, F> Default for WireframeApp<S, C, E, F>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec + Default,
{
    /// Initializes empty handlers and middleware with the default serializer
    /// and no lifecycle hooks.
    fn default() -> Self {
        let codec = F::default();
        Self {
            handlers: HashMap::new(),
            streaming_handlers: HashMap::new(),
            middleware: Vec::new(),
            serializer: Box::new(S::default()),
            on_connect: None,
            on_disconnect: None,
            protocol: None,
            push_dlq: None,
            push_queues: None,
            session_registry: Arc::new(SessionRegistry::new()),
            codec,
            read_timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            fragmentation: None,
            message_assembler: None,
            memory_budgets: None,
            recovery_hook: None,
            recovery_config: RecoveryConfig::default(),
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
    ///
    /// let _app: WireframeApp = WireframeApp::new().expect("failed to initialize app");
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
            serializer: Box::new(serializer),
            ..Self::default()
        })
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
