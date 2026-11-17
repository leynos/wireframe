//! Reading inbound frames for one connection.
//!
//! The connection loop reads one frame at a time and keeps at most one frame
//! buffered while the previous one is handled. A read timeout returns control
//! to the loop instead of retrying in place: the loop is what purges expired
//! fragmentation and assembly state, and what fails a streaming body whose
//! assembly expired. Retrying the read here would leave an expired assembly
//! undetected for as long as the peer stayed quiet.
//!
//! Every read also observes shutdown: a stop that were not observed here would
//! wait for a peer that keeps its socket open without sending anything, which
//! would pin the connection task, its gauge guard, and the server drain.

use std::{io, time::Duration};

use futures::{Stream, StreamExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::super::codec_driver::FramePipeline;
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

/// Read side of the inbound loop.
///
/// Groups the frame buffered while the previous one was being handled with the
/// state the loop purges on a read timeout. They are only ever touched
/// together, and holding them as one value lets the loop read through a single
/// call that also observes shutdown.
pub(super) struct InboundFrames<T> {
    /// Frame read while the previous one was still being handled.
    pub(super) pending: Option<T>,
    /// Fragmentation pipeline, whose expired buffers a timeout drops.
    pub(super) pipeline: FramePipeline<T>,
    /// Reassembled messages, likewise purged on a timeout.
    pub(super) message_assembly: Option<MessageAssemblyState>,
}

impl<T> InboundFrames<T> {
    /// Read the next frame, or report EOF, shutdown, or a read timeout.
    ///
    /// A timeout returns [`NextFrame::TimedOut`] rather than retrying, so the
    /// connection loop can purge expired state and fail any streaming body
    /// whose assembly it evicted.
    pub(super) async fn next<R>(
        &mut self,
        reader: &mut R,
        read_timeout_ms: u64,
        shutdown: &CancellationToken,
    ) -> io::Result<NextFrame<T>>
    where
        R: Stream<Item = io::Result<T>> + Unpin,
    {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => Ok(NextFrame::Closed),
            frame = next_inbound_frame(&mut self.pending, reader, read_timeout_ms) => frame,
        }
    }
}