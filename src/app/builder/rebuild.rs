//! Rebuilding [`WireframeApp`] when its type parameters change.
//!
//! Swapping the serializer, the codec, or the connection state type produces
//! a new builder value. These helpers centralize that reconstruction, so every
//! type-changing method names only what it actually changes.

use std::sync::Arc;

use super::WireframeApp;
use crate::{
    app::{envelope::Packet, lifecycle::ConnectionSetup, memory_budgets::MemoryBudgets},
    codec::FrameCodec,
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};

/// Groups the type-changing parameters for [`WireframeApp::rebuild_with_params`].
///
/// Consolidates serializer, codec, protocol, fragmentation, message assembler,
/// and memory budgets into a single value to keep the rebuild signature
/// concise.
pub(super) struct RebuildParams<S2, F2: FrameCodec> {
    /// Serializer to carry into the rebuilt application type.
    pub(super) serializer: Box<S2>,
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
    pub(super) message_assembler: Option<Box<dyn MessageAssembler>>,
    /// Memory limits retained across a type transition.
    pub(super) memory_budgets: Option<MemoryBudgets>,
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
    /// The `WireframeApp` builder carries 14 fields that must be moved together
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
            recovery_hook: self.recovery_hook,
            recovery_config: self.recovery_config,
        }
    }

    /// Rebuild the app after changing the connection state type parameter.
    ///
    /// A teardown hook receives the old `C` state and cannot be carried across
    /// a transition to `C2`. Register `on_connection_teardown` after
    /// `on_connection_setup` when both hooks are needed for the same state.
    pub(super) fn rebuild_with_connection_state<C2>(
        self,
        on_connect: Option<Box<ConnectionSetup<C2>>>,
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
            recovery_hook: self.recovery_hook,
            recovery_config: self.recovery_config,
        }
    }
}
