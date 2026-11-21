//! Routing of one decoded inbound frame into the streaming dispatch slot.
//!
//! [`dispatch_frame`] decodes a frame, then hands each [`FrameOutcome`] to the
//! component that owns it: the slot for streaming bodies, the middleware chain
//! for complete buffered messages, and the parking queue for anything that
//! arrives while the slot is busy. Keeping this routing separate from the loop
//! leaves the loop readable as a sequence of decisions.

use std::io;

use futures::Sink;
use tracing::warn;

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch, FrameOutcome, ResponseContext},
    },
    core::{FrameBuildContext, decode_outcome},
    stream_diagnostics,
    stream_handles::{ChunkBudget, ChunkOffer, DispatchHandles, FirstChunk, LoopState},
    stream_parked::{self, ParkedMessage, dispatch_parked},
    stream_slot::BodyFailure,
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssemblyState,
    metrics::StreamingBodyOutcome,
    serializer::Serializer,
};

#[cfg(test)]
#[cfg(feature = "metrics")]
#[path = "stream_outcome_ingress_tests.rs"]
mod outcome_tests;

/// Decode one frame and route its outcome through the dispatch slot.
pub(super) async fn dispatch_frame<S, E, K, F>(
    frame: &F::Frame,
    state: &mut LoopState,
    mut handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
    let decoded = decode_frame(frame, state, &mut handles)?;
    route_outcome(decoded, state, handles).await
}

/// One decoded frame plus the context its routing needs.
struct DecodedFrame {
    /// Routing decision produced by the decoder.
    outcome: FrameOutcome,
    /// Whether a streaming body already owns the dispatch slot.
    stream_live: bool,
}

/// Decode `frame` and measure the budget its routing needs.
fn decode_frame<S, E, K, F>(
    frame: &F::Frame,
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<DecodedFrame>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let stream_live = state.streams.is_live();
    let outcome = decode_outcome(FrameBuildContext::<S, F> {
        frame,
        serializer: handles.ctx.serializer,
        message_assembler: handles.ctx.message_assembler,
        streaming_routes: handles.ctx.streaming_routes,
        stream_live,
        pipeline: &mut *handles.pipeline,
        message_assembly: &mut *handles.assembly,
        deser_failures: &mut state.deser_failures,
    })?;
    Ok(DecodedFrame {
        outcome,
        stream_live,
    })
}

/// Hand one decoded outcome to the component that owns it.
async fn route_outcome<S, E, K, F>(
    decoded: DecodedFrame,
    state: &mut LoopState,
    mut handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let DecodedFrame {
        outcome,
        stream_live,
    } = decoded;
    match outcome {
        FrameOutcome::Pending | FrameOutcome::Discarded => Ok(Dispatch::Continue),
        FrameOutcome::Buffered(env) => dispatch_buffered(env, stream_live, state, handles).await,
        FrameOutcome::StreamStarted {
            key,
            assembly,
            parts,
            chunk,
            is_last,
        } => {
            let streaming_handler = handles.ctx.streaming_routes.get(&parts.id());
            let first = FirstChunk {
                key,
                assembly,
                parts,
                chunk,
                is_last,
            };
            start_stream(state, streaming_handler, first);
            Ok(Dispatch::Continue)
        }
        FrameOutcome::StreamChunk {
            key,
            chunk,
            is_last,
        } => {
            let offer = ChunkOffer {
                key,
                chunk,
                is_last,
            };
            deliver_chunk(state, handles.assembly, offer);
            Ok(Dispatch::Continue)
        }
        FrameOutcome::StreamFallback { parts, body } => {
            let message = ParkedMessage::Stream { parts, body };
            if stream_live {
                stream_parked::park(&mut state.parked, message)?;
                return Ok(Dispatch::Continue);
            }
            dispatch_parked(message, &state.parked, handles).await
        }
        FrameOutcome::StreamFailed { key, error } => {
            let budget = ChunkBudget::measure(&handles, &state.parked);
            let failure = BodyFailure { key, error };
            state
                .streams
                .fail(failure, budget, handles.reborrow())
                .await
        }
    }
}

/// Hand a first streaming frame to its route handler.
///
/// The handler is resolved by the caller so this helper borrows only what it
/// needs: taking the whole [`DispatchHandles`] would require it to be `Sync`,
/// which the transport sink is not. The call only registers the exchange;
/// the loop admits the first chunk on a later step so a busy byte account
/// cannot stall this frame's processing.
fn start_stream(
    state: &mut LoopState,
    route_handler: Option<&StreamingHandler>,
    first: FirstChunk,
) {
    let Some(handler) = route_handler else {
        stream_diagnostics::no_streaming_handler(first.parts.id());
        StreamingBodyOutcome::NoHandler.record();
        return;
    };
    state.streams.start(first, handler);
}

/// Queue a continuation chunk, discarding it when the body was abandoned.
fn deliver_chunk(
    state: &mut LoopState,
    assembly: &mut Option<MessageAssemblyState>,
    offer: ChunkOffer,
) {
    let key = offer.key;
    if state.streams.push_chunk(offer) {
        return;
    }
    stream_diagnostics::abandoned_chunk(key);
    StreamingBodyOutcome::Abandoned.record();
    if let Some(assembly_state) = assembly.as_mut() {
        assembly_state.discard_streaming(key);
    }
}

/// Send a complete buffered message through its middleware chain, or park it.
async fn dispatch_buffered<S, E, K, F>(
    env: Envelope,
    stream_live: bool,
    state: &mut LoopState,
    handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    if stream_live {
        stream_parked::park(&mut state.parked, ParkedMessage::Envelope(env))?;
        return Ok(Dispatch::Continue);
    }
    let Some(service) = handles.ctx.routes.get(&env.id) else {
        warn!(
            message_id = env.id,
            correlation_id = ?env.correlation_id,
            "no handler for message id"
        );
        return Ok(Dispatch::Continue);
    };
    frame_handling::forward_response(
        env,
        service,
        ResponseContext {
            serializer: handles.ctx.serializer,
            sink: handles.sink,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
            actor: handles.actor,
            protocol: handles.ctx.protocol,
        },
    )
    .await
}
