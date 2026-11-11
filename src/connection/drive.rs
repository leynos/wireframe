//! Sink-driven execution loop for the connection actor.
//!
//! Every frame is composed by the actor and then written through an awaited
//! [`FrameSink`]. Buffering per event keeps the standalone `Vec` output order
//! identical to the historical synchronous driver while letting an integrated
//! transport apply back-pressure before the next source is polled.

use std::{io, time::Duration};

use log::info;

use super::{
    ConnectionActor,
    FrameSink,
    event::Event,
    output::{ActiveOutput, AvailableOutput, EventAvailability},
    polling::EventSources,
    state::ActorState,
    writes::{WriteOutcome, flush_frames},
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
    response::{Response, WireframeError},
};

/// Idle grace granted to a response once the peer can no longer be observed.
///
/// The peer is unobservable after it closes its input half, and after an
/// inbound frame is buffered for the next request so that the read half stops
/// being polled. Long enough for a producer that is merely slow to deliver its
/// next frame, and short enough that a vanished peer cannot pin a connection
/// open indefinitely.
pub(crate) const PEER_CLOSED_RESPONSE_GRACE: Duration = Duration::from_secs(30);

/// How an installed handler response finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseOutcome {
    /// The response completed, so its protocol terminator may be emitted.
    Completed,
    /// The response stream reported a protocol error.
    Failed,
    /// The connection shut down or the peer disappeared before completion.
    Cancelled,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Drive the actor until all sources are exhausted or shutdown is triggered.
    ///
    /// Frames are appended to `out` in the order they are processed.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the response stream yields an I/O error.
    pub async fn run(&mut self, out: &mut Vec<F>) -> Result<(), WireframeError<E>> {
        self.run_with_sink(out).await
    }

    /// Drive the actor, awaiting `sink` for every emitted frame.
    ///
    /// Frames produced by a single loop iteration are buffered, then delivered
    /// in order. Awaiting the sink before polling the next event is what
    /// transmits back-pressure from a bounded transport to the actor's
    /// sources.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if a source or the sink reports an I/O
    /// error.
    pub(crate) async fn run_with_sink<S>(&mut self, sink: &mut S) -> Result<(), WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        // If cancellation has already been requested, exit immediately. Nothing
        // will be drained and any streaming response is abandoned. This mirrors
        // a hard shutdown and is required for the tests.
        if self.shutdown.is_cancelled() {
            info!(
                "connection aborted before start: id={:?}, peer={:?}",
                self.connection_id, self.peer_addr
            );
            let _ = self.counter.take();
            return Ok(());
        }

        let mut state = ActorState::new(
            self.active_output.is_response(),
            self.active_output.is_multi_packet(),
        );

        while !state.is_done() {
            let event = self.next_event(&state).await;
            let mut frames = Vec::new();
            self.dispatch_event(event, &mut state, &mut frames)?;
            if self.flush_or_cancel(frames, sink).await? == WriteOutcome::Cancelled {
                info!(
                    "connection aborted during write: id={:?}, peer={:?}",
                    self.connection_id, self.peer_addr
                );
                let _ = self.counter.take();
                return Ok(());
            }
            self.run_deferred_command_end();
        }
        info!(
            "connection closed: id={:?}, peer={:?}",
            self.connection_id, self.peer_addr
        );
        let _ = self.counter.take();
        Ok(())
    }

    /// Install a handler response and drive it to completion.
    ///
    /// `Single`, `Vec`, and `Empty` responses complete immediately: they are
    /// emitted without a stream terminator and `on_command_end` runs once.
    /// `Stream` and `MultiPacket` reuse the existing setters, so the usual
    /// mutual-exclusion rules apply, and are then driven by
    /// [`drive_response`](Self::drive_response).
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::Io`] when another output is already active or
    /// the sink fails.
    pub(crate) async fn install_response<S>(
        &mut self,
        response: Response<F, E>,
        correlation_id: Option<u64>,
        sink: &mut S,
    ) -> Result<ResponseOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        // Diagnostics for a stalled response report the request it belonged to.
        self.response_correlation = correlation_id;
        match response {
            Response::Single(frame) => {
                self.emit_response_frames(vec![frame], correlation_id, sink)
                    .await?;
                self.hooks.on_command_end(&mut self.ctx);
                Ok(ResponseOutcome::Completed)
            }
            Response::Vec(frames) => {
                self.emit_response_frames(frames, correlation_id, sink)
                    .await?;
                self.hooks.on_command_end(&mut self.ctx);
                Ok(ResponseOutcome::Completed)
            }
            Response::Empty => {
                ensure_output_idle(&self.active_output)?;
                self.hooks.on_command_end(&mut self.ctx);
                Ok(ResponseOutcome::Completed)
            }
            Response::Stream(stream) => {
                self.response_failed = false;
                self.set_response(Some(stream))
                    .map_err(output_state_error)?;
                self.drive_response(sink).await
            }
            Response::MultiPacket(receiver) => {
                self.response_failed = false;
                self.set_multi_packet_with_correlation(Some(receiver), correlation_id)
                    .map_err(output_state_error)?;
                self.drive_response(sink).await
            }
        }
    }

    /// Drive the active response output until it completes.
    ///
    /// The biased select loop and fairness rules are retained, so urgent
    /// pushes and multi-packet frames are still serviced while a stream runs.
    /// Returns when the response completes, shutdown is observed, or the sink
    /// fails. It deliberately leaves push receivers open and the actor's
    /// connection-lifetime state intact, because the surrounding connection
    /// loop decides when the connection ends.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] from the response stream or the sink.
    pub(crate) async fn drive_response<S>(
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
            let event = self.next_event_watching_peer(&state).await;
            if matches!(event, Event::Shutdown) {
                ResponseCancellationReason::Shutdown.record();
                return Ok(ResponseOutcome::Cancelled);
            }
            if matches!(event, Event::PeerClosed) {
                return self.drive_after_peer_close(sink).await;
            }
            if let Some(outcome) = self.dispatch_and_write(event, &mut state, sink).await? {
                return Ok(outcome);
            }
        }
        Ok(self.finish_outcome())
    }

    /// Map the response loop's terminal flags to an outcome.
    pub(super) const fn finish_outcome(&self) -> ResponseOutcome {
        if self.response_failed {
            ResponseOutcome::Failed
        } else {
            ResponseOutcome::Completed
        }
    }

    /// Record that the protocol's completion hook belongs after the final
    /// write.
    ///
    /// Streamed responses announce completion while the terminator frame is
    /// still buffered. Deferring the hook until
    /// [`run_deferred_command_end`](Self::run_deferred_command_end) runs keeps
    /// `on_command_end` after a successful write, matching the ordering of
    /// single-frame responses, and suppresses it entirely when the write
    /// fails.
    pub(super) const fn defer_command_end(&mut self) { self.command_end_pending = true; }

    /// Run a deferred completion hook once the final frame has been written.
    pub(super) fn run_deferred_command_end(&mut self) {
        if std::mem::take(&mut self.command_end_pending) {
            self.hooks.on_command_end(&mut self.ctx);
        }
    }

    /// Emit a completed response, stamping the request correlation ID.
    ///
    /// A frame that already carries an identifier keeps it when the request
    /// has none, matching [`CorrelatableFrame`] inheritance elsewhere: a
    /// middleware-provided response id must not be cleared by a request that
    /// simply never supplied one.
    async fn emit_response_frames<S>(
        &mut self,
        frames: Vec<F>,
        correlation_id: Option<u64>,
        sink: &mut S,
    ) -> Result<(), WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        ensure_output_idle(&self.active_output)?;
        let mut out = Vec::with_capacity(frames.len());
        for mut frame in frames {
            if let Some(request_correlation) = correlation_id {
                frame.set_correlation_id(Some(request_correlation));
            }
            self.process_frame_with_hooks_and_metrics(frame, &mut out);
        }
        flush_frames(out, sink).await
    }

    /// Compute which event sources are currently available for polling.
    const fn compute_availability(&self, state: &ActorState) -> EventAvailability {
        EventAvailability {
            high: self.high_rx.is_some(),
            low: self.low_rx.is_some(),
            output: self
                .active_output
                .available_output(state.is_shutting_down()),
        }
    }

    /// Await the next ready event using biased priority ordering.
    ///
    /// Shutdown is observed first, followed by high-priority pushes, then
    /// low-priority pushes, multi-packet channels, and finally the response
    /// stream. This matches the design documentation. The final `else` branch
    /// prevents `tokio::select!` from panicking if all guards are false.
    ///
    /// The `strict_priority_order` and `shutdown_signal_precedence` tests
    /// assert that this ordering is preserved across refactors.
    pub(super) async fn next_event(&mut self, state: &ActorState) -> Event<F, E> {
        self.next_event_inner(state, false).await
    }

    /// Await the next event while also observing the peer's half-close.
    async fn next_event_watching_peer(&mut self, state: &ActorState) -> Event<F, E> {
        self.next_event_inner(state, true).await
    }

    /// Whether peer-close observation applies to the current actor state.
    const fn should_watch_peer(watch_peer: bool, state: &ActorState) -> bool {
        watch_peer && state.is_active()
    }

    /// Shared select loop for both observation modes.
    async fn next_event_inner(&mut self, state: &ActorState, watch_peer: bool) -> Event<F, E> {
        let avail = self.compute_availability(state);
        let can_poll_multi_packet = avail.output == AvailableOutput::MultiPacket;
        let can_poll_response = avail.output == AvailableOutput::Response;
        let EventSources {
            high,
            low,
            multi_packet,
            response,
            shutdown,
            peer_closed,
        } = self.event_sources();

        tokio::select! {
            biased;

            () = Self::wait_shutdown(shutdown), if state.is_active() => Event::Shutdown,
            () = Self::wait_peer_closed(peer_closed), if Self::should_watch_peer(watch_peer, state) => {
                Event::PeerClosed
            }
            res = Self::poll_queue(high), if avail.high => Event::High(res),
            res = Self::poll_queue(low), if avail.low => Event::Low(res),
            res = Self::poll_queue(multi_packet), if can_poll_multi_packet => Event::MultiPacket(res),
            res = Self::poll_response(response), if can_poll_response => Event::Response(res),
            else => Event::Idle,
        }
    }
}

/// Reject a completed response while another output source is active.
fn ensure_output_idle<F, E>(output: &ActiveOutput<F, E>) -> Result<(), WireframeError<E>> {
    if output.is_response() || output.is_multi_packet() {
        return Err(WireframeError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot install a response while another response is active",
        )));
    }
    Ok(())
}

/// Translate the public state error into the actor's I/O error channel.
fn output_state_error<E>(error: super::ConnectionStateError) -> WireframeError<E> {
    WireframeError::Io(io::Error::new(io::ErrorKind::InvalidInput, error))
}
