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
//!
//! This module owns the state machine — which event arrived, what it means, and
//! what to do next. The maintenance actions it schedules live in
//! [`super::stream_actions`].

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::{Sink, Stream};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
        memory_budgets::MemoryBudgets,
    },
    actor::observe_peer,
    frames::{InboundFrames, NextFrame},
    peer_watch::{PeerObservation, handle_watching_peer},
    stream_actions::{
        apply_memory_pressure,
        drain_parked,
        drive_pending_admission,
        expire_finished_assemblies,
        fail_after_decode_error,
        fail_body_ending_with,
        finish_step,
    },
    stream_diagnostics,
    stream_frame::dispatch_frame,
    stream_handles::{DispatchHandles, LoopState},
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

#[cfg(test)]
#[path = "stream_watch_tests.rs"]
mod watch_tests;

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
///
/// Shared with [`super::stream_actions`], whose actions can end the loop.
pub(super) enum Step {
    /// Keep reading frames.
    Continue,
    /// Stop dispatching: the peer closed, or a reply left its stream open.
    Closed,
    /// The connection must end with this error.
    Failed(io::Error),
}

/// One connected transport and the immutable inputs that drive it.
pub(super) struct Connection<'a, S, E, R, K, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    /// Read half of the framed transport.
    pub(super) reader: &'a mut R,
    /// Write half of the framed transport.
    pub(super) sink: &'a mut K,
    /// Inbound framing state, including the frame buffered while the previous
    /// one was handled.
    pub(super) inbound: &'a mut InboundFrames<F::Frame>,
    /// Actor that schedules and delivers every reply.
    pub(super) actor: &'a mut ConnectionActor<Envelope, ()>,
    /// Immutable connection inputs.
    pub(super) ctx: &'a DispatchContext<'a, S, E, F>,
    /// Server shutdown signal; cancelling it stops this connection.
    pub(super) shutdown: &'a CancellationToken,
    /// Connection-wide token cancelled only by a clean peer EOF.
    pub(super) peer_eof: &'a CancellationToken,
    /// Maximum wait for a handler or a body send while teardown runs.
    pub(super) wait: Duration,
}

/// Read and process frames until the stream closes or an I/O error occurs.
///
/// The connection is taken by value: the loop owns the transport, the framing
/// state and the actor for the whole exchange, so there is nothing left for the
/// caller to use once it returns.
pub(super) async fn run<S, E, F, R, K>(
    mut connection: Connection<'_, S, E, R, K, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let mut state = LoopState::new(QueuedBytes::new());
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
        // Shutdown is observed here, not only at the read boundary. A pending
        // chunk admission and a parked reply both keep the loop off the reader,
        // so a cancelled token would otherwise never be seen while either one
        // retries, and the connection would outlive the server's drain.
        if self.shutdown.is_cancelled() {
            state.streams.abort();
            return Ok(Step::Closed);
        }
        let queued = state.streams.queued();
        {
            let mut handles = self.handles(queued.clone());
            if expire_finished_assemblies(state, &mut handles).await? == Dispatch::Stop {
                return Ok(Step::Closed);
            }
            apply_memory_pressure(state, &mut handles).await?;
            // A pending admission means the slot is live, so the parked drain
            // below cannot run in this step. Deciding it first keeps the drain
            // outside this borrow scope, where the reader stays reachable.
            if state.streams.has_pending_send() {
                return drive_pending_admission(state, &mut handles).await;
            }
        }
        if !state.streams.is_live()
            && self
                .dispatch_observed(state, queued.clone(), ObservedAction::Drain)
                .await?
                == Dispatch::Stop
        {
            return Ok(Step::Closed);
        }
        let event = self.next_event(state).await;
        self.handle_event(state, event, queued).await
    }

    /// Run one reply-producing action while watching the peer's read half.
    ///
    /// Both actions here write a reply that may stream for an unbounded time,
    /// so both poll the reader alongside themselves. Without that, a peer that
    /// half-closed is never observed, the actor's bounded peer-close drain is
    /// never armed, and a producer that stops yielding keeps the connection
    /// alive with nobody left to answer.
    async fn dispatch_observed(
        &mut self,
        state: &mut LoopState,
        queued: QueuedBytes,
        action: ObservedAction<'_, F::Frame>,
    ) -> io::Result<Dispatch> {
        // Built from field borrows rather than through `handles`, because the
        // reader and its buffered frame must stay reachable for the action.
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
        let observation = PeerObservation::new(self.peer_eof, &unobserved, &cause);
        let pending = &mut self.inbound.pending;
        match action {
            ObservedAction::Frame(frame) => {
                handle_watching_peer(
                    dispatch_frame(frame, state, handles),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
            ObservedAction::Drain => {
                handle_watching_peer(
                    drain_parked(state, &mut handles),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
        }
    }

    /// Build the action-scoped handle bundle from disjoint connection borrows.
    ///
    /// The bundle borrows the two mutable pieces of inbound framing rather
    /// than the whole value, so the reader and its buffered frame stay
    /// reachable for the duration of the action.
    const fn handles(&mut self, queued: QueuedBytes) -> DispatchHandles<'_, S, E, K, F> {
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
            Event::Frame(Ok(Some(frame))) => {
                let dispatch = self
                    .dispatch_observed(state, queued, ObservedAction::Frame(&frame))
                    .await?;
                Ok(finish_step(dispatch))
            }
            Event::Frame(Ok(None)) => {
                // The read half ended, which is permanent for the connection.
                // Record it before the parked messages drain, so each of their
                // replies is bounded by the actor's peer-close grace instead of
                // waiting on a producer nobody can interrupt.
                self.peer_eof.cancel();
                {
                    let mut handles = self.handles(queued.clone());
                    // A body cut short by the peer's EOF cannot complete, so it
                    // fails with the same reason a truncated upload reports.
                    let reason = io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed before the request body completed",
                    );
                    fail_body_ending_with(state, &mut handles, reason, Step::Closed).await?;
                }
                self.dispatch_observed(state, queued, ObservedAction::Drain)
                    .await?;
                Ok(Step::Closed)
            }
            Event::Frame(Err(error)) => {
                let mut handles = self.handles(queued);
                fail_after_decode_error(state, &mut handles, error).await
            }
        }
    }
}

/// One reply-producing action that must not hide a peer that went away.
enum ObservedAction<'a, T> {
    /// Dispatch one inbound frame.
    Frame(&'a T),
    /// Answer the messages parked behind a live request body.
    Drain,
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
