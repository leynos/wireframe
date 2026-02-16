//! Codec and serializer configuration for `WireframeApp`.

use super::{WireframeApp, core::RebuildParams};
use crate::{
    app::Packet,
    codec::{FrameCodec, LengthDelimitedFrameCodec, clamp_frame_length},
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Replace the frame codec used for framing I/O.
    ///
    /// This resets any installed protocol hooks because the frame type may
    /// change across codecs. Fragmentation is disabled so callers can
    /// reconfigure explicitly for the new frame budget.
    #[must_use]
    pub fn with_codec<F2: FrameCodec>(mut self, codec: F2) -> WireframeApp<S, C, E, F2>
    where
        S: Default,
    {
        let serializer = std::mem::take(&mut self.serializer);
        let message_assembler = self.message_assembler.take();
        self.rebuild_with_params(RebuildParams {
            serializer,
            codec,
            protocol: None,
            fragmentation: None,
            message_assembler,
        })
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
        self.rebuild_with_params(RebuildParams {
            serializer,
            codec,
            protocol,
            fragmentation,
            message_assembler,
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
        let capacity = clamp_frame_length(capacity);
        self.codec = LengthDelimitedFrameCodec::new(capacity);
        self.fragmentation = None;
        self
    }
}
