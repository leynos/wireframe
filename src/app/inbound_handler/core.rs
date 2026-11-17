//! Shared frame and stream processing for prepared application routes.

use std::{collections::HashMap, sync::Arc};

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
    actor::build_connection_actor,
    frames::InboundFrames,
    framing::prepare_connection_framing,
    stream_dispatch::{self, DispatchContext},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    hooks::{ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, DeserializeContext, EncodeWith},
    message_assembler::{MessageAssembler, MessageAssemblyState},
    middleware::HandlerService,
    serializer::Serializer,
};
#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

/// Immutable stream-wide configuration shared by all inbound frames.
pub(super) struct StreamProcessingContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(super) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Streaming handlers keyed by message identifier.
    pub(super) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Serializer shared by all frames on the connection.
    pub(super) serializer: Arc<S>,
    /// Codec configuration shared by all frames on the connection.
    pub(super) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(super) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks applied to every outbound frame on this connection.
    pub(super) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Protocol whose response hooks drive the connection actor.
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(super) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
    /// Server shutdown signal; cancelling it stops this connection.
    pub(super) shutdown: &'a CancellationToken,
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(super) async fn process_stream<S, E, F, W>(
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
    };
    // A child token so the guard below cancels only this connection, while a
    // server shutdown still reaches it through the parent.
    let connection_shutdown = context.shutdown.child_token();
    // Cancelled only by a clean EOF, which is permanent for the connection.
    let peer_eof = CancellationToken::new();
    let (mut response_actor, _cancel_on_exit) =
        build_connection_actor::<F>(context.protocol.as_ref(), connection_shutdown)?;
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
    stream_dispatch::run(
        &mut reader,
        &mut sink,
        &mut inbound,
        &mut response_actor.actor,
        &dispatch,
        context.shutdown,
        &peer_eof,
    )
    .await
}

/// Detect a bincode budget failure so fallback decoding does not repeat the work.
fn is_bincode_limit_exceeded(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<bincode::error::DecodeError>()
        .is_some_and(|decode_error| {
            matches!(decode_error, bincode::error::DecodeError::LimitExceeded)
        })
}

/// Decode exactly one envelope, using metadata to select full deserialization.
pub(super) fn parse_envelope<S>(
    serializer: &S,
    payload: &[u8],
) -> std::result::Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    Envelope: DecodeWith<S>,
{
    let (envelope, consumed) = match serializer.parse(payload) {
        Ok((parsed_envelope, metadata_bytes_consumed)) => {
            if serializer.should_deserialize_after_parse() {
                let context = DeserializeContext {
                    frame_metadata: payload.get(..metadata_bytes_consumed),
                    message_id: Some(parsed_envelope.id),
                    correlation_id: parsed_envelope.correlation_id,
                    metadata_bytes_consumed: Some(metadata_bytes_consumed),
                };
                serializer.deserialize_with_context::<Envelope>(payload, &context)
            } else {
                Ok((parsed_envelope, metadata_bytes_consumed))
            }
        }
        Err(error) if is_bincode_limit_exceeded(&error) => {
            Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
        }
        Err(_) => serializer.deserialize::<Envelope>(payload),
    }?;
    if consumed != payload.len() {
        crate::metrics::inc_decode_rejection(
            crate::metrics::DecodeRejectionPath::Envelope,
            crate::metrics::DecodeRejectionReason::TrailingBytes,
        );
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing bytes after envelope: frame must contain exactly one value",
        )
        .into());
    }
    Ok((envelope, consumed))
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
