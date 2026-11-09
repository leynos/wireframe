//! Sink-driven execution loop for the connection actor.
//!
//! Every frame is composed by the actor and then written through an awaited
//! [`FrameSink`]. Buffering per event keeps the standalone `Vec` output order
//! identical to the historical synchronous driver while letting an integrated
//! transport apply back-pressure before the next source is polled.

use std::io;

use log::info;

use super::{
    ConnectionActor,
    FrameSink,
    event::Event,
    output::{ActiveOutput, AvailableOutput, EventAvailability},
    state::ActorState,
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    push::FrameLike,
    response::{Response, WireframeError},
};

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
            flush_frames(frames, sink).await?;
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
    ) -> Result<(), WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        match response {
            Response::Single(frame) => {
                self.emit_response_frames(vec![frame], correlation_id, sink)
                    .await?;
                self.hooks.on_command_end(&mut self.ctx);
            }
            Response::Vec(frames) => {
                self.emit_response_frames(frames, correlation_id, sink)
                    .await?;
                self.hooks.on_command_end(&mut self.ctx);
            }
            Response::Empty => {
                ensure_output_idle(&self.active_output)?;
                self.hooks.on_command_end(&mut self.ctx);
            }
            Response::Stream(stream) => {
                self.set_response(Some(stream))
                    .map_err(output_state_error)?;
                self.drive_response(sink).await?;
            }
            Response::MultiPacket(receiver) => {
                self.set_multi_packet_with_correlation(Some(receiver), correlation_id)
                    .map_err(output_state_error)?;
                self.drive_response(sink).await?;
            }
        }
        Ok(())
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
    pub(crate) async fn drive_response<S>(&mut self, sink: &mut S) -> Result<(), WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        while self.active_output.is_response() || self.active_output.is_multi_packet() {
            let mut state = ActorState::new(
                self.active_output.is_response(),
                self.active_output.is_multi_packet(),
            );
            let event = self.next_event(&state).await;
            if matches!(event, Event::Shutdown) {
                return Ok(());
            }

            let mut frames = Vec::new();
            self.dispatch_event(event, &mut state, &mut frames)?;
            flush_frames(frames, sink).await?;
        }
        Ok(())
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
        let avail = self.compute_availability(state);
        let can_poll_multi_packet = avail.output == AvailableOutput::MultiPacket;
        let can_poll_response = avail.output == AvailableOutput::Response;

        // Extract mutable references before the select! to satisfy the borrow
        // checker. Only one of these can be Some due to the ActiveOutput enum
        // invariant.
        let (multi_rx, response_stream) = match &mut self.active_output {
            ActiveOutput::MultiPacket(ctx) => (ctx.channel_mut(), None),
            ActiveOutput::Response(stream) => (None, Some(stream)),
            ActiveOutput::None => (None, None),
        };
        let shutdown = &self.shutdown;
        let high_rx = self.high_rx.as_mut();
        let low_rx = self.low_rx.as_mut();

        tokio::select! {
            biased;

            () = Self::wait_shutdown(shutdown), if state.is_active() => Event::Shutdown,
            res = Self::poll_queue(high_rx), if avail.high => Event::High(res),
            res = Self::poll_queue(low_rx), if avail.low => Event::Low(res),
            res = Self::poll_queue(multi_rx), if can_poll_multi_packet => Event::MultiPacket(res),
            res = Self::poll_response(response_stream), if can_poll_response => Event::Response(res),
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

/// Write buffered frames through the sink in emission order.
async fn flush_frames<F, E, S>(frames: Vec<F>, sink: &mut S) -> Result<(), WireframeError<E>>
where
    S: FrameSink<F> + Send,
{
    for frame in frames {
        sink.send_frame(frame).await.map_err(WireframeError::Io)?;
    }
    Ok(())
}
