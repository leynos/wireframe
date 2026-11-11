//! Continuing a response after the peer can no longer be observed.
//!
//! A peer may close only its sending half and keep reading, so end-of-file must
//! not truncate a healthy stream. The inbound loop also stops observing the
//! peer once it buffers a frame for the next request. In either case the peer
//! may equally have vanished, and an idle producer would then keep the
//! connection, its gauge guard, and the server drain pending forever. Each wait
//! and each write is therefore bounded by the actor's idle grace.

use std::sync::Arc;

use super::{
    ConnectionActor,
    FrameSink,
    drive::ResponseOutcome,
    event::Event,
    grace::GraceFuture,
    state::ActorState,
    writes::{WriteOutcome, flush_frames},
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
    response::WireframeError,
};

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Continue a response once the peer can no longer be observed.
    ///
    /// A peer may close only its sending half and keep reading, so EOF must not
    /// truncate a healthy stream. The inbound loop also stops observing the
    /// peer when it buffers a frame for the next request. In either case the
    /// peer may equally have vanished, and an idle producer would then keep the
    /// connection, its gauge guard, and the server drain pending forever.
    /// Bounding the wait for each event lets progress complete the response
    /// while guaranteeing teardown after a stall. The bound comes from the
    /// actor's [`GraceTimer`](super::grace::GraceTimer), so production uses the
    /// ambient clock and a test can fire the deadline by hand.
    pub(super) async fn drive_after_peer_close<S>(
        &mut self,
        sink: &mut S,
    ) -> Result<ResponseOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
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
                event = self.next_event(&state) => event,
                () = grace.as_mut() => {
                    self.cancel_stalled_response();
                    return Ok(ResponseOutcome::Cancelled);
                }
            };
            if matches!(event, Event::Shutdown) {
                ResponseCancellationReason::Shutdown.record();
                return Ok(ResponseOutcome::Cancelled);
            }
            let mut frames = Vec::new();
            self.dispatch_event(event, &mut state, &mut frames)?;
            if self
                .write_after_peer_close(frames, sink, &mut grace)
                .await?
                == WriteOutcome::Cancelled
            {
                return Ok(ResponseOutcome::Cancelled);
            }
            self.run_deferred_command_end();
        }
        Ok(self.finish_outcome())
    }

    /// Write one event's frames, bounded by the idle grace and by shutdown.
    ///
    /// The same grace that bounded the wait for the event also bounds the
    /// write. A peer that stopped reading applies back-pressure that can hold a
    /// send pending forever, so bounding only the wait for the next event would
    /// still let a vanished peer pin the connection, its gauge, and the drain.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the sink fails.
    async fn write_after_peer_close<S>(
        &mut self,
        frames: Vec<F>,
        sink: &mut S,
        grace: &mut GraceFuture,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        let shutdown = self.shutdown.clone();
        tokio::select! {
            biased;
            // A ready write is preferred so that a cancellation observed while
            // frames are still being accepted does not abandon queued output.
            // Only a write that would block is raced against the bounds.
            result = flush_frames(frames, sink) => result.map(|()| WriteOutcome::Written),
            () = shutdown.cancelled() => {
                ResponseCancellationReason::Shutdown.record();
                Ok(WriteOutcome::Cancelled)
            }
            () = grace.as_mut() => {
                self.cancel_stalled_response();
                Ok(WriteOutcome::Cancelled)
            }
        }
    }
}
