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

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use futures::StreamExt;
use log::debug;
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
        frame_handling::{self, MemoryPressureAction},
        memory_budgets::MemoryBudgets,
    },
    stream_frame::dispatch_frame,
    stream_handles::{DispatchHandles, LoopState},
    stream_parked::{dispatch_parked, parked_bytes},
    stream_slot::PendingEvent,
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

/// What the loop should do after one step.
enum Step {
    /// Keep reading frames.
    Continue,
    /// The peer closed the connection and every parked message was answered.
    Closed,
    /// The connection must end with this error.
    Failed(io::Error),
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(super) async fn run<S, E, F, W>(
    mut framed: Framed<W, ConnectionCodec<F>>,
    ctx: DispatchContext<'_, S, E, F>,
    mut pipeline: FramePipeline<F::Frame>,
    message_assembly: Option<MessageAssemblyState>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let wait = Duration::from_millis(ctx.read_timeout_ms);
    let mut state = LoopState::new();
    let mut assembly = message_assembly;
    let mut handles = DispatchHandles {
        assembly: &mut assembly,
        pipeline: &mut pipeline,
        framed: &mut framed,
        ctx: &ctx,
        wait,
    };
    loop {
        match step(&mut state, &mut handles).await? {
            Step::Continue => {}
            Step::Closed => return Ok(()),
            Step::Failed(error) => return Err(error),
        }
    }
}

/// Advance the connection loop by one decision.
async fn step<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<Step>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    expire_finished_assemblies(state, handles).await?;
    apply_memory_pressure(state, handles).await?;
    if !state.streams.is_live() {
        drain_parked(state, handles).await?;
    }
    if state.streams.has_pending_send() {
        return drive_pending_admission(state, handles).await;
    }
    let event = next_event(state, handles).await;
    handle_event(state, handles, event).await
}

/// Advance the in-flight chunk admission, or the handler if it resolves first.
async fn drive_pending_admission<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    match state.streams.drive_pending().await {
        PendingEvent::Send(result) => state.streams.complete_send(result),
        PendingEvent::Handler(outcome) => {
            state
                .streams
                .finish_active(outcome, handles.reborrow())
                .await?;
        }
    }
    Ok(Step::Continue)
}

/// Fail bodies whose assemblies expired, then release the expired entries.
async fn expire_finished_assemblies<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let purged = frame_handling::purge_expired_assemblies(handles.assembly);
    handles.pipeline.purge_expired();
    if purged.streaming().is_empty() {
        return Ok(());
    }
    state
        .streams
        .expire(purged.streaming(), handles.reborrow())
        .await
}

/// Abort the connection when retained bytes exceed the hard cap.
async fn apply_memory_pressure<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    let pressure = frame_handling::evaluate_memory_pressure_with_retained(
        handles.assembly.as_ref(),
        Some(handles.ctx.effective_budgets),
        handles.pipeline.fragmentation_buffered_bytes(),
        state
            .streams
            .retained_bytes()
            .saturating_add(parked_bytes(&state.parked)),
    );
    if matches!(pressure, MemoryPressureAction::Abort) {
        state.streams.abort();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "per-connection memory budget hard cap exceeded",
        ));
    }
    frame_handling::apply_memory_pressure(pressure, || {}).await
}

/// Dispatch every parked message while the slot is free.
async fn drain_parked<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    while let Some(message) = state.parked.pop_front() {
        dispatch_parked(message, handles.reborrow()).await?;
    }
    Ok(())
}

/// Await the next inbound frame, or the live handler resolving first.
async fn next_event<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> Event<F::Frame>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    match state.streams.active.as_mut() {
        Some(active) => {
            active
                .drive_open::<F, W>(handles.framed, handles.wait)
                .await
        }
        None => read_frame::<F, W>(handles.framed, handles.wait).await,
    }
}

/// Route one loop event to its handler.
async fn handle_event<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
    event: Event<F::Frame>,
) -> io::Result<Step>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    match event {
        Event::Tick => {
            debug!("read timeout elapsed; continuing to wait for next frame");
            Ok(Step::Continue)
        }
        Event::Handler(outcome) => {
            state
                .streams
                .finish_active(outcome, handles.reborrow())
                .await?;
            Ok(Step::Continue)
        }
        Event::Frame(Err(error)) => fail_after_decode_error(state, handles, error).await,
        Event::Frame(Ok(None)) => close_after_eof(state, handles).await,
        Event::Frame(Ok(Some(frame))) => {
            dispatch_frame(&frame, state, handles.reborrow()).await?;
            Ok(Step::Continue)
        }
    }
}

/// Fail the live body, then end the connection with the decoder error.
async fn fail_after_decode_error<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
    error: io::Error,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        error.kind(),
        "inbound frame decoding failed while a request body was open",
    );
    state
        .streams
        .close_active(Some(reason), handles.reborrow())
        .await?;
    Ok(Step::Failed(error))
}

/// Fail the live body, answer parked messages, and end the connection cleanly.
async fn close_after_eof<S, E, F, W>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, F, W>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "connection closed before the request body completed",
    );
    state
        .streams
        .close_active(Some(reason), handles.reborrow())
        .await?;
    drain_parked(state, handles).await?;
    Ok(Step::Closed)
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
