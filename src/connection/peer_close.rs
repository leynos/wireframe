//! Continuing a response under the actor's idle grace.
//!
//! A peer may close only its sending half and keep reading, so end-of-file must
//! not truncate a healthy stream. The inbound loop also stops observing the
//! peer once it buffers a frame for the next request. In either case the peer
//! may equally have vanished, and an idle producer would then keep the
//! connection, its gauge guard, and the server drain pending forever. Each wait
//! and each write is therefore bounded by the actor's idle grace.
//!
//! The same bounded drive carries a response across a server `stop()` when the
//! connection must not abandon it: progress still completes the response, and
//! only a producer that stops yielding is cancelled.

use std::sync::Arc;

use super::{
    ConnectionActor,
    FrameSink,
    PushSink,
    drain::ActorStep,
    drive::ResponseOutcome,
    event::{Event, EventWatch},
    state::ActorState,
    writes::{self, WriteOutcome},
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
    response::WireframeError,
};

/// Output produced by one loop iteration, split by frame kind.
///
/// Responses are application frames the sink serializes; pushes arrive already
/// encoded for the wire. Keeping them apart lets the actor flush pushes first
/// without conflating the two frame types.
struct PendingOutput<F, P> {
    /// Response frames produced by the event.
    frames: Vec<F>,
    /// Push frames produced by the event.
    pushes: Vec<P>,
}

impl<F, P> PendingOutput<F, P> {
    /// Start an empty batch for one loop iteration.
    const fn new() -> Self {
        Self {
            frames: Vec::new(),
            pushes: Vec::new(),
        }
    }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Continue a response under the idle grace.
    ///
    /// The bounded drain is entered in two situations. The peer may no longer
    /// be observable: it closed only its sending half and kept reading, the
    /// inbound loop buffered a frame and stopped polling the read half, or it
    /// vanished entirely. A server connection may also have observed
    /// `stop()` while it must still deliver a response its peer is reading.
    ///
    /// In every case an idle producer or a stalled write would keep the
    /// connection, its gauge guard, and the server drain pending forever.
    /// Bounding the wait for each event lets progress complete the response
    /// while guaranteeing teardown after a stall. The bound comes from the
    /// actor's [`GraceTimer`](super::grace::GraceTimer), so production uses the
    /// ambient clock and a test can fire the deadline by hand.
    pub(super) async fn drive_bounded_drain<S>(
        &mut self,
        sink: &mut S,
    ) -> Result<ResponseOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + PushSink<P> + Send,
    {
        while self.active_output.is_response() || self.active_output.is_multi_packet() {
            let mut state = ActorState::new(
                self.active_output.is_response(),
                self.active_output.is_multi_packet(),
            );
            let timer = Arc::clone(&self.grace_timer);
            let mut grace = timer.elapse(self.peer_closed_grace);
            let event = tokio::select! {
                biased;
                // The peer is no longer observed here, and each wait is already
                // bounded below, so only the drain policy decides whether a
                // shutdown still preempts the wait.
                event = self.next_event_in_bounded_drain(&state) => event,
                () = grace.as_mut() => {
                    self.cancel_stalled_response();
                    return Ok(ResponseOutcome::Cancelled);
                }
            };
            if matches!(event, Event::Shutdown) {
                ResponseCancellationReason::Shutdown.record();
                return Ok(ResponseOutcome::Cancelled);
            }
            let mut output = PendingOutput::new();
            let mut step = ActorStep {
                state: &mut state,
                out: &mut output.frames,
                pushes: &mut output.pushes,
            };
            self.dispatch_event(event, &mut step)?;
            if self.write_after_peer_close(output, sink).await? == WriteOutcome::Cancelled {
                return Ok(ResponseOutcome::Cancelled);
            }
            self.run_deferred_command_end();
        }
        Ok(self.finish_outcome())
    }

    /// Await the next event for a prepared connection while it drains.
    ///
    /// The prepared connection keeps driving a response whose peer is reading
    /// even after `stop()`, so that ADR-014's graceful drain still delivers it
    /// and runs its completion hook. Its writes remain bounded, which is what
    /// stops a peer that no longer reads from pinning the drain.
    ///
    /// Shutdown is observed even when the response must not be abandoned. The
    /// driver decides what that event means: a stop, or a switch to the bounded
    /// drain that only abandons a producer which stops yielding. Without
    /// observing it, a producer that yields nothing would keep this wait
    /// pending past `stop()` and hold the connection, its gauge and the server
    /// drain open for as long as the peer stays connected.
    pub(super) async fn next_event_while_draining(&mut self, state: &ActorState) -> Event<F, E, P> {
        let watch = EventWatch {
            peer: true,
            shutdown: true,
        };
        self.next_event_inner(state, watch).await
    }

    /// Await the next event in the bounded drain.
    ///
    /// Each wait there is already bounded by the idle grace, so shutdown is
    /// observed only when it must abandon the response. Watching a cancelled
    /// token unconditionally would make the biased select return immediately on
    /// every iteration and spin the loop.
    pub(super) async fn next_event_in_bounded_drain(
        &mut self,
        state: &ActorState,
    ) -> Event<F, E, P> {
        let watch = EventWatch {
            peer: false,
            shutdown: self.abandon_response_on_shutdown,
        };
        self.next_event_inner(state, watch).await
    }

    /// Write one event's output, bounded by the idle grace and by shutdown.
    ///
    /// A peer that stopped reading applies back-pressure that can hold a send
    /// pending forever, so bounding only the wait for the next event would still
    /// let a vanished peer pin the connection, its gauge, and the drain. Each
    /// frame is bounded separately, and each bound is fresh: accepting a frame
    /// is progress, so a peer that keeps reading must not be abandoned merely
    /// because writing the whole batch takes longer than one grace.
    ///
    /// Queued pushes are written before the response frames produced with them,
    /// matching the actor's own priority ordering.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the sink fails.
    async fn write_after_peer_close<S>(
        &mut self,
        output: PendingOutput<F, P>,
        sink: &mut S,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + PushSink<P> + Send,
    {
        let PendingOutput { frames, pushes } = output;
        let progress = sink.progress();
        let lanes = pushes
            .into_iter()
            .map(writes::Output::Push)
            .chain(frames.into_iter().map(writes::Output::Frame));
        for item in lanes {
            let outcome = self
                .write_output_after_peer_close(progress.clone(), item, sink)
                .await?;
            if outcome == WriteOutcome::Cancelled {
                return Ok(WriteOutcome::Cancelled);
            }
        }
        Ok(WriteOutcome::Written)
    }

    /// Write one piece of buffered output under the peer-close grace.
    async fn write_output_after_peer_close<S>(
        &mut self,
        progress: Option<Arc<super::ProgressSignal>>,
        item: writes::Output<F, P>,
        sink: &mut S,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + PushSink<P> + Send,
    {
        let limit = {
            let bounds = self.write_bounds();
            match item {
                writes::Output::Push(push) => {
                    let mut pinned = std::pin::pin!(sink.send_push(push));
                    writes::write_within_grace(pinned.as_mut(), &bounds, progress.as_deref()).await
                }
                writes::Output::Frame(frame) => {
                    let mut pinned = std::pin::pin!(sink.send_frame(frame));
                    writes::write_within_grace(pinned.as_mut(), &bounds, progress.as_deref()).await
                }
            }
            .map_err(WireframeError::Io)?
        };
        Ok(self.record_write_limit(limit))
    }
}
