//! Bounded writes for queued pushes delivered outside the connection actor.
//!
//! A push written by the inbound loop is not bounded by the actor's own write
//! policy, so it needs the same two bounds applied here: the server's stop
//! token and, once the peer can no longer be observed, the idle grace. Each
//! frame is bounded separately, because the grace measures idleness rather than
//! the duration of a whole batch.

use std::{future::Future, io, pin::Pin};

use futures::{
    Sink,
    Stream,
    StreamExt,
    future::{self, Either},
};
use tokio::time::{Instant, Sleep, sleep, sleep_until};
use tokio_util::sync::CancellationToken;

use super::{
    super::PushWriteDiagnostics,
    frames::InboundExpiry,
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
///
/// # Why these steps stay together
///
/// The peer observation cannot move out of the write wait. While a write is
/// pending this loop is not reading the socket, so an end-of-file or a frame
/// that arrives at that moment is only observable by polling the reader in the
/// same select as the write. Polling the reader beforehand would miss exactly
/// the peer that half-closed while the transport stalled.
///
/// Abandonment accounting stays with each branch instead: only the branch that
/// fired knows which bound ended the write, and each stopped connection is
/// counted once. The shutdown arm records its own abandonment because it
/// returns before `finish_write` runs; the bounded path records after
/// `finish_write` has classified the bound.
pub(super) async fn write_frame<S, K, F, R, I>(
    transport: &mut PushTransport<'_, S, K, F>,
    frame: F::Frame,
    batch: &mut PushBatch<'_, F, R, I>,
    peer_closed: bool,
) -> io::Result<FrameWrite>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    R: Stream<Item = io::Result<I>> + Unpin,
{
    // Read the identity before the write borrows the transport: the write
    // future lives until this call returns, so the field is unreachable after.
    let diagnostics = transport.diagnostics.with_priority(batch.priority);
    let write = transport.write(frame);
    tokio::pin!(write);
    let mut closed = peer_closed;
    if batch.pending_frame.is_none() && !closed {
        let started = Instant::now();
        loop {
            let purge = purge_wait(Some(batch.expiry.due()));
            tokio::pin!(purge);
            tokio::select! {
                biased;
                // A ready write is preferred: a frame the transport accepts
                // without blocking is still delivered, and only a write that
                // would block is abandoned.
                result = &mut write => {
                    result?;
                    return Ok(FrameWrite::written(closed));
                }
                // This arm abandons the frame before the bounded wait sees it,
                // so it records the abandonment itself; the bounded path below
                // records once and must not be counted twice.
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
                // The deadline purges buffered state without cancelling the
                // write: a partially written frame keeps its place on the wire.
                () = &mut purge => batch.expiry.expire_silence(),
            }
        }
    }
    Ok(
        if finish_write_purging(write.as_mut(), batch, diagnostics).await? {
            FrameWrite::written(closed)
        } else {
            FrameWrite::stopped(closed)
        },
    )
}

/// Finish a delivery write, keeping the inbound expiry active.
///
/// The read half is no longer polled here, so `read_deadline` would otherwise
/// never be observed and a partial assembly below the memory-pressure threshold
/// could stay buffered past its timeout.
async fn finish_write_purging<W, F, R, I>(
    write: Pin<&mut W>,
    batch: &mut PushBatch<'_, F, R, I>,
    diagnostics: PushWriteDiagnostics,
) -> io::Result<bool>
where
    W: Future<Output = io::Result<()>>,
    F: FrameCodec,
{
    finish_write_with(write, batch.shutdown, diagnostics, Some(&mut batch.expiry)).await
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
    finish_write_with(write, shutdown, diagnostics, None).await
}

/// Await a write's bounds, optionally purging expired inbound state on the way.
///
/// The grace is armed once, from the first poll, and the purge interval never
/// restarts it: a write is bounded by idleness, not by how many idle intervals
/// a purge reported along the way. A purge wake-up leaves the write future
/// untouched, so a frame already partly on the wire is finished rather than
/// abandoned and restarted.
async fn finish_write_with<W>(
    write: Pin<&mut W>,
    shutdown: &CancellationToken,
    diagnostics: PushWriteDiagnostics,
    mut expiry: Option<&mut InboundExpiry<'_>>,
) -> io::Result<bool>
where
    W: Future<Output = io::Result<()>>,
{
    let started = Instant::now();
    // The expiry state is folded into the grace future, so the write is raced
    // against the same two bounds as every other app-layer write: the stop
    // token and the idle grace. Purging is a wake-up inside the grace, not a
    // third outcome.
    let bound = match expiry.as_mut() {
        Some(state) => write_before_bound(write, shutdown, grace_with_purges(state)).await?,
        None => write_before_bound(write, shutdown, sleep(PEER_CLOSED_RESPONSE_GRACE)).await?,
    };
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

/// Complete when the idle grace expires, purging expired inbound state first.
///
/// Every purge interval that passes while the write waits is observed here, so
/// buffered state keeps expiring for as long as the transport stalls.
async fn grace_with_purges(expiry: &mut InboundExpiry<'_>) {
    let grace = sleep(PEER_CLOSED_RESPONSE_GRACE);
    tokio::pin!(grace);
    loop {
        let purge = purge_wait(Some(expiry.due()));
        tokio::pin!(purge);
        tokio::select! {
            biased;
            // The grace is the bound, so it wins a tie with a purge that falls
            // due at the same instant.
            () = &mut grace => return,
            () = &mut purge => expiry.expire_silence(),
        }
    }
}

/// Wake at the inbound read deadline, or never when none is armed.
///
/// Waking is only a signal: the caller purges and re-enters the wait, so the
/// write it was awaiting is neither cancelled nor restarted.
fn purge_wait(deadline: Option<Instant>) -> Either<Sleep, future::Pending<()>> {
    deadline.map_or_else(
        || Either::Right(future::pending()),
        |at| Either::Left(sleep_until(at)),
    )
}
