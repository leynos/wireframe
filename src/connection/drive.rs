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
    PushSink,
    drain::ActorStep,
    event::Event,
    output::ActiveOutput,
    state::ActorState,
    writes::WriteOutcome,
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

/// How an event ends a response's unbounded wait.
///
/// A response is driven first without a deadline, because a producer is allowed
/// to take as long as it needs while the connection is healthy. Only these two
/// events end that wait, and each continues the response differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResponseInterruption {
    /// Abandon the response without a terminator, because shutdown preempts it.
    Abandon,
    /// Continue under the bounded drain, which renews the idle grace per event.
    Bounded,
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
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
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
        S: FrameSink<F> + PushSink<P> + Send,
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
            let mut pushes = Vec::new();
            let mut step = ActorStep {
                state: &mut state,
                out: &mut frames,
                pushes: &mut pushes,
            };
            self.dispatch_event(event, &mut step)?;
            // Pushes are written before response frames, matching the order
            // the sources produced them.
            if self.flush_or_cancel(pushes, frames, sink).await? == WriteOutcome::Cancelled {
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
    /// emitted without a stream terminator and `on_command_end` runs once. A
    /// write abandoned by shutdown or by the peer's idle grace reports
    /// [`ResponseOutcome::Cancelled`] and skips the hook.
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
        S: FrameSink<F> + PushSink<P> + Send,
    {
        // Diagnostics for a stalled response report the request it belonged to.
        self.response_correlation = correlation_id;
        match response {
            Response::Single(frame) => {
                let outcome = self
                    .emit_response_frames(vec![frame], correlation_id, sink)
                    .await?;
                Ok(self.finish_immediate_response(outcome))
            }
            Response::Vec(frames) => {
                let outcome = self
                    .emit_response_frames(frames, correlation_id, sink)
                    .await?;
                Ok(self.finish_immediate_response(outcome))
            }
            Response::Empty => {
                ensure_output_idle(&self.active_output)?;
                self.run_completion_hook();
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
        S: FrameSink<F> + PushSink<P> + Send,
    {
        while self.active_output.is_response() || self.active_output.is_multi_packet() {
            let mut state = ActorState::new(
                self.active_output.is_response(),
                self.active_output.is_multi_packet(),
            );
            let event = self.next_event_while_draining(&state).await;
            match self.response_interruption(&event) {
                Some(ResponseInterruption::Abandon) => {
                    ResponseCancellationReason::Shutdown.record();
                    return Ok(ResponseOutcome::Cancelled);
                }
                Some(ResponseInterruption::Bounded) => {
                    return self.enter_bounded_drain(&event, sink).await;
                }
                None => {}
            }
            if let Some(outcome) = self.dispatch_and_write(event, &mut state, sink).await? {
                return Ok(outcome);
            }
        }
        Ok(self.finish_outcome())
    }

    /// Switch to the bounded drain, dropping pushes if a stop caused it.
    ///
    /// A stop that starts the bounded drain ends push delivery: the documented
    /// shutdown rule is that a frame still waiting in a push queue at that
    /// moment is dropped rather than flushed. Discarding the lanes enforces
    /// that rule and stops a producer from renewing the drain's idle grace with
    /// a fresh push, which would let it hold the connection open indefinitely.
    /// A peer close keeps the lanes open, because the connection still drains
    /// the frames it accepted before the half-close.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the sink fails while draining.
    async fn enter_bounded_drain<S>(
        &mut self,
        event: &Event<F, E, P>,
        sink: &mut S,
    ) -> Result<ResponseOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + PushSink<P> + Send,
    {
        if matches!(event, Event::Shutdown) {
            self.discard_push_lanes();
        }
        self.drive_bounded_drain(sink).await
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
            self.run_completion_hook();
        }
    }

    /// Emit a completed response, stamping the request correlation ID.
    ///
    /// A frame that already carries an identifier keeps it when the request
    /// has none, matching [`CorrelatableFrame`] inheritance elsewhere: a
    /// middleware-provided response id must not be cleared by a request that
    /// simply never supplied one.
    ///
    /// The write is bounded like any other. A single frame is enough to make a
    /// peer that stopped reading block the connection, and a graceful drain
    /// must not wait for it.
    async fn emit_response_frames<S>(
        &mut self,
        frames: Vec<F>,
        correlation_id: Option<u64>,
        sink: &mut S,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + PushSink<P> + Send,
    {
        ensure_output_idle(&self.active_output)?;
        let mut out = Vec::with_capacity(frames.len());
        for mut frame in frames {
            if let Some(request_correlation) = correlation_id {
                frame.set_correlation_id(Some(request_correlation));
            }
            self.process_frame_with_hooks_and_metrics(frame, &mut out);
        }
        self.flush_or_cancel(Vec::new(), out, sink).await
    }

    /// Finish an immediately emitted response.
    ///
    /// The completion hook belongs after a successful write. A cancelled write
    /// leaves a closing connection, so it is reported without the hook, the
    /// same way a cancelled stream is.
    fn finish_immediate_response(&mut self, outcome: WriteOutcome) -> ResponseOutcome {
        if outcome == WriteOutcome::Cancelled {
            return ResponseOutcome::Cancelled;
        }
        self.run_completion_hook();
        ResponseOutcome::Completed
    }

    /// Run the completion hook when this actor owns one.
    ///
    /// A prepared connection's actor is built in no-hook mode, so completion
    /// belongs to its `FramePipeline` and this is a no-op.
    fn run_completion_hook(&mut self) {
        if let Some(protocol) = self.protocol.as_mut() {
            protocol.on_command_end();
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
