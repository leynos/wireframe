//! Codec and serializer configuration for `WireframeApp`.

use super::{WireframeApp, rebuild::RebuildParams};
use crate::{
    app::Packet,
    codec::{FrameCodec, LengthDelimitedFrameCodec, clamp_frame_length},
    serializer::Serializer,
    session::SessionRegistry,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Return a reference to the configured frame codec.
    pub const fn codec(&self) -> &F { &self.codec }

    /// Replace the frame codec used for framing I/O.
    ///
    /// This resets any installed protocol hooks, push-queue template, and
    /// session registry because the frame type may change across codecs.
    /// Fragmentation is disabled so callers can reconfigure explicitly for the
    /// new frame budget.
    #[must_use]
    pub fn with_codec<F2: FrameCodec>(mut self, codec: F2) -> WireframeApp<S, C, E, F2> {
        let message_assembler = self.message_assembler.take();
        let memory_budgets = self.memory_budgets.take();
        // The serializer is owned directly, so it moves out of `self` rather
        // than being cloned. Destructuring keeps the remaining fields
        // constructible while the old codec, protocol, and fragmentation
        // configuration are deliberately dropped.
        let Self {
            handlers,
            streaming_handlers,
            middleware,
            serializer,
            on_connect,
            on_disconnect,
            push_dlq,
            read_timeout_ms,
            recovery_hook,
            recovery_config,
            ..
        } = self;
        WireframeApp {
            handlers,
            // A streaming handler takes `RequestParts` and a body stream and
            // returns bytes, so it names no frame type and survives a codec
            // change. Resetting it would silently unregister routes the
            // caller registered before this call.
            streaming_handlers,
            middleware,
            serializer,
            on_connect,
            on_disconnect,
            protocol: None,
            push_dlq,
            push_queues: None,
            session_registry: std::sync::Arc::new(SessionRegistry::new()),
            codec,
            read_timeout_ms,
            fragmentation: None,
            message_assembler,
            memory_budgets,
            recovery_hook,
            recovery_config,
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
        let push_queues = self.push_queues.take();
        let session_registry = std::sync::Arc::clone(&self.session_registry);
        let fragmentation = self.fragmentation.take();
        let message_assembler = self.message_assembler.take();
        let memory_budgets = self.memory_budgets.take();
        self.rebuild_with_params(RebuildParams {
            serializer: Box::new(serializer),
            codec,
            protocol,
            push_queues,
            session_registry,
            fragmentation,
            message_assembler,
            memory_budgets,
        })
    }
}

impl<S, C, E> WireframeApp<S, C, E, LengthDelimitedFrameCodec>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Set the initial buffer capacity for framed reads.
    /// Clamped between 64 bytes and 16 MiB.
    ///
    /// This also clears any previously configured fragmentation settings.
    /// Re-enable fragmentation explicitly with [`WireframeApp::enable_fragmentation`]
    /// (or [`WireframeApp::fragmentation`]) after changing the frame budget.
    #[must_use]
    pub fn buffer_capacity(mut self, capacity: usize) -> Self {
        let clamped_capacity = clamp_frame_length(capacity);
        self.codec = LengthDelimitedFrameCodec::new(clamped_capacity);
        self.fragmentation = None;
        self
    }
}
