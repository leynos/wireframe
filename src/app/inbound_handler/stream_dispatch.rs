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

use std::{io, time::Duration};

use futures::{Sink, Stream};
use tokio_util::sync::CancellationToken;

// Re-exported so the loop's callers keep naming these through the module that
// owns the state machine, while their definitions stay in `stream_types`.
pub(super) use super::stream_types::{AdmissionWait, DispatchContext, Event, Step};
use super::{
    super::{
        PushWriteDiagnostics,
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
    },
    frames::{InboundExpiry, InboundFrames, NextFrame},
    push_service::{PushBatch, PushTransport, deliver_pushes, drain_queued},
    stream_actions::{apply_memory_pressure, expire_finished_assemblies, finish_step},
    stream_diagnostics,
    stream_handles::{DispatchHandles, LoopState},
    stream_observed::ObservedAction,
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PendingPushes},
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    request::QueuedBytes,
    serializer::Serializer,
};

#[cfg(test)]
#[path = "stream_watch_support.rs"]
mod watch_support;

#[cfg(test)]
#[path = "stream_admission_watch_tests.rs"]
mod admission_watch_tests;

#[cfg(test)]
#[path = "stream_watch_tests.rs"]
mod watch_tests;

#[cfg(test)]
#[path = "stream_reply_watch_tests.rs"]
mod reply_watch_tests;

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
    pub(super) actor: &'a mut ConnectionActor<Envelope, (), F::Frame>,
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
        if self.expire_bodies(state, queued.clone()).await? {
            return Ok(Step::Closed);
        }
        match self.drive_admission(state, queued.clone()).await? {
            AdmissionWait::Handler(outcome) => {
                // The handler's reply may stream for an unbounded time, so it is
                // forwarded through the same peer-observed path as a frame's
                // reply.
                let dispatch = self
                    .dispatch_observed(state, queued, ObservedAction::Handler(outcome))
                    .await?;
                return Ok(finish_step(dispatch));
            }
            // The read half settled the step, so it is handled exactly as an
            // event read outside an admission would be.
            AdmissionWait::Peer(event) => return self.handle_event(state, event, queued).await,
            AdmissionWait::Continue => {}
        }
        if state.streams.has_pending_send() {
            // A chunk still waiting for admission cannot be joined by another:
            // `push_chunk` refuses it, and the refusal is indistinguishable
            // from an abandoned body. Return to the top instead, where shutdown
            // is re-checked, expiry and the memory budget run again, and the
            // admission is driven against a freshly measured budget.
            return Ok(Step::Continue);
        }
        if self.drain_parked(state, queued.clone()).await? {
            return Ok(Step::Closed);
        }
        let event = self.next_event(state).await;
        self.handle_event(state, event, queued).await
    }

    /// Apply the memory budget, then act on any assembly that has expired.
    ///
    /// Purged inside the handle scope and acted on after it: the reply an
    /// expired body produces may stream for an unbounded time, so it is written
    /// through the peer-observed path where the reader stays reachable.
    ///
    /// Returns `true` when the connection must stop.
    async fn expire_bodies(
        &mut self,
        state: &mut LoopState,
        queued: QueuedBytes,
    ) -> io::Result<bool> {
        let expired = {
            let mut handles = self.handles(queued.clone());
            apply_memory_pressure(state, &mut handles).await?;
            expire_finished_assemblies(&mut handles)
        };
        if expired.is_empty() {
            return Ok(false);
        }
        let dispatch = self
            .dispatch_observed(state, queued, ObservedAction::Expire(expired))
            .await?;
        Ok(dispatch == Dispatch::Stop)
    }

    /// Dispatch parked messages once the live slot is free.
    ///
    /// Returns `true` when the connection must stop.
    async fn drain_parked(
        &mut self,
        state: &mut LoopState,
        queued: QueuedBytes,
    ) -> io::Result<bool> {
        if state.streams.is_live() {
            return Ok(false);
        }
        let dispatch = self
            .dispatch_observed(state, queued, ObservedAction::Drain)
            .await?;
        Ok(dispatch == Dispatch::Stop)
    }

    /// Build the action-scoped handle bundle from disjoint connection borrows.
    ///
    /// The bundle borrows the two mutable pieces of inbound framing rather
    /// than the whole value, so the reader and its buffered frame stay
    /// reachable for the duration of the action.
    pub(super) const fn handles(&mut self, queued: QueuedBytes) -> DispatchHandles<'_, S, E, K, F> {
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

    /// Await the next unit of work for this connection.
    ///
    /// Three sources are raced in one biased select: queued pushes, the live
    /// handler resolving, and the inbound read. Pushes win ties, matching the
    /// connection actor's own priority ordering, so a queued push is not held
    /// behind a client that keeps its socket busy. Two rules bound that bias: a
    /// frame buffered during dispatch is served first, and after each push
    /// batch the reader gets one non-blocking attempt, so an inbound frame
    /// waits behind at most one batch.
    async fn next_event(&mut self, state: &mut LoopState) -> Event<F::Frame> {
        let read_timeout_ms = self.ctx.read_timeout_ms;
        let shutdown = self.shutdown;
        // The probe is the reader's turn, so it is spent whether or not it
        // produces a frame.
        if std::mem::take(&mut state.serve_read_first) {
            return match self.inbound.try_next(self.reader).await {
                Ok(NextFrame::TimedOut) => self.await_event(state, read_timeout_ms).await,
                Ok(next) => frame_event(next, shutdown),
                Err(error) => Event::Frame(Err(error)),
            };
        }
        self.await_event(state, read_timeout_ms).await
    }

    /// Race the push lanes, the live handler, and the inbound read.
    ///
    /// The reader, the sink, and the actor are borrowed separately here, so a
    /// push lane and the read half are polled by the same select without
    /// borrowing the whole connection.
    async fn await_event(
        &mut self,
        state: &mut LoopState,
        read_timeout_ms: u64,
    ) -> Event<F::Frame> {
        let Connection {
            reader,
            inbound,
            actor,
            shutdown,
            ..
        } = self;
        let shutdown = *shutdown;
        let push_lanes_open = actor.has_open_push_lanes();
        let read = async {
            match inbound.next_frame(reader, read_timeout_ms, shutdown).await {
                Ok(next) => frame_event(next, shutdown),
                Err(error) => Event::Frame(Err(error)),
            }
        };
        if let Some(active) = state.streams.active.as_mut() {
            tokio::select! {
                biased;
                pushes = actor.next_pushes(), if push_lanes_open => push_event(pushes),
                outcome = active.poll_handler() => Event::Handler(outcome),
                event = read => event,
            }
        } else {
            tokio::select! {
                biased;
                pushes = actor.next_pushes(), if push_lanes_open => push_event(pushes),
                event = read => event,
            }
        }
    }

    /// Read the next inbound frame and classify it for the loop.
    pub(super) async fn read_frame(&mut self, read_timeout_ms: u64) -> Event<F::Frame> {
        let read = self
            .inbound
            .next_frame(self.reader, read_timeout_ms, self.shutdown)
            .await;
        match read {
            Ok(next) => frame_event(next, self.shutdown),
            Err(error) => Event::Frame(Err(error)),
        }
    }

    /// Write one batch of queued pushes through the connection's transport.
    ///
    /// A write to a peer that stopped reading can stay pending indefinitely, so
    /// it is bounded by the shutdown token and, once the peer can no longer be
    /// observed, by the idle grace. The read half keeps being polled while the
    /// write runs, because this is the last place that can notice a peer which
    /// half-closed and then stopped reading.
    ///
    /// The write also keeps the inbound read deadline expiring, and an expired
    /// streaming assembly is failed here rather than inside the write: the
    /// write holds the reader, so the reply a failed body owes can only be
    /// forwarded once the batch has been written.
    async fn write_pushes(
        &mut self,
        state: &mut LoopState,
        pushes: PendingPushes<F::Frame>,
        queued: QueuedBytes,
    ) -> io::Result<Step> {
        let read_timeout_ms = self.ctx.read_timeout_ms;
        let serializer = self.ctx.serializer;
        let codec = self.ctx.codec;
        let shutdown = self.shutdown;
        let peer_eof = self.peer_eof;
        let mut evicted = Vec::new();
        {
            let Connection {
                reader,
                sink,
                inbound,
                actor,
                ..
            } = self;
            let mut transport = PushTransport {
                serializer,
                codec,
                sink,
                pipeline: &mut inbound.pipeline,
                diagnostics: push_diagnostics::<F>(&**actor),
            };
            let batch = PushBatch {
                frames: pushes.frames,
                priority: pushes.priority,
                reader,
                pending_frame: &mut inbound.pending,
                expiry: InboundExpiry::new(
                    &mut inbound.message_assembly,
                    &mut inbound.read_deadline,
                    read_timeout_ms,
                    &mut evicted,
                ),
                peer_eof,
                shutdown,
            };
            deliver_pushes(&mut transport, &mut **actor, batch).await?;
        }
        // The reader is owed the next pass, so a continuously ready push lane
        // cannot win every biased select and starve the client's frames.
        state.serve_read_first = true;
        if evicted.is_empty() {
            return Ok(Step::Continue);
        }
        let dispatch = self
            .dispatch_observed(state, queued, ObservedAction::Expire(evicted))
            .await?;
        Ok(finish_step(dispatch))
    }

    /// Deliver frames still queued when the peer stopped writing.
    async fn drain_pushes(&mut self) -> io::Result<()> {
        let serializer = self.ctx.serializer;
        let codec = self.ctx.codec;
        let shutdown = self.shutdown;
        let Connection {
            sink,
            inbound,
            actor,
            ..
        } = self;
        let mut transport = PushTransport {
            serializer,
            codec,
            sink,
            pipeline: &mut inbound.pipeline,
            diagnostics: push_diagnostics::<F>(&**actor),
        };
        drain_queued(&mut transport, &mut **actor, shutdown).await
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
            Event::Pushes(pushes) => self.write_pushes(state, pushes, queued).await,
            Event::LanesIdle => {
                // Every push handle has been dropped, so only the inbound read
                // can produce work from here on.
                Ok(Step::Continue)
            }
            Event::Handler(outcome) => {
                // Forwarding the reply writes to the transport, which can block
                // for as long as the peer declines to read, so the peer's read
                // half stays observed throughout.
                let dispatch = self
                    .dispatch_observed(state, queued, ObservedAction::Handler(outcome))
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
                // A body cut short by the peer's EOF cannot complete, so it
                // fails with the same reason a truncated upload reports.
                let reason = io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed before the request body completed",
                );
                // A reply cancelled while it was being written leaves a partial
                // frame behind, so parked messages must not be served on that
                // connection: each would be read as part of the broken stream.
                if self
                    .dispatch_observed(state, queued.clone(), ObservedAction::FailBody(reason))
                    .await?
                    == Dispatch::Stop
                {
                    return Ok(Step::Closed);
                }
                // The read half has ended, so the loop closes however the drain
                // finished; its outcome only decides whether the writes below it
                // were still permitted.
                let dispatch = self
                    .dispatch_observed(state, queued, ObservedAction::Drain)
                    .await?;
                // Frames already queued are still delivered; anything pushed
                // afterwards is rejected because the actor closes both lanes.
                self.drain_pushes().await?;
                match dispatch {
                    Dispatch::Continue | Dispatch::Stop => Ok(Step::Closed),
                }
            }
            Event::Frame(Err(error)) => {
                let reason = io::Error::new(
                    error.kind(),
                    "inbound frame decoding failed while a request body was open",
                );
                // The decode error ends the connection whatever the body's reply
                // did, so the delivery outcome cannot change the decision. The
                // reply is still forwarded through the observed path, because it
                // may stream for an unbounded time.
                self.dispatch_observed(state, queued, ObservedAction::FailBody(reason))
                    .await?;
                Ok(Step::Failed(error))
            }
        }
    }
}

/// Classify one read outcome for the loop.
///
/// A shutdown that arrives during the read closes it, so it is reported as a
/// stop rather than as a peer EOF: the peer may still be connected.
fn frame_event<T>(next: NextFrame<T>, shutdown: &CancellationToken) -> Event<T> {
    match next {
        NextFrame::Ready(frame) => Event::Frame(Ok(Some(frame))),
        NextFrame::Closed if shutdown.is_cancelled() => Event::Shutdown,
        NextFrame::Closed => Event::Frame(Ok(None)),
        NextFrame::TimedOut => Event::Tick,
    }
}

/// Classify one push-lane poll for the loop.
fn push_event<T>(pushes: Option<PendingPushes<T>>) -> Event<T> {
    pushes.map_or(Event::LanesIdle, Event::Pushes)
}

/// Identity this connection reports when it abandons a push write.
fn push_diagnostics<F>(actor: &ConnectionActor<Envelope, (), F::Frame>) -> PushWriteDiagnostics
where
    F: FrameCodec,
{
    PushWriteDiagnostics::new(actor.connection_id(), actor.peer_addr())
}
