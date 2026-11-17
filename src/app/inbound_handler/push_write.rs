//! Bounded writes for queued pushes delivered outside the connection actor.
//!
//! A push written by the inbound loop is not bounded by the actor's own write
//! policy, so it needs the same two bounds applied here: the server's stop
//! token and, once the peer can no longer be observed, the idle grace. Each
//! frame is bounded separately, because the grace measures idleness rather than
//! the duration of a whole batch.

use std::{future::Future, io, pin::Pin};

use futures::{Sink, Stream, StreamExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    super::PushWriteDiagnostics,
    push_service::{PushBatch, PushTransport},
};
use crate::{
    codec::FrameCodec,
    connection::{PEER_CLOSED_RESPONSE_GRACE, write_before_bound},
    metrics::PushWriteAbandonReason,
    serializer::Serializer,
};

/// What one push write means for the rest of the batch.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameWrite {
    /// Whether the connection may keep serving its peer.
    may_continue: bool,
    /// Whether the peer closed its write half during the write.
    peer_closed: bool,
}

impl FrameWrite {
    /// The frame reached the transport.
    const fn written(peer_closed: bool) -> Self {
        Self {
            may_continue: true,
            peer_closed,
        }
    }

    /// The write was abandoned, so the connection must stop.
    const fn stopped(peer_closed: bool) -> Self {
        Self {
            may_continue: false,
            peer_closed,
        }
    }

    /// Whether the connection may keep serving its peer.
    pub(super) const fn may_continue(self) -> bool { self.may_continue }

    /// Whether the peer closed its write half during the write.
    pub(super) const fn peer_closed(self) -> bool { self.peer_closed }
}

/// Write one push frame, observing the peer while it is still being read.
///
/// Each write is bounded on its own, so the grace is rebuilt per frame. One
/// deadline for the batch would measure total duration rather than idleness,
/// and abandon frames the peer was still accepting.
///
/// The rule `handle_watching_peer` applies: a buffered frame or an earlier EOF
/// means the read half is not polled, so the write is bounded by the grace
/// from its first poll instead.
pub(super) async fn write_frame<S, K, F, R>(
    transport: &mut PushTransport<'_, S, K, F>,
    frame: F::Frame,
    batch: &mut PushBatch<'_, F, R>,
    peer_closed: bool,
) -> io::Result<FrameWrite>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
{
    // Read the identity before the write borrows the transport: the write
    // future lives until this call returns, so the field is unreachable after.
    let diagnostics = transport.diagnostics.with_priority(batch.priority);
    let write = transport.write(frame);
    tokio::pin!(write);
    let mut closed = peer_closed;
    if batch.pending_frame.is_none() && !closed {
        let started = Instant::now();
        tokio::select! {
            biased;
            // A ready write is preferred: a frame the transport accepts
            // without blocking is still delivered, and only a write that
            // would block is abandoned.
            result = &mut write => {
                result?;
                return Ok(FrameWrite::written(closed));
            }
            // This arm abandons the frame before `finish_write` sees it, so it
            // records the abandonment itself; the bounded path below records
            // once and must not be counted twice.
            () = batch.shutdown.cancelled() => {
                diagnostics.record_abandonment(
                    PushWriteAbandonReason::Shutdown,
                    started.elapsed(),
                );
                return Ok(FrameWrite::stopped(closed));
            }
            next = batch.reader.next() => match next {
                Some(Ok(next_frame)) => *batch.pending_frame = Some(next_frame),
                Some(Err(error)) => return Err(error),
                None => {
                    batch.peer_eof.cancel();
                    closed = true;
                }
            },
        }
    }
    Ok(
        if finish_write(write.as_mut(), batch.shutdown, diagnostics).await? {
            FrameWrite::written(closed)
        } else {
            FrameWrite::stopped(closed)
        },
    )
}

/// Finish a push write under the shutdown token and the idle grace.
///
/// Returns whether the frame reached the transport. A blocked write is
/// abandoned by either bound and reported with the reason that ended it: only
/// the grace means the peer stopped reading, which is the case the structured
/// warning names, while a shutdown is counted without a warning because the
/// connection's own teardown already records the stop.
pub(super) async fn finish_write<W>(
    write: Pin<&mut W>,
    shutdown: &CancellationToken,
    diagnostics: PushWriteDiagnostics,
) -> io::Result<bool>
where
    W: Future<Output = io::Result<()>>,
{
    let started = Instant::now();
    let bound = write_before_bound(
        write,
        shutdown,
        tokio::time::sleep(PEER_CLOSED_RESPONSE_GRACE),
    )
    .await?;
    if bound.is_finished() {
        return Ok(true);
    }
    let reason = if bound.is_grace_expired() {
        PushWriteAbandonReason::IdleGraceExpired
    } else {
        PushWriteAbandonReason::Shutdown
    };
    diagnostics.record_abandonment(reason, started.elapsed());
    Ok(false)
}
