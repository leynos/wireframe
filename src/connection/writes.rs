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
    ProgressSignal,
    drive::ResponseOutcome,
    event::Event,
    grace::GraceTimer,
    sink::next_progress,
    state::ActorState,
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
    response::WireframeError,
};

/// Whether a bounded write finished before the connection was cancelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WriteOutcome {
    /// Every buffered frame reached the sink.
    Written,
    /// Shutdown was requested while the write was still pending.
    Cancelled,
}

/// Cancellation bounds applied to a buffered write.
///
/// Grouped so the write helper takes one bound rather than each token and
/// deadline separately, which keeps its signature readable as bounds change.
pub(super) struct WriteBounds<'a> {
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
#[derive(Clone, Copy, Debug)]
pub(super) enum WriteLimit {
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
    /// Each frame is bounded separately so that accepting one renews the idle
    /// grace for the next. Bounding the whole batch instead would abandon a
    /// peer that is still reading whenever the batch takes longer than one
    /// grace to write, even though no individual write stalled. A sink that
    /// splits one frame into several physical writes renews the grace per
    /// physical write through its [`FrameSink::progress`] signal.
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
        for frame in frames {
            let progress = sink.progress();
            let outcome = self
                .write_frame_bounded(sink.send_frame(frame), progress.as_deref())
                .await
                .map_err(WireframeError::Io)?;
            if outcome == WriteOutcome::Cancelled {
                return Ok(WriteOutcome::Cancelled);
            }
        }
        Ok(WriteOutcome::Written)
    }

    /// Await one write under this connection's write bounds.
    ///
    /// The prepared connection writes its protocol terminator outside the
    /// actor's buffered path, but that last frame can block on a peer that
    /// stopped reading just as readily as any other. Sharing this bound keeps
    /// one policy for every write the prepared path performs: a ready write
    /// wins, and a stalled one is abandoned at shutdown or once the peer's idle
    /// grace expires.
    ///
    /// # Errors
    ///
    /// Returns the error the write itself reports.
    pub(crate) async fn write_frame_bounded<Err, Fut>(
        &mut self,
        writing: Fut,
        progress: Option<&ProgressSignal>,
    ) -> Result<WriteOutcome, Err>
    where
        Fut: Future<Output = Result<(), Err>>,
    {
        let limit = {
            let bounds = self.write_bounds();
            write_bounded(writing, &bounds, progress).await?
        };
        Ok(self.record_write_limit(limit))
    }

    /// Report and record why a bounded write stopped.
    pub(super) fn record_write_limit(&mut self, limit: WriteLimit) -> WriteOutcome {
        match limit {
            WriteLimit::Written => WriteOutcome::Written,
            WriteLimit::Shutdown => {
                ResponseCancellationReason::Shutdown.record();
                WriteOutcome::Cancelled
            }
            WriteLimit::Grace => {
                self.cancel_stalled_response();
                WriteOutcome::Cancelled
            }
        }
    }

    /// Snapshot the bounds that apply to one buffered write.
    pub(super) fn write_bounds(&self) -> WriteBounds<'_> {
        WriteBounds {
            shutdown: &self.shutdown,
            peer_closed: &self.peer_closed,
            timer: Arc::clone(&self.grace_timer),
            grace: self.peer_closed_grace,
        }
    }
}

/// Await one write unless the connection ends or the peer's grace expires
/// first.
///
/// The grace is armed only once the peer becomes unobservable, so a write that
/// began while the peer was still being read is bounded from the moment that
/// observation ends rather than from the start of the write.
async fn write_bounded<Err>(
    writing: impl Future<Output = Result<(), Err>>,
    bounds: &WriteBounds<'_>,
    progress: Option<&ProgressSignal>,
) -> Result<WriteLimit, Err> {
    tokio::pin!(writing);
    tokio::select! {
        biased;
        // A ready write is preferred so that a cancellation observed while
        // frames are still being accepted does not abandon queued output. Only
        // a write that would block is raced against the bounds.
        result = &mut writing => {
            result?;
            Ok(WriteLimit::Written)
        }
        () = bounds.shutdown.cancelled() => Ok(WriteLimit::Shutdown),
        () = bounds.peer_closed.cancelled() => {
            write_within_grace(writing.as_mut(), bounds, progress).await
        }
    }
}

/// Finish a write that was pending when peer observation ended.
///
/// The write keeps its chance to complete, because a half-close alone does not
/// mean the peer stopped reading. Each accepted physical write renews the
/// deadline, so a peer that keeps reading is never abandoned merely because the
/// whole frame takes longer than one grace. Only a write that accepts nothing
/// for a whole idle grace, or a connection that shuts down, abandons it.
pub(super) async fn write_within_grace<Err>(
    mut writing: Pin<&mut impl Future<Output = Result<(), Err>>>,
    bounds: &WriteBounds<'_>,
    progress: Option<&ProgressSignal>,
) -> Result<WriteLimit, Err> {
    loop {
        tokio::select! {
            biased;
            result = &mut writing => {
                result?;
                return Ok(WriteLimit::Written);
            }
            () = bounds.shutdown.cancelled() => return Ok(WriteLimit::Shutdown),
            () = next_progress(progress) => {
                // Progress renews the grace: re-arm and keep waiting.
            }
            () = bounds.timer.elapse(bounds.grace) => return Ok(WriteLimit::Grace),
        }
    }
}
