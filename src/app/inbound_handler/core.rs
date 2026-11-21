//! Shared frame and stream processing for prepared application routes.
//!
//! [`process_stream`] prepares the framed transport, assembly state, push
//! queues, and response actor for one connection, then hands the reader, sink,
//! and actor to [`stream_dispatch::run`]. [`decode_outcome`] turns a raw frame
//! into a [`FrameOutcome`] by decoding its envelope in
//! [`parse_envelope`](super::dispatch::parse_envelope), reassembling fragments,
//! and applying protocol-level assembly against the route tables carried by
//! [`StreamProcessingContext`].
//!
//! The streaming loop reaches this module through
//! [`stream_frame::dispatch_frame`](super::stream_frame::dispatch_frame), which
//! calls [`decode_outcome`] for every inbound frame.
//!
//! Push queues and the stop token arrive through [`StreamProcessingContext`]:
//! the queue template the actor clones into these lanes, the server shutdown
//! token, from which the loop derives a child token so its own exit cancels
//! only this connection, the [`SessionRegistry`](crate::session::SessionRegistry)
//! the actor registers with, and the peer address diagnostics report.

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        StreamingHandler,
        codec_driver::FramePipeline,
        envelope::{Envelope, Packet},
        frame_handling::{self, FrameOutcome},
        memory_budgets::MemoryBudgets,
    },
    MAX_DESER_FAILURES,
    actor::{ConnectionActorInit, build_connection_actor},
    dispatch::parse_envelope,
    frames::InboundFrames,
    framing::prepare_connection_framing,
    stream_dispatch::{self, DispatchContext},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, EncodeWith},
    message_assembler::{MessageAssembler, MessageAssemblyState},
    middleware::HandlerService,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};
#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

/// Immutable stream-wide configuration shared by all inbound frames.
pub(crate) struct StreamProcessingContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(crate) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Streaming handlers keyed by message identifier.
    pub(crate) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Serializer shared by all frames on the connection.
    pub(crate) serializer: Arc<S>,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(crate) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(crate) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks applied to every outbound frame on this connection.
    pub(crate) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Protocol whose response hooks drive the connection actor.
    pub(crate) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
    /// Server shutdown signal; cancelling it stops this connection.
    pub(crate) shutdown: &'a CancellationToken,
    /// Template cloned to build this connection's push queues.
    pub(crate) push_queues: PushQueuesBuilder<F::Frame>,
    /// Registry this connection publishes its push handle to.
    pub(crate) session_registry: Arc<SessionRegistry<F::Frame>>,
    /// Peer address of the accepted socket, when the server reported one.
    pub(crate) peer_addr: Option<SocketAddr>,
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(crate) async fn process_stream<S, E, F, W>(
    stream: W,
    context: StreamProcessingContext<'_, S, E, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let (mut sink, mut reader, connection_codec) =
        prepare_connection_framing(stream, context.codec).split();
    let requested_frame_length = connection_codec.max_frame_length();
    let effective_budgets =
        frame_handling::resolve_effective_budgets(context.memory_budgets, requested_frame_length);
    let mut inbound = InboundFrames {
        // A frame read while the previous one was still being handled, so the
        // loop never polls the read half twice for one inbound frame.
        pending: None,
        pipeline: FramePipeline::with_hooks(context.fragmentation, context.protocol_hooks),
        message_assembly: context.message_assembler.map(|_| {
            frame_handling::new_message_assembly_state(
                context.fragmentation,
                requested_frame_length,
                Some(effective_budgets),
            )
        }),
        // Armed on the first silent read, so a fresh connection never starts by
        // purging state it has not buffered.
        read_deadline: None,
    };
    // A child token so the guard below cancels only this connection, while a
    // server shutdown still reaches it through the parent.
    let connection_shutdown = context.shutdown.child_token();
    // Cancelled only by a clean EOF, which is permanent for the connection.
    let peer_eof = CancellationToken::new();
    let (mut response_actor, _cancel_on_exit) = build_connection_actor::<F>(ConnectionActorInit {
        protocol: context.protocol.as_ref(),
        shutdown: connection_shutdown,
        registry: &context.session_registry,
        push_queues: &context.push_queues,
        context: ConnectionContext::default().with_peer_addr(context.peer_addr),
    })?;
    // The identifier is the one the actor claimed when it published its
    // handle, so the sink's callbacks and the registry agree on it. The
    // sink-owned pipeline runs `before_send` too, so it must know which
    // connection the callback is running for.
    inbound.pipeline.set_context(
        ConnectionContext::with_connection_id(Some(response_actor.connection_id))
            .with_peer_addr(context.peer_addr),
    );
    let dispatch = DispatchContext {
        routes: context.routes,
        streaming_routes: context.streaming_routes,
        serializer: context.serializer.as_ref(),
        codec: &connection_codec,
        message_assembler: context.message_assembler,
        protocol: context.protocol.as_ref(),
        effective_budgets,
        read_timeout_ms: context.read_timeout_ms,
    };
    stream_dispatch::run(stream_dispatch::Connection {
        reader: &mut reader,
        sink: &mut sink,
        inbound: &mut inbound,
        actor: &mut response_actor.actor,
        ctx: &dispatch,
        shutdown: context.shutdown,
        peer_eof: &peer_eof,
        wait: Duration::from_millis(context.read_timeout_ms),
    })
    .await
}

/// Borrowed inputs for turning one raw frame into a dispatch outcome.
pub(super) struct FrameBuildContext<'a, S, F>
where
    S: Serializer + Send + Sync,
    F: FrameCodec,
{
    /// Raw frame borrowed while decoding its envelope metadata and payload.
    pub(super) frame: &'a F::Frame,
    /// Serializer used to decode the envelope.
    pub(super) serializer: &'a S,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(super) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Streaming route identifiers eligible for incremental delivery.
    pub(super) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Whether a streaming body already owns the dispatch slot.
    pub(super) stream_live: bool,
    /// Pipeline needed to reassemble fragmented input and emit responses.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Mutable assembly state retained across inbound frames.
    pub(super) message_assembly: &'a mut Option<MessageAssemblyState>,
    /// Failure counter used to enforce the malformed-input limit.
    pub(super) deser_failures: &'a mut u32,
}

/// Decode, reassemble, and assemble one inbound frame.
///
/// A frame that completes a logical message resets the consecutive-failure
/// counter; a frame that merely validates does not. See
/// [`clear_deser_failures_after`] for why each outcome is classified that way.
pub(super) fn decode_outcome<S, F>(context: FrameBuildContext<'_, S, F>) -> io::Result<FrameOutcome>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    F: FrameCodec,
    Envelope: DecodeWith<S>,
{
    let FrameBuildContext {
        frame,
        serializer,
        message_assembler,
        streaming_routes,
        stream_live,
        pipeline,
        message_assembly,
        deser_failures,
    } = context;
    let mut failure_tracker =
        frame_handling::DeserFailureTracker::new(deser_failures, MAX_DESER_FAILURES);
    let Some(envelope) = frame_handling::decode_envelope::<F>(
        parse_envelope(serializer, F::frame_payload(frame)),
        frame,
        &mut failure_tracker,
    )?
    else {
        return Ok(FrameOutcome::Pending);
    };
    let Some(reassembled_envelope) = frame_handling::reassemble_if_needed(
        pipeline,
        deser_failures,
        envelope,
        MAX_DESER_FAILURES,
    )?
    else {
        return Ok(FrameOutcome::Pending);
    };
    let outcome = frame_handling::assemble_if_needed(
        frame_handling::AssemblyRuntime::new(
            message_assembler,
            message_assembly,
            streaming_routes,
            stream_live,
        ),
        deser_failures,
        reassembled_envelope,
        MAX_DESER_FAILURES,
    )?;

    clear_deser_failures_after(&outcome, deser_failures);
    Ok(outcome)
}

/// Clear the consecutive-failure counter after a frame that completed a message.
///
/// Only a completed message proves the peer is making progress, so only that
/// kind of frame ends the current failure run:
///
/// - a buffered message, or a streaming-route message assembled into a pre-filled fallback, arrived
///   in full;
/// - a streaming first frame or continuation that is also the last frame completes its message;
/// - a non-final streaming frame or a discarded continuation validated, but established no
///   completion;
/// - a [`FrameOutcome::Pending`] frame may be an accepted partial or a rejected frame, and a
///   [`FrameOutcome::StreamFailed`] rejection was counted before its outcome was built.
///
/// A peer that follows each rejected message with a fresh non-final first frame
/// would otherwise reset the count every time and never reach
/// [`MAX_DESER_FAILURES`] while completing no valid message.
const fn clear_deser_failures_after(outcome: &FrameOutcome, deser_failures: &mut u32) {
    if completes_message(outcome) {
        *deser_failures = 0;
    }
}

/// Return whether the outcome finished a logical message.
const fn completes_message(outcome: &FrameOutcome) -> bool {
    match outcome {
        FrameOutcome::Buffered(_) | FrameOutcome::StreamFallback { .. } => true,
        FrameOutcome::StreamStarted { is_last, .. } | FrameOutcome::StreamChunk { is_last, .. } => {
            *is_last
        }
        FrameOutcome::Pending | FrameOutcome::Discarded | FrameOutcome::StreamFailed { .. } => {
            false
        }
    }
}
