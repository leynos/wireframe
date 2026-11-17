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
//!
//! The loop drives the reader and writer halves of one framed transport. Each
//! action rebuilds its [`DispatchHandles`] bundle from disjoint borrows, so the
//! reader is never held while a reply is written, and the bundle is never held
//! while the next frame is awaited.

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::{Sink, Stream};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch, MemoryPressureAction},
        memory_budgets::MemoryBudgets,
    },
    actor::observe_peer,
    frames::{InboundFrames, NextFrame},
    peer_watch::{PeerObservation, handle_watching_peer},
    stream_diagnostics,
    stream_frame::dispatch_frame,
    stream_handles::{DispatchHandles, LoopState},
    stream_parked::{dispatch_parked, parked_bytes},
    stream_slot::PendingEvent,
};
use crate::{
    codec::FrameCodec,
    connection::ConnectionActor,
    frame::FrameMetadata,
    hooks::WireframeProtocol,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    request::QueuedBytes,
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
    pub(super) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Protocol whose terminator and completion hooks follow a response.
    pub(super) protocol:
        Option<&'a Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Effective byte budgets for this connection.
    pub(super) effective_budgets: MemoryBudgets,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
}

/// What the loop should do after one step.
enum Step {
    /// Keep reading frames.
    Continue,
    /// Stop dispatching: the peer closed, or a reply left its stream open.
    Closed,
    /// The connection must end with this error.
    Failed(io::Error),
}

/// One connected transport and the immutable inputs that drive it.
struct Connection<'a, S, E, R, K, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    /// Read half of the framed transport.
    reader: &'a mut R,
    /// Write half of the framed transport.
    sink: &'a mut K,
    /// Inbound framing state, including the frame buffered while the previous
    /// one was handled.
    inbound: &'a mut InboundFrames<F::Frame>,
    /// Actor that schedules and delivers every reply.
    actor: &'a mut ConnectionActor<Envelope, ()>,
    /// Immutable connection inputs.
    ctx: &'a DispatchContext<'a, S, E, F>,
    /// Server shutdown signal; cancelling it stops this connection.
    shutdown: &'a CancellationToken,
    /// Connection-wide token cancelled only by a clean peer EOF.
    peer_eof: &'a CancellationToken,
    /// Maximum wait for a handler or a body send while teardown runs.
    wait: Duration,
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(super) async fn run<S, E, F, R, K>(
    reader: &mut R,
    sink: &mut K,
    inbound: &mut InboundFrames<F::Frame>,
    actor: &mut ConnectionActor<Envelope, ()>,
    ctx: &DispatchContext<'_, S, E, F>,
    shutdown: &CancellationToken,
    peer_eof: &CancellationToken,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let wait = Duration::from_millis(ctx.read_timeout_ms);
    let queued = QueuedBytes::new();
    let mut state = LoopState::new(queued);
    let mut connection = Connection {
        reader,
        sink,
        inbound,
        actor,
        ctx,
        shutdown,
        peer_eof,
        wait,
    };
    loop {
        match connection.step(&mut state).await? {
            Step::Continue => {}
            Step::Closed => return Ok(()),
            Step::Failed(error) => return Err(error),
        }
    }
}

impl<S, E, R, K, F> Connection<'_, S, E, R, K, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Advance the connection loop by one decision.
    async fn step(&mut self, state: &mut LoopState) -> io::Result<Step> {
        let queued = state.streams.queued();
        {
            let mut handles = self.handles(queued.clone());
            if expire_finished_assemblies(state, &mut handles).await? == Dispatch::Stop {
                return Ok(Step::Closed);
            }
            apply_memory_pressure(state, &mut handles).await?;
            if !state.streams.is_live()
                && drain_parked(state, &mut handles).await? == Dispatch::Stop
            {
                return Ok(Step::Closed);
            }
            if state.streams.has_pending_send() {
                return drive_pending_admission(state, &mut handles).await;
            }
        }
        let event = self.next_event(state).await;
        self.handle_event(state, event, queued).await
    }

    /// Build the action-scoped handle bundle from disjoint connection borrows.
    ///
    /// The bundle borrows the two mutable pieces of inbound framing rather
    /// than the whole value, so the reader and its buffered frame stay
    /// reachable for the duration of the action.
    fn handles<'h>(&'h mut self, queued: QueuedBytes) -> DispatchHandles<'h, S, E, K, F> {
        DispatchHandles {
            assembly: &mut self.inbound.message_assembly,
            pipeline: &mut self.inbound.pipeline,
            sink: &mut *self.sink,
            actor: &mut *self.actor,
            queued,
            ctx: self.ctx,
            wait: self.wait,
        }
    }

    /// Await the next inbound frame, or the live handler resolving first.
    async fn next_event(&mut self, state: &mut LoopState) -> Event<F::Frame> {
        let read_timeout_ms = self.ctx.read_timeout_ms;
        match state.streams.active.as_mut() {
            Some(active) => {
                tokio::select! {
                    outcome = active.poll_handler() => Event::Handler(outcome),
                    event = self.read_frame(read_timeout_ms) => event,
                }
            }
            None => self.read_frame(read_timeout_ms).await,
        }
    }

    /// Read the next inbound frame and classify it for the loop.
    async fn read_frame(&mut self, read_timeout_ms: u64) -> Event<F::Frame> {
        let read = self
            .inbound
            .next(self.reader, read_timeout_ms, self.shutdown)
            .await;
        match read {
            Ok(NextFrame::Ready(frame)) => Event::Frame(Ok(Some(frame))),
            Ok(NextFrame::Closed) if self.shutdown.is_cancelled() => Event::Shutdown,
            Ok(NextFrame::Closed) => Event::Frame(Ok(None)),
            Ok(NextFrame::TimedOut) => Event::Tick,
            Err(error) => Event::Frame(Err(error)),
        }
    }

    /// Route one loop event to its handler.
    async fn handle_event(
        &mut self,
        state: &mut LoopState,
        event: Event<F::Frame>,
        queued: QueuedBytes,
    ) -> io::Result<Step> {
        match event {
            Event::Shutdown => {
                state.streams.abort();
                Ok(Step::Closed)
            }
            Event::Tick => {
                stream_diagnostics::read_timed_out();
                Ok(Step::Continue)
            }
            Event::Handler(outcome) => {
                let mut handles = self.handles(queued);
                let dispatch = state
                    .streams
                    .finish_active(outcome, handles.reborrow())
                    .await?;
                Ok(finish_step(dispatch))
            }
            Event::Frame(Ok(Some(frame))) => self.dispatch_frame(frame, state, queued).await,
            Event::Frame(Ok(None)) => {
                let mut handles = self.handles(queued);
                close_after_eof(state, &mut handles).await
            }
            Event::Frame(Err(error)) => {
                let mut handles = self.handles(queued);
                fail_after_decode_error(state, &mut handles, error).await
            }
        }
    }

    /// Decode one frame and route it through the dispatch slot.
    async fn dispatch_frame(
        &mut self,
        frame: F::Frame,
        state: &mut LoopState,
        queued: QueuedBytes,
    ) -> io::Result<Step> {
        // Built from field borrows rather than through `handles`, because the
        // reader and the buffered frame must stay reachable for the dispatch.
        let mut handles = DispatchHandles {
            assembly: &mut self.inbound.message_assembly,
            pipeline: &mut self.inbound.pipeline,
            sink: &mut *self.sink,
            actor: &mut *self.actor,
            queued,
            ctx: self.ctx,
            wait: self.wait,
        };
        let (unobserved, cause) = observe_peer(handles.actor, self.peer_eof);
        let dispatch = handle_watching_peer(
            dispatch_frame(&frame, state, handles.reborrow()),
            self.reader,
            PeerObservation::new(self.peer_eof, &unobserved, &cause),
            &mut self.inbound.pending,
        )
        .await?;
        Ok(finish_step(dispatch))
    }
}

/// Translate a response outcome into the loop's next decision.
const fn finish_step(dispatch: Dispatch) -> Step {
    match dispatch {
        Dispatch::Continue => Step::Continue,
        Dispatch::Stop => Step::Closed,
    }
}

/// Advance the in-flight chunk admission, or the handler if it resolves first.
async fn drive_pending_admission<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    match state.streams.drive_pending(handles.wait).await {
        PendingEvent::Send(result) => state.streams.complete_send(result),
        PendingEvent::Handler(outcome) => {
            let dispatch = state
                .streams
                .finish_active(outcome, handles.reborrow())
                .await?;
            return Ok(finish_step(dispatch));
        }
        // The admission stays queued; the next step re-checks expiry and the
        // memory budget before resuming it.
        PendingEvent::Idle => {}
    }
    Ok(Step::Continue)
}

/// Fail bodies whose assemblies expired, then release the expired entries.
async fn expire_finished_assemblies<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let purged = frame_handling::purge_expired_assemblies(handles.assembly);
    handles.pipeline.purge_expired();
    if purged.streaming().is_empty() {
        return Ok(Dispatch::Continue);
    }
    state
        .streams
        .expire(purged.streaming(), handles.reborrow())
        .await
}

/// Abort the connection when retained bytes exceed the hard cap.
async fn apply_memory_pressure<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
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
async fn drain_parked<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let mut dispatch = Dispatch::Continue;
    while let Some(message) = state.parked.pop_front() {
        // The messages still queued behind this one keep their bytes
        // reserved while it is answered.
        dispatch = dispatch_parked(message, &state.parked, handles.reborrow()).await?;
        if dispatch == Dispatch::Stop {
            break;
        }
    }
    Ok(dispatch)
}

/// Fail the live body, then end the loop with `step`.
async fn fail_body_ending_with<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
    reason: io::Error,
    step: Step,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    state
        .streams
        .close_active(Some(reason), handles.reborrow())
        .await?;
    Ok(step)
}

/// Fail the live body, then end the connection with the decoder error.
async fn fail_after_decode_error<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
    error: io::Error,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        error.kind(),
        "inbound frame decoding failed while a request body was open",
    );
    fail_body_ending_with(state, handles, reason, Step::Failed(error)).await
}

/// Fail the live body, answer parked messages, and end the connection cleanly.
async fn close_after_eof<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "connection closed before the request body completed",
    );
    fail_body_ending_with(state, handles, reason, Step::Closed).await?;
    drain_parked(state, handles).await?;
    Ok(Step::Closed)
}

/// One step of the connection loop.
pub(super) enum Event<T> {
    /// An inbound frame, EOF, or decoder error.
    Frame(io::Result<Option<T>>),
    /// The server stopped the connection.
    Shutdown,
    /// The read timeout elapsed with no frame.
    Tick,
    /// The handler resolved while the loop was waiting.
    Handler(Option<Bytes>),
}
