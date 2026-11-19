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

/// Deadline for the current silence, armed on first use.
fn arm_deadline(deadline: &mut Option<Instant>, read_timeout_ms: u64) -> Instant {
    *deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(read_timeout_ms))
}

/// Start the next idle interval after a purge.
fn restart_deadline(deadline: &mut Option<Instant>, read_timeout_ms: u64) {
    *deadline = Some(Instant::now() + Duration::from_millis(read_timeout_ms));
}

/// Buffered inbound state an outbound write must keep expiring.
///
/// A push write stops running the loop's expiry command while it waits, and it
/// holds the transport's fragmentation pipeline for the whole write, so it
/// borrows the remaining expiry state directly: the reassembled messages and
/// the silence deadline. Sealing a partially written frame is worse than
/// letting this state expire a few frames later, and the pipeline's expired
/// fragment buffers are purged at each frame boundary instead.
pub(super) struct InboundExpiry<'a> {
    /// Reassembled messages the read deadline purges.
    assembly: &'a mut Option<MessageAssemblyState>,
    /// Deadline at which the current silence purges expired state.
    deadline: &'a mut Option<Instant>,
    /// Maximum interval to wait for the next inbound frame.
    read_timeout_ms: u64,
}

impl<'a> InboundExpiry<'a> {
    /// Borrow the expiry state of one connection's buffered reads.
    pub(super) const fn new(
        assembly: &'a mut Option<MessageAssemblyState>,
        deadline: &'a mut Option<Instant>,
        read_timeout_ms: u64,
    ) -> Self {
        Self {
            assembly,
            deadline,
            read_timeout_ms,
        }
    }

    /// Instant at which the current silence expires buffered state.
    pub(super) fn due(&mut self) -> Instant { arm_deadline(self.deadline, self.read_timeout_ms) }

    /// Purge expired assemblies once the deadline has passed.
    ///
    /// The deadline is shared with the read path, so a purge here is the same
    /// event that path would have performed; the next silence starts from now
    /// rather than from whenever the write finishes.
    pub(super) fn expire_silence(&mut self) {
        if arm_deadline(self.deadline, self.read_timeout_ms) <= Instant::now() {
            frame_handling::purge_expired_assemblies(self.assembly);
            restart_deadline(self.deadline, self.read_timeout_ms);
        }
    }
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

    /// Expire buffered state once the current silence has lasted the timeout.
    ///
    /// This is the read timeout as an explicit command. The read wait below
    /// arms and observes the same deadline, but the connection loop polls
    /// pushes and the reader in one biased select, so an always-ready push
    /// lane wins every pass and the wait is cancelled before its timeout
    /// fires. Running this command on each pass keeps the deadline measured
    /// from the last observed frame rather than from the last wait, so a
    /// client that only receives pushes still expires its buffered state.
    pub(super) fn expire_silence(&mut self, read_timeout_ms: u64) {
        if self.arm_deadline(read_timeout_ms) <= Instant::now() {
            self.purge_after_timeout(read_timeout_ms);
        }
    }

    /// Poll once for a frame the buffered slot or the reader already holds.
    ///
    /// The connection loop polls pushes and the reader in one biased select, so
    /// an always-ready push lane wins every pass. This gives the reader one
    /// non-blocking chance per pass, which bounds how long an inbound frame can
    /// wait behind a push stream.
    ///
    /// The probe only reads. Expiring buffered state is the separate
    /// [`Self::expire_silence`] command, so a probe cannot drop state as a side
    /// effect of being called. A returned frame still ends the idle interval,
    /// so the next silence gets its full timeout rather than the rest of this
    /// one.
    pub(super) async fn try_next<R>(&mut self, reader: &mut R) -> io::Result<NextFrame<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        let next = poll_frame(&mut self.pending, reader, Duration::ZERO).await?;
        if !matches!(next, NextFrame::TimedOut) {
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
    ///
    /// This reads Tokio's ambient clock rather than an injected timer seam.
    /// The clock *is* this loop's injection point: every test that exercises
    /// the deadline runs on a paused Tokio runtime, which `Instant::now` and
    /// `timeout_at` already follow, so a test advances the deadline
    /// deterministically without a custom trait. The connection layer's
    /// `GraceTimer` exists for the opposite case: those tests fire one
    /// deadline by hand and never advance a clock at all.
    fn arm_deadline(&mut self, read_timeout_ms: u64) -> Instant {
        arm_deadline(&mut self.read_deadline, read_timeout_ms)
    }

    /// Purge expired state and start the next idle interval.
    fn purge_after_timeout(&mut self, read_timeout_ms: u64) {
        purge_expired(&mut self.pipeline, &mut self.message_assembly);
        restart_deadline(&mut self.read_deadline, read_timeout_ms);
    }
}

/// Buffered read state holding one incomplete assembly that any purge evicts.
///
/// The assembly timeout is zero so that a purge always drops it; assertions
/// then measure *when* the purge runs rather than the assembly clock, which is
/// a `std::time::Instant` that a paused runtime does not advance.
#[cfg(test)]
pub(super) fn incomplete_inbound<T>() -> InboundFrames<T> {
    use crate::message_assembler::{
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
    };

    let mut assembly = MessageAssemblyState::new(
        std::num::NonZeroUsize::new(1024).unwrap_or(std::num::NonZeroUsize::MIN),
        Duration::ZERO,
    );
    let first = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: 4,
        total_body_len: Some(8),
        is_last: false,
    };
    let input = FirstFrameInput::new(&first, EnvelopeRouting::default(), Vec::new(), b"half")
        .expect("the incomplete frame is valid");
    assembly
        .accept_first_frame(input)
        .expect("the first frame is buffered");
    InboundFrames {
        pending: None,
        pipeline: FramePipeline::new(None),
        message_assembly: Some(assembly),
        read_deadline: None,
    }
}

/// Number of incomplete assemblies still buffered.
#[cfg(test)]
pub(super) fn buffered_assemblies<T>(inbound: &InboundFrames<T>) -> Option<usize> {
    inbound
        .message_assembly
        .as_ref()
        .map(crate::message_assembler::MessageAssemblyState::buffered_count)
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
