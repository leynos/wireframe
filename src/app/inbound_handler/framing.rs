//! Per-connection framing state for prepared inbound processing.
//!
//! Framing is built once per connection and then shared by the inbound reader
//! and the actor-driven response writer. Keeping the construction here leaves
//! the frame-processing loop focused on dispatch.

use std::sync::Arc;

use futures::{
    StreamExt,
    stream::{SplitSink, SplitStream},
};
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

// The decoder half is recovery-aware, so a decoder failure arrives as a
// stream item rather than terminating the stream.
use super::super::combined_codec::{ConnectionCodec, connection_codec};
use super::{
    super::{
        codec_driver::FramePipeline,
        frame_handling,
        memory_budgets::MemoryBudgets,
        recovery_codec::DecodedItem,
    },
    frames::InboundFrames,
};
use crate::{
    codec::{FrameCodec, MAX_FRAME_LENGTH, clamp_frame_length},
    hooks::ProtocolHooks,
    message_assembler::MessageAssembler,
};

/// Per-connection framing state shared by the inbound reader and response
/// writer.
pub(super) struct ConnectionFraming<W, F>
where
    F: FrameCodec,
{
    /// Codec clone that carries per-connection framing state.
    pub(super) codec: F,
    /// Framed stream built from the codec's decoder and encoder.
    pub(super) framed: Framed<W, ConnectionCodec<F>>,
}

/// Write half of a framed connection transport.
pub(super) type TransportSink<W, F> =
    SplitSink<Framed<W, ConnectionCodec<F>>, <F as FrameCodec>::Frame>;

/// Read half of a framed connection transport.
pub(super) type TransportStream<W, F> = SplitStream<Framed<W, ConnectionCodec<F>>>;

impl<W, F> ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Split the transport into its write sink, read stream, and codec.
    pub(super) fn split(self) -> (TransportSink<W, F>, TransportStream<W, F>, F) {
        let Self { codec, framed } = self;
        let (sink, stream) = framed.split();
        (sink, stream, codec)
    }
}

/// Build per-connection framing state and guard the requested frame length.
pub(super) fn prepare_connection_framing<W, F>(stream: W, codec: &F) -> ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    // Each connection needs isolated framing state: cloning resets the
    // counters `SeqFrameCodec` and `TaggedFrameCodec::wrap_payload` consume.
    let session_codec = codec.clone();
    let mut framed = Framed::new(stream, connection_codec(&session_codec));
    let requested_frame_length = session_codec.max_frame_length();
    let max_frame_length = clamp_frame_length(requested_frame_length);
    if requested_frame_length > MAX_FRAME_LENGTH {
        warn!(
            "codec max frame length exceeds guardrail; clamping to {MAX_FRAME_LENGTH} bytes \
             (requested={requested_frame_length})"
        );
    }
    framed.read_buffer_mut().reserve(max_frame_length);
    ConnectionFraming {
        codec: session_codec,
        framed,
    }
}

/// Frame-length limit and resolved budgets for one connection's framing.
#[derive(Clone, Copy)]
pub(super) struct FramingLimits {
    /// Maximum frame length advertised by the codec.
    requested_frame_length: usize,
    /// Budgets resolved from the connection's configuration.
    pub(super) effective_budgets: MemoryBudgets,
}

/// Resolve the frame-length limit and memory budgets for one connection.
pub(super) fn resolve_framing_limits<F>(budgets: Option<MemoryBudgets>, codec: &F) -> FramingLimits
where
    F: FrameCodec,
{
    let requested_frame_length = codec.max_frame_length();
    FramingLimits {
        requested_frame_length,
        effective_budgets: frame_handling::resolve_effective_budgets(
            budgets,
            requested_frame_length,
        ),
    }
}

/// Build the framing state one inbound loop reads and dispatches through.
///
/// Assembly state exists only when an assembler is installed, so a read
/// timeout purges exactly the state this connection can hold.
pub(super) fn prepare_inbound_state<F>(
    fragmentation: Option<crate::fragment::FragmentationConfig>,
    protocol_hooks: ProtocolHooks<F::Frame, ()>,
    message_assembler: Option<&Arc<dyn MessageAssembler>>,
    limits: FramingLimits,
) -> InboundFrames<DecodedItem<F::Frame>, F::Frame>
where
    F: FrameCodec,
{
    InboundFrames {
        // A frame read while the previous one was still being handled, so the
        // loop never polls the read half twice for one inbound frame.
        pending: None,
        pipeline: FramePipeline::with_hooks(fragmentation, protocol_hooks),
        message_assembly: message_assembler.map(|_| {
            frame_handling::new_message_assembly_state(
                fragmentation,
                limits.requested_frame_length,
                Some(limits.effective_budgets),
            )
        }),
    }
}
