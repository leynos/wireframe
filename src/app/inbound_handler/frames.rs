//! Reading inbound frames for one connection.
//!
//! The connection loop reads one frame at a time and keeps at most one frame
//! buffered while the previous one is handled. The state a read timeout purges
//! travels with that buffered frame, and every read also observes shutdown:
//! a read timeout only retries the read, so a stop that were not observed here
//! would wait for a peer that keeps its socket open without sending anything.
//! That would pin the connection task, its gauge guard, and the server drain
//! for as long as the peer stayed connected.

use std::{io, time::Duration};

use futures::{Stream, StreamExt};
use log::debug;
use tokio::time::timeout;
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

/// Wait for the next inbound frame, reusing one buffered during dispatch.
///
/// A frame read while the previous one was being handled is returned first, so
/// the read half is never polled twice for one inbound frame.
pub(super) async fn next_inbound_frame<R, F>(
    pending_frame: &mut Option<F>,
    reader: &mut R,
    read_timeout_ms: u64,
) -> io::Result<NextFrame<F>>
where
    R: Stream<Item = io::Result<F>> + Unpin,
{
    if let Some(frame) = pending_frame.take() {
        return Ok(NextFrame::Ready(frame));
    }
    match timeout(Duration::from_millis(read_timeout_ms), reader.next()).await {
        Ok(Some(Ok(frame))) => Ok(NextFrame::Ready(frame)),
        Ok(Some(Err(error))) => Err(error),
        Ok(None) => Ok(NextFrame::Closed),
        Err(_) => Ok(NextFrame::TimedOut),
    }
}

/// State purged when a read timeout expires without a frame.
pub(super) struct ExpiryState<'a, T> {
    /// Fragmentation pipeline whose expired buffers are dropped.
    pub(super) pipeline: &'a mut FramePipeline<T>,
    /// Message assembly state whose expired assemblies are dropped.
    pub(super) message_assembly: &'a mut Option<MessageAssemblyState>,
}

/// Wait for the next inbound frame, purging expired state on a read timeout.
///
/// Returns `None` once the peer has closed, which ends the connection.
pub(super) async fn await_inbound_frame<T, R>(
    pending_frame: &mut Option<T>,
    reader: &mut R,
    read_timeout_ms: u64,
    expiry: &mut ExpiryState<'_, T>,
) -> io::Result<Option<T>>
where
    R: Stream<Item = io::Result<T>> + Unpin,
{
    loop {
        match next_inbound_frame(pending_frame, reader, read_timeout_ms).await? {
            NextFrame::Ready(frame) => return Ok(Some(frame)),
            NextFrame::Closed => return Ok(None),
            NextFrame::TimedOut => {
                debug!("read timeout elapsed; continuing to wait for next frame");
                purge_expired(expiry.pipeline, expiry.message_assembly);
            }
        }
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

    /// Read the next frame, or `None` once reading must stop.
    ///
    /// Reading stops when the peer closes its input half, which ends the
    /// connection, and when the server stops.
    ///
    /// A read timeout only retries the read, so a shutdown that were not
    /// observed here would wait for a peer that keeps its socket open without
    /// sending anything. That would pin the connection task, its gauge guard,
    /// and the server drain for as long as the peer stayed connected.
    pub(super) async fn next<R>(
        &mut self,
        reader: &mut R,
        read_timeout_ms: u64,
        shutdown: &CancellationToken,
    ) -> io::Result<Option<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        let mut expiry = ExpiryState {
            pipeline: &mut self.pipeline,
            message_assembly: &mut self.message_assembly,
        };
        tokio::select! {
            biased;
            () = shutdown.cancelled() => Ok(None),
            frame = await_inbound_frame(&mut self.pending, reader, read_timeout_ms, &mut expiry) => frame,
        }
    }
}
