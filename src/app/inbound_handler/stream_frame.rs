//! Routing of one decoded inbound frame into the streaming dispatch slot.
//!
//! [`dispatch_frame`] decodes a frame, then hands each [`FrameOutcome`] to the
//! component that owns it: the slot for streaming bodies, the middleware chain
//! for complete buffered messages, and the parking queue for anything that
//! arrives while the slot is busy. Keeping this routing separate from the loop
//! leaves the loop readable as a sequence of decisions.

use std::io;

use log::{debug, warn};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::{self, FrameOutcome, ResponseContext},
    },
    core::{FrameBuildContext, decode_outcome},
    stream_handles::{ChunkBudget, ChunkOffer, DispatchHandles, FirstChunk, LoopState},
    stream_parked::{self, ParkedMessage, dispatch_parked},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssemblyState,
    serializer::Serializer,
};

/// Decode one frame and route its outcome through the dispatch slot.
pub(super) async fn dispatch_frame<S, E, F, W>(
    frame: &F::Frame,
    state: &mut LoopState,
    mut handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
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
    /// Byte budget for any body chunk this frame carries.
    budget: ChunkBudget,
}

/// Decode `frame` and measure the budget its routing needs.
fn decode_frame<S, E, F, W>(
    frame: &F::Frame,
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<DecodedFrame>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
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
        budget: chunk_budget(handles),
    })
}

/// Compute the byte budget one body chunk must fit within.
fn chunk_budget<S, E, F, W>(handles: &DispatchHandles<'_, S, E, F, W>) -> ChunkBudget
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    let other_retained = handles
        .assembly
        .as_ref()
        .map_or(0, MessageAssemblyState::total_buffered_bytes)
        .saturating_add(handles.pipeline.fragmentation_buffered_bytes());
    ChunkBudget {
        other_retained,
        limit: frame_handling::active_aggregate_limit_bytes(handles.ctx.effective_budgets),
    }
}

/// Hand one decoded outcome to the component that owns it.
async fn route_outcome<S, E, F, W>(
    decoded: DecodedFrame,
    state: &mut LoopState,
    mut handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let DecodedFrame {
        outcome,
        stream_live,
        budget,
    } = decoded;
    match outcome {
        FrameOutcome::Pending | FrameOutcome::Discarded => Ok(()),
        FrameOutcome::Buffered(env) => dispatch_buffered(env, stream_live, state, handles).await,
        FrameOutcome::StreamStarted {
            key,
            parts,
            chunk,
            is_last,
        } => {
            let streaming_handler = handles.ctx.streaming_routes.get(&parts.id());
            let first = FirstChunk {
                key,
                parts,
                chunk,
                is_last,
            };
            start_stream(state, streaming_handler, first, budget).await;
            Ok(())
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
            deliver_chunk(state, handles.assembly, offer, budget);
            Ok(())
        }
        FrameOutcome::StreamFallback { parts, body } => {
            let message = ParkedMessage::Stream { parts, body };
            if stream_live {
                stream_parked::park(&mut state.parked, message)?;
                return Ok(());
            }
            dispatch_parked(message, handles).await
        }
        FrameOutcome::StreamFailed { key, error } => {
            state.streams.fail(key, error, handles.reborrow()).await
        }
    }
}

/// Hand a first streaming frame to its route handler.
///
/// The handler is resolved by the caller so this helper borrows only what it
/// needs: taking the whole [`DispatchHandles`] would require it to be `Sync`,
/// which the framed connection is not.
async fn start_stream(
    state: &mut LoopState,
    route_handler: Option<&StreamingHandler>,
    first: FirstChunk,
    budget: ChunkBudget,
) {
    let Some(handler) = route_handler else {
        warn!(
            "no streaming handler for message id: id={}",
            first.parts.id()
        );
        return;
    };
    state.streams.start(first, handler, budget).await;
}

/// Queue a continuation chunk, discarding it when the body was abandoned.
fn deliver_chunk(
    state: &mut LoopState,
    assembly: &mut Option<MessageAssemblyState>,
    offer: ChunkOffer,
    budget: ChunkBudget,
) {
    let key = offer.key;
    if state.streams.push_chunk(offer, budget) {
        return;
    }
    if let Some(assembly_state) = assembly.as_mut() {
        debug!("dropping chunk for abandoned streaming message: key={key:?}");
        assembly_state.discard_streaming(key);
    }
}

/// Send a complete buffered message through its middleware chain, or park it.
async fn dispatch_buffered<S, E, F, W>(
    env: Envelope,
    stream_live: bool,
    state: &mut LoopState,
    handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    if stream_live {
        stream_parked::park(&mut state.parked, ParkedMessage::Envelope(env))?;
        return Ok(());
    }
    let Some(service) = handles.ctx.routes.get(&env.id) else {
        warn!(
            "no handler for message id: id={}, correlation_id={:?}",
            env.id, env.correlation_id
        );
        return Ok(());
    };
    frame_handling::forward_response(
        env,
        service,
        ResponseContext {
            serializer: handles.ctx.serializer,
            framed: handles.framed,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
        },
    )
    .await
}
