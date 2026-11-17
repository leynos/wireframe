//! Reading inbound frames for one connection.
//!
//! The connection loop reads one frame at a time and keeps at most one frame
//! buffered while the previous one is handled. The state a read timeout purges
//! travels with that buffered frame, and every read also observes shutdown:
//! a read timeout only retries the read, so a stop that were not observed here
//! would wait for a peer that keeps its socket open without sending anything.
//! That would pin the connection task, its gauge guard, and the server drain
//! for as long as the peer stayed connected.

use std::{io, task::Poll, time::Duration};

use futures::{Stream, StreamExt};
use log::debug;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

use super::super::{codec_driver::FramePipeline, frame_handling, memory_budgets::MemoryBudgets};
use crate::message_assembler::MessageAssemblyState;

/// Outcome of waiting for the next inbound frame.
pub(super) enum NextFrame<F> {
    /// A frame is ready for dispatch.
    Ready(F),
    /// The peer closed its input half without sending another frame.
    Closed,
    /// No frame arrived within the configured read timeout.
    TimedOut,
}

/// Poll the buffered frame or the reader, giving up after `bound`.
///
/// A zero bound is a single non-blocking poll rather than a timer: Tokio
/// rounds a timer deadline up to its next tick, so `timeout(Duration::ZERO,
/// ..)` would hold an always-ready competitor — the push lanes — for up to a
/// tick on every pass. Polling once costs no time at all, which is what a
/// caller that owes the push lanes an attempt after each batch needs.
async fn poll_frame<R, F>(
    pending_frame: &mut Option<F>,
    reader: &mut R,
    bound: Duration,
) -> io::Result<NextFrame<F>>
where
    R: Stream<Item = io::Result<F>> + Unpin,
{
    if let Some(frame) = pending_frame.take() {
        return Ok(NextFrame::Ready(frame));
    }
    if bound.is_zero() {
        return Ok(match futures::poll!(reader.next()) {
            Poll::Ready(Some(Ok(frame))) => NextFrame::Ready(frame),
            Poll::Ready(Some(Err(error))) => return Err(error),
            Poll::Ready(None) => NextFrame::Closed,
            Poll::Pending => NextFrame::TimedOut,
        });
    }
    match timeout(bound, reader.next()).await {
        Ok(Some(Ok(frame))) => Ok(NextFrame::Ready(frame)),
        Ok(Some(Err(error))) => Err(error),
        Ok(None) => Ok(NextFrame::Closed),
        Err(_) => Ok(NextFrame::TimedOut),
    }
}

/// Drop expired fragmentation buffers and message assemblies.
pub(super) fn purge_expired<T>(
    pipeline: &mut FramePipeline<T>,
    message_assembly: &mut Option<MessageAssemblyState>,
) {
    pipeline.purge_expired();
    frame_handling::purge_expired_assemblies(message_assembly);
}

/// Read side of the inbound loop.
///
/// Groups the frame buffered while the previous one was being handled with the
/// state a read timeout purges. They are only ever touched together, and
/// holding them as one value lets the loop read through a single call that also
/// observes shutdown.
pub(super) struct InboundFrames<T> {
    /// Frame read while the previous one was still being handled.
    pub(super) pending: Option<T>,
    /// Fragmentation pipeline, whose expired buffers a timeout drops.
    pub(super) pipeline: FramePipeline<T>,
    /// Reassembled messages, likewise purged on a timeout.
    pub(super) message_assembly: Option<MessageAssemblyState>,
    /// Deadline at which the current silence purges expired state.
    ///
    /// Kept here rather than armed inside one wait because a push batch
    /// cancels the read whenever it has frames to deliver. A deadline owned by
    /// the wait would be re-armed from scratch after every batch, so a client
    /// that only ever receives pushes would never reach the purge.
    pub(super) read_deadline: Option<Instant>,
}

impl<T> InboundFrames<T> {
    /// Apply memory pressure before reading the next inbound frame.
    ///
    /// The read pauses for the configured interval while soft limits are
    /// exceeded, and expired outbound and assembly state is purged on every
    /// pass.
    pub(super) async fn relieve_pressure(&mut self, budgets: MemoryBudgets) -> io::Result<()> {
        let pressure = frame_handling::evaluate_memory_pressure(
            self.message_assembly.as_ref(),
            Some(budgets),
            self.pipeline.fragmentation_buffered_bytes(),
        );
        frame_handling::apply_memory_pressure(pressure, || {
            purge_expired(&mut self.pipeline, &mut self.message_assembly);
        })
        .await
    }

    /// Poll once for a frame the buffered slot or the reader already holds.
    ///
    /// The connection loop polls pushes and the reader in one biased select, so
    /// an always-ready push lane wins every pass. This gives the reader one
    /// non-blocking chance per pass, which bounds how long an inbound frame can
    /// wait behind a push stream.
    ///
    /// The probe also runs the expiry purge once the shared read deadline has
    /// passed. It arms that deadline rather than a timer, because a loop whose
    /// push lane is ready on every pass never polls the read wait below;
    /// arming the deadline only there would leave buffered state without one,
    /// so the purge would wait for a timeout that never starts.
    pub(super) async fn try_next<R>(
        &mut self,
        reader: &mut R,
        read_timeout_ms: u64,
    ) -> io::Result<NextFrame<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        if self.arm_deadline(read_timeout_ms) <= Instant::now() {
            self.purge_after_timeout(read_timeout_ms);
        }
        let next = poll_frame(&mut self.pending, reader, Duration::ZERO).await?;
        if !matches!(next, NextFrame::TimedOut) {
            // Observing the peer ends the idle interval, so the next silence
            // gets its full timeout rather than the rest of this one.
            self.read_deadline = None;
        }
        Ok(next)
    }

    /// Read the next frame, or `None` once reading must stop.
    ///
    /// Reading stops when the peer closes its input half, which ends the
    /// connection, and when the server stops.
    ///
    /// A read timeout only retries the read, so a shutdown that were not
    /// observed here would wait for a peer that keeps its socket open without
    /// sending anything. That would pin the connection task, its gauge guard,
    /// and the server drain for as long as the peer stayed connected.
    ///
    /// The wait is cancelled whenever a push batch has frames to deliver, so
    /// the deadline it arms lives in the reader's state rather than in this
    /// call: the silence is measured across batches, not restarted by them.
    pub(super) async fn next<R>(
        &mut self,
        reader: &mut R,
        read_timeout_ms: u64,
        shutdown: &CancellationToken,
    ) -> io::Result<Option<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => Ok(None),
            frame = self.await_frame(reader, read_timeout_ms) => frame,
        }
    }

    /// Await one frame, purging expired state when the read deadline fires.
    async fn await_frame<R>(
        &mut self,
        reader: &mut R,
        read_timeout_ms: u64,
    ) -> io::Result<Option<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        loop {
            if self.pending.is_some() {
                self.read_deadline = None;
                return Ok(self.pending.take());
            }
            match timeout_at(self.arm_deadline(read_timeout_ms), reader.next()).await {
                Ok(Some(Ok(frame))) => {
                    self.read_deadline = None;
                    return Ok(Some(frame));
                }
                Ok(Some(Err(error))) => return Err(error),
                Ok(None) => return Ok(None),
                Err(_) => {
                    debug!("read timeout elapsed; continuing to wait for next frame");
                    self.purge_after_timeout(read_timeout_ms);
                }
            }
        }
    }

    /// Deadline for the current silence, armed on first use.
    fn arm_deadline(&mut self, read_timeout_ms: u64) -> Instant {
        *self
            .read_deadline
            .get_or_insert_with(|| Instant::now() + Duration::from_millis(read_timeout_ms))
    }

    /// Purge expired state and start the next idle interval.
    fn purge_after_timeout(&mut self, read_timeout_ms: u64) {
        purge_expired(&mut self.pipeline, &mut self.message_assembly);
        self.read_deadline = Some(Instant::now() + Duration::from_millis(read_timeout_ms));
    }
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
