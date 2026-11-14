//! Bounded writes for the connection actor.
//!
//! Every frame is composed by the actor and then written through an awaited
//! [`FrameSink`], which may apply back-pressure for as long as the peer keeps
//! its receive window closed. These helpers keep a write abandonable, while
//! preferring writes that can still complete so that a cancellation observed
//! mid-drain does not discard output the sink would have accepted.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use super::{
    ConnectionActor,
    FrameSink,
    drive::ResponseOutcome,
    event::Event,
    grace::GraceTimer,
    state::ActorState,
};
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

/// Cancellation bounds applied to a buffered write.
///
/// Grouped so the write helper takes one bound rather than each token and
/// deadline separately, which keeps its signature readable as bounds change.
struct WriteBounds<'a> {
    /// Token cancelled when the connection must stop.
    shutdown: &'a CancellationToken,
    /// Token cancelled once the peer can no longer be observed.
    peer_closed: &'a CancellationToken,
    /// Deadline source used to bound a write after peer observation ends.
    timer: Arc<dyn GraceTimer>,
    /// How long such a write may stay pending before it is abandoned.
    grace: Duration,
}

/// Why a bounded write stopped.
enum WriteLimit {
    /// The frames reached the sink.
    Written,
    /// The connection shut down while the write was pending.
    Shutdown,
    /// The peer became unobservable and did not accept the write in time.
    Grace,
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
            return Ok(Some(ResponseOutcome::Cancelled));
        }
        self.run_deferred_command_end();
        Ok(None)
    }

    /// Write buffered frames, abandoning them if the connection can no longer
    /// deliver them.
    ///
    /// A peer that stops reading applies back-pressure that can hold a write
    /// pending indefinitely, so a graceful server drain would wait for a peer
    /// that may never read again. Racing the write against the actor's
    /// shutdown token lets the connection end instead. Abandoning a write can
    /// leave a partial frame on the wire, which is acceptable only because the
    /// connection is closing.
    ///
    /// A peer that becomes unobservable mid-write is a separate case: the write
    /// may still succeed, so it is granted the same idle grace the observation
    /// loop uses rather than being abandoned at once. Only a peer that closes
    /// its input half and then accepts nothing for a whole grace is treated as
    /// gone.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the sink fails.
    pub(super) async fn flush_or_cancel<S>(
        &mut self,
        frames: Vec<F>,
        sink: &mut S,
    ) -> Result<WriteOutcome, WireframeError<E>>
    where
        S: FrameSink<F> + Send,
    {
        let limit = flush_bounded(frames, sink, &self.write_bounds()).await?;
        Ok(match limit {
            WriteLimit::Written => WriteOutcome::Written,
            WriteLimit::Shutdown => {
                ResponseCancellationReason::Shutdown.record();
                WriteOutcome::Cancelled
            }
            WriteLimit::Grace => {
                self.cancel_stalled_response();
                WriteOutcome::Cancelled
            }
        })
    }

    /// Snapshot the bounds that apply to one buffered write.
    fn write_bounds(&self) -> WriteBounds<'_> {
        WriteBounds {
            shutdown: &self.shutdown,
            peer_closed: &self.peer_closed,
            timer: Arc::clone(&self.grace_timer),
            grace: self.peer_closed_grace,
        }
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

/// Write frames unless the connection ends or the peer's grace expires first.
///
/// The grace is armed only once the peer becomes unobservable, so a write that
/// began while the peer was still being read is bounded from the moment that
/// observation ends rather than from the start of the write.
async fn flush_bounded<F, E, S>(
    frames: Vec<F>,
    sink: &mut S,
    bounds: &WriteBounds<'_>,
) -> Result<WriteLimit, WireframeError<E>>
where
    S: FrameSink<F> + Send,
{
    let writing = flush_frames(frames, sink);
    tokio::pin!(writing);
    let limit = tokio::select! {
        biased;
        // A ready write is preferred so that a cancellation observed while
        // frames are still being accepted does not abandon queued output. Only
        // a write that would block is raced against the bounds.
        result = &mut writing => {
            result?;
            WriteLimit::Written
        }
        () = bounds.shutdown.cancelled() => WriteLimit::Shutdown,
        () = bounds.peer_closed.cancelled() => {
            write_within_grace(writing.as_mut(), bounds).await?
        }
    };
    Ok(limit)
}

/// Finish a write that was pending when peer observation ended.
///
/// The write keeps its chance to complete, because a half-close alone does not
/// mean the peer stopped reading. Only a write that stays pending for the whole
/// idle grace, or a connection that shuts down, abandons it.
async fn write_within_grace<E>(
    mut writing: Pin<&mut impl Future<Output = Result<(), WireframeError<E>>>>,
    bounds: &WriteBounds<'_>,
) -> Result<WriteLimit, WireframeError<E>> {
    tokio::select! {
        biased;
        result = &mut writing => {
            result?;
            Ok(WriteLimit::Written)
        }
        () = bounds.shutdown.cancelled() => Ok(WriteLimit::Shutdown),
        () = bounds.timer.elapse(bounds.grace) => Ok(WriteLimit::Grace),
    }
}
