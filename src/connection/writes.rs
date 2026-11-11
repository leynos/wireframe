//! Bounded writes for the connection actor.
//!
//! Every frame is composed by the actor and then written through an awaited
//! [`FrameSink`], which may apply back-pressure for as long as the peer keeps
//! its receive window closed. These helpers keep a write abandonable, while
//! preferring writes that can still complete so that a cancellation observed
//! mid-drain does not discard output the sink would have accepted.

use tokio_util::sync::CancellationToken;

use super::{ConnectionActor, FrameSink, drive::ResponseOutcome, event::Event, state::ActorState};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
    response::WireframeError,
};

/// Whether a buffered write finished before the connection was cancelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WriteOutcome {
    /// Every buffered frame reached the sink.
    Written,
    /// Shutdown was requested while the write was still pending.
    Cancelled,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Dispatch one event and write its frames.
    ///
    /// Returns `Some(outcome)` when the response must stop, and `None` when the
    /// loop should continue with the next event.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] from the event dispatch or the sink.
    pub(super) async fn dispatch_and_write<S>(
        &mut self,
        event: Event<F, E>,
        state: &mut ActorState,
        sink: &mut S,
    ) -> Result<Option<ResponseOutcome>, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        let mut frames = Vec::new();
        self.dispatch_event(event, state, &mut frames)?;
        if self.flush_or_cancel(frames, sink).await? == WriteOutcome::Cancelled {
            ResponseCancellationReason::Shutdown.record();
            return Ok(Some(ResponseOutcome::Cancelled));
        }
        self.run_deferred_command_end();
        Ok(None)
    }

    /// Write buffered frames, abandoning them if shutdown is requested.
    ///
    /// A peer that stops reading applies back-pressure that can hold a write
    /// pending indefinitely, so a graceful server drain would wait for a peer
    /// that may never read again. Racing the write against the actor's
    /// shutdown token lets the connection end instead. Abandoning a write can
    /// leave a partial frame on the wire, which is acceptable only because the
    /// connection is closing.
    pub(super) async fn flush_or_cancel<S>(
        &mut self,
        frames: Vec<F>,
        sink: &mut S,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        write_or_cancel(frames, sink, &self.shutdown).await
    }
}

/// Write buffered frames through the sink in emission order.
pub(super) async fn flush_frames<F, E, S>(
    frames: Vec<F>,
    sink: &mut S,
) -> Result<(), WireframeError<E>>
where
    S: FrameSink<F> + Send,
{
    for frame in frames {
        sink.send_frame(frame).await.map_err(WireframeError::Io)?;
    }
    Ok(())
}

/// Write buffered frames unless `shutdown` is cancelled first.
///
/// See [`ConnectionActor::flush_or_cancel`] for why a pending write must be
/// abandonable.
async fn write_or_cancel<F, E, S>(
    frames: Vec<F>,
    sink: &mut S,
    shutdown: &CancellationToken,
) -> Result<WriteOutcome, WireframeError<E>>
where
    S: FrameSink<F> + Send,
{
    tokio::select! {
        biased;
        // A ready write is preferred so that a cancellation observed while
        // frames are still being accepted does not abandon queued output. Only
        // a write that would block is raced against shutdown.
        result = flush_frames(frames, sink) => result.map(|()| WriteOutcome::Written),
        () = shutdown.cancelled() => Ok(WriteOutcome::Cancelled),
    }
}
