//! Connection loop with a single streaming request-body dispatch slot.
//!
//! A connection delivers at most one streaming request body at a time. The
//! slot owns the handler future, the byte-accounted sender, and at most one
//! in-flight chunk admission. No task is detached: the loop polls the handler,
//! the admission future, and the inbound socket from one place, so cancelling
//! the connection cancels every part of the exchange.
//!
//! Completed buffered messages that arrive while a stream is live are parked
//! and dispatched in completion order once the slot frees. A message for a
//! streaming route that completed while the slot was busy is parked too and
//! later delivered as a pre-filled, single-chunk body stream.

use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::Arc,
    time::Duration,
};

use futures::StreamExt;
use log::{debug, warn};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};
use tokio_util::codec::Framed;

use super::{
    super::{
        StreamingHandler,
        codec_driver::FramePipeline,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
        frame_handling::{self, FrameOutcome, MemoryPressureAction, ResponseContext},
        memory_budgets::MemoryBudgets,
    },
    core::{FrameBuildContext, decode_outcome},
    stream_parked::{ParkedMessage, dispatch_parked, parked_bytes},
    stream_slot::{ConnectionStreams, PendingEvent},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssemblyState,
    middleware::HandlerService,
    serializer::Serializer,
};

/// Immutable connection inputs needed by the dispatch loop.
pub(super) struct DispatchContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch buffered envelopes.
    pub(super) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Streaming handlers keyed by message identifier.
    pub(super) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Serializer shared by all frames on the connection.
    pub(super) serializer: &'a S,
    /// Codec configuration shared by all frames on the connection.
    pub(super) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(super) message_assembler: Option<&'a Arc<dyn crate::message_assembler::MessageAssembler>>,
    /// Effective byte budgets for this connection.
    pub(super) effective_budgets: MemoryBudgets,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(super) async fn run<S, E, F, W>(
    mut framed: Framed<W, ConnectionCodec<F>>,
    ctx: DispatchContext<'_, S, E, F>,
    mut pipeline: FramePipeline<F::Frame>,
    mut message_assembly: Option<MessageAssemblyState>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let mut deser_failures = 0u32;
    let mut streams = ConnectionStreams::new();
    let mut parked: VecDeque<ParkedMessage> = VecDeque::new();
    let wait = Duration::from_millis(ctx.read_timeout_ms);

    loop {
        let purged = frame_handling::purge_expired_assemblies(&mut message_assembly);
        pipeline.purge_expired();
        if !purged.streaming().is_empty() {
            streams
                .expire(
                    purged.streaming(),
                    &mut message_assembly,
                    &mut pipeline,
                    &mut framed,
                    &ctx,
                    wait,
                )
                .await?;
        }

        let pressure = frame_handling::evaluate_memory_pressure_with_retained(
            message_assembly.as_ref(),
            Some(ctx.effective_budgets),
            pipeline.fragmentation_buffered_bytes(),
            streams
                .retained_bytes()
                .saturating_add(parked_bytes(&parked)),
        );
        if matches!(pressure, MemoryPressureAction::Abort) {
            streams.abort();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "per-connection memory budget hard cap exceeded",
            ));
        }
        frame_handling::apply_memory_pressure(pressure, || {}).await?;

        if !streams.is_live() {
            while let Some(message) = parked.pop_front() {
                dispatch_parked(
                    message,
                    &ctx,
                    &mut pipeline,
                    &mut framed,
                    message_assembly.as_ref(),
                )
                .await?;
            }
        }

        if let Some(active) = streams.active.as_mut()
            && active.pending.is_some()
        {
            let event = active.drive_pending().await;
            match event {
                PendingEvent::Send(result) => streams.complete_send(result),
                PendingEvent::Handler(outcome) => {
                    streams
                        .finish_active(
                            outcome,
                            &mut message_assembly,
                            &mut pipeline,
                            &mut framed,
                            &ctx,
                        )
                        .await?;
                }
            }
            continue;
        }

        let event = if let Some(active) = streams.active.as_mut() {
            active.drive_open::<F, W>(&mut framed, wait).await
        } else {
            read_frame::<F, W>(&mut framed, wait).await
        };

        match event {
            Event::Tick => {
                debug!("read timeout elapsed; continuing to wait for next frame");
            }
            Event::Handler(outcome) => {
                streams
                    .finish_active(
                        outcome,
                        &mut message_assembly,
                        &mut pipeline,
                        &mut framed,
                        &ctx,
                    )
                    .await?;
            }
            Event::Frame(Err(error)) => {
                streams
                    .close_active(
                        Some(io::Error::new(
                            error.kind(),
                            "inbound frame decoding failed while a request body was open",
                        )),
                        &mut message_assembly,
                        &mut pipeline,
                        &mut framed,
                        &ctx,
                        wait,
                    )
                    .await?;
                return Err(error);
            }
            Event::Frame(Ok(None)) => {
                streams
                    .close_active(
                        Some(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "connection closed before the request body completed",
                        )),
                        &mut message_assembly,
                        &mut pipeline,
                        &mut framed,
                        &ctx,
                        wait,
                    )
                    .await?;
                while let Some(message) = parked.pop_front() {
                    dispatch_parked(
                        message,
                        &ctx,
                        &mut pipeline,
                        &mut framed,
                        message_assembly.as_ref(),
                    )
                    .await?;
                }
                return Ok(());
            }
            Event::Frame(Ok(Some(frame))) => {
                dispatch_frame(
                    &frame,
                    &mut streams,
                    &mut parked,
                    &mut deser_failures,
                    &mut message_assembly,
                    &mut pipeline,
                    &mut framed,
                    &ctx,
                    wait,
                )
                .await?;
            }
        }
    }
}

/// Decode one frame and route its outcome through the dispatch slot.
#[expect(
    clippy::too_many_arguments,
    reason = "loop state is threaded explicitly to avoid shared mutable aliases"
)]
async fn dispatch_frame<S, E, F, W>(
    frame: &F::Frame,
    streams: &mut ConnectionStreams,
    parked: &mut VecDeque<ParkedMessage>,
    deser_failures: &mut u32,
    message_assembly: &mut Option<MessageAssemblyState>,
    pipeline: &mut FramePipeline<F::Frame>,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    ctx: &DispatchContext<'_, S, E, F>,
    wait: Duration,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
    let stream_live = streams.is_live();
    let outcome = decode_outcome(FrameBuildContext::<S, F> {
        frame,
        serializer: ctx.serializer,
        message_assembler: ctx.message_assembler,
        streaming_routes: ctx.streaming_routes,
        stream_live,
        pipeline,
        message_assembly,
        deser_failures,
    })?;
    let other_retained = message_assembly
        .as_ref()
        .map_or(0, MessageAssemblyState::total_buffered_bytes)
        .saturating_add(pipeline.fragmentation_buffered_bytes());
    let limit = frame_handling::active_aggregate_limit_bytes(ctx.effective_budgets);

    match outcome {
        FrameOutcome::Pending | FrameOutcome::Discarded => Ok(()),
        FrameOutcome::Buffered(env) => {
            if stream_live {
                parked.push_back(ParkedMessage::Envelope(env));
                return Ok(());
            }
            let Some(service) = ctx.routes.get(&env.id) else {
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
                    serializer: ctx.serializer,
                    framed,
                    pipeline,
                    codec: ctx.codec,
                },
            )
            .await
        }
        FrameOutcome::StreamStarted {
            key,
            parts,
            chunk,
            is_last,
        } => {
            if let Some(handler) = ctx.streaming_routes.get(&parts.id()) {
                streams
                    .start(key, parts, chunk, is_last, handler, other_retained, limit)
                    .await;
            } else {
                warn!("no streaming handler for message id: id={}", parts.id());
            }
            Ok(())
        }
        FrameOutcome::StreamChunk {
            key,
            chunk,
            is_last,
        } => {
            if !streams.push_chunk(key, chunk, is_last, other_retained, limit)
                && let Some(state) = message_assembly.as_mut()
            {
                debug!("dropping chunk for abandoned streaming message: key={key:?}");
                state.discard_streaming(key);
            }
            Ok(())
        }
        FrameOutcome::StreamFallback { parts, body } => {
            let message = ParkedMessage::Stream { parts, body };
            if stream_live {
                parked.push_back(message);
                return Ok(());
            }
            dispatch_parked(message, ctx, pipeline, framed, message_assembly.as_ref()).await
        }
        FrameOutcome::StreamFailed { key, error } => {
            streams
                .fail(key, error, message_assembly, pipeline, framed, ctx, wait)
                .await
        }
    }
}

/// Read one frame, reporting a read timeout as a tick.
pub(super) async fn read_frame<F, W>(
    framed: &mut Framed<W, ConnectionCodec<F>>,
    wait: Duration,
) -> Event<F::Frame>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    match timeout(wait, framed.next()).await {
        Ok(Some(Ok(frame))) => Event::Frame(Ok(Some(frame))),
        Ok(Some(Err(error))) => Event::Frame(Err(error)),
        Ok(None) => Event::Frame(Ok(None)),
        Err(_) => Event::Tick,
    }
}

/// One step of the connection loop.
pub(super) enum Event<T> {
    /// An inbound frame, EOF, or decoder error.
    Frame(io::Result<Option<T>>),
    /// The read timeout elapsed.
    Tick,
    /// The handler resolved while the loop was waiting.
    Handler(Option<Vec<u8>>),
}
