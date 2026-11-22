//! Core builder types for `WireframeApp`.

use std::{collections::HashMap, num::NonZeroUsize, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    app::{
        Route,
        builder_defaults::DEFAULT_READ_TIMEOUT_MS,
        envelope::{Envelope, Packet},
        error::Result,
        lifecycle::{ConnectionSetup, ConnectionTeardown},
        memory_budgets::MemoryBudgets,
        middleware_types::Middleware,
    },
    codec::{FrameCodec, LengthDelimitedFrameCodec},
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
    /// Serializer retained by every connection built from this application.
    ///
    /// The handle is shared because the connection actor's protocol hooks are
    /// `'static` and need to decode terminator frames without copying the
    /// serializer.
    pub(in crate::app) serializer: Arc<S>,
    /// Optional asynchronous hook that creates per-connection state.
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    /// Optional asynchronous hook that releases state after processing once
    /// setup has produced it. It runs for clean completion, returned
    /// stream-processing errors, a caught handler panic, and cooperative
    /// cancellation observed at a read boundary; see
    /// [`ConnectionTeardown`](crate::app::ConnectionTeardown) for the gaps.
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
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
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
    /// Optional cap on concurrently in-flight message assemblies.
    ///
    /// `None` means the application has not overridden the derived default of
    /// `DEFAULT_MAX_CONCURRENT_ASSEMBLIES`.
    pub(in crate::app) max_concurrent_assemblies: Option<NonZeroUsize>,
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
            middleware: Vec::new(),
            serializer: Arc::new(S::default()),
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
            max_concurrent_assemblies: None,
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
            serializer: Arc::new(serializer),
            ..Self::default()
        })
    }
}

/// Groups the type-changing parameters for [`WireframeApp::rebuild_with_params`].
///
/// Consolidates serializer, codec, protocol, fragmentation, message assembler,
/// memory budgets, and the concurrent-assembly cap into a single value to keep
/// the rebuild signature concise.
pub(super) struct RebuildParams<S2, F2: FrameCodec> {
    /// Serializer to carry into the rebuilt application type.
    pub(super) serializer: Arc<S2>,
    /// Codec to carry into the rebuilt application type.
    pub(super) codec: F2,
    /// Protocol hook whose frame type matches the replacement codec.
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F2::Frame, ProtocolError = ()>>>,
    /// Push-queue template matching the replacement codec's frame type.
    pub(super) push_queues: Option<PushQueuesBuilder<F2::Frame>>,
    /// Registry matching the replacement codec's frame type.
    pub(super) session_registry: Arc<SessionRegistry<F2::Frame>>,
    /// Fragmentation settings retained across a type transition.
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Message assembler retained across a type transition.
    pub(super) message_assembler: Option<Arc<dyn MessageAssembler>>,
    /// Memory limits retained across a type transition.
    pub(super) memory_budgets: Option<MemoryBudgets>,
    /// Concurrent-assembly cap retained across a type transition.
    pub(super) max_concurrent_assemblies: Option<NonZeroUsize>,
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
    /// The `WireframeApp` builder carries 12 fields that must be moved together
    /// when swapping serializer or codec types. Centralizing the reconstruction
    /// here keeps the transitions consistent and avoids repeating the same
    /// field list across each type-changing method. For smaller builders with
    /// only a handful of fields and single-field updates, prefer the macro-based
    /// pattern used by `WireframeClientBuilder`.
    pub(super) fn rebuild_with_params<S2, F2>(
        self,
        params: RebuildParams<S2, F2>,
    ) -> WireframeApp<S2, C, E, F2>
    where
        S2: Serializer + Send + Sync,
        F2: FrameCodec,
    {
        WireframeApp {
            handlers: self.handlers,
            middleware: self.middleware,
            serializer: params.serializer,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: params.protocol,
            push_dlq: self.push_dlq,
            push_queues: params.push_queues,
            session_registry: params.session_registry,
            codec: params.codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation: params.fragmentation,
            message_assembler: params.message_assembler,
            memory_budgets: params.memory_budgets,
            max_concurrent_assemblies: params.max_concurrent_assemblies,
        }
    }

    /// Rebuild the app after changing the connection state type parameter.
    ///
    /// A teardown hook receives the old `C` state and cannot be carried across
    /// a transition to `C2`. Register `on_connection_teardown` after
    /// `on_connection_setup` when both hooks are needed for the same state.
    pub(super) fn rebuild_with_connection_state<C2>(
        self,
        on_connect: Option<Arc<ConnectionSetup<C2>>>,
    ) -> WireframeApp<S, C2, E, F>
    where
        C2: Send + 'static,
    {
        WireframeApp {
            handlers: self.handlers,
            middleware: self.middleware,
            serializer: self.serializer,
            on_connect,
            on_disconnect: None,
            protocol: self.protocol,
            push_dlq: self.push_dlq,
            push_queues: self.push_queues,
            session_registry: self.session_registry,
            codec: self.codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation: self.fragmentation,
            message_assembler: self.message_assembler,
            memory_budgets: self.memory_budgets,
            max_concurrent_assemblies: self.max_concurrent_assemblies,
        }
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
