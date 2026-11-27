//! Delivery of queued server pushes alongside inbound frame reading.
//!
//! The inbound loop must not wait for the next client frame before it can
//! deliver a server push, and a busy push stream must not starve inbound
//! traffic. [`super::stream_dispatch`] polls both sources in one biased
//! `tokio::select!`: pushes win ties, matching the connection actor's own
//! priority ordering. Bias alone would starve the client, so that loop serves a
//! buffered frame first and gives the reader one non-blocking attempt after
//! each batch.
//!
//! This module owns the write half of that arrangement: the batch the actor
//! hands over, the transport it is written through, and the drain that follows
//! a peer closing its write half.

use futures::{Sink, Stream};
use tokio::io;
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        PushWriteDiagnostics,
        codec_driver::{ActorFrameSink, FramePipeline},
        envelope::Envelope,
    },
    frames::InboundExpiry,
    push_write::{finish_write, write_frame},
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PushSink},
    push::PushPriority,
    serializer::Serializer,
};

#[cfg(test)]
#[path = "push_service_tests.rs"]
mod tests;

/// Transport resources shared by push writes and the EOF drain.
///
/// A push is already encoded for the wire, so only the `before_send` hook and
/// the transport write remain; fragmentation, serialization, and codec
/// wrapping stay on the response path.
pub(super) struct PushTransport<'a, S, K, F>
where
    F: FrameCodec,
{
    /// Serializer shared with the response path.
    pub(super) serializer: &'a S,
    /// Codec used to configure the framed transport.
    pub(super) codec: &'a F,
    /// Write half of the connection's framed transport.
    pub(super) sink: &'a mut K,
    /// Outbound pipeline supplying the per-connection `before_send` hook.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Connection identity reported when a write is abandoned.
    pub(super) diagnostics: PushWriteDiagnostics,
}

impl<S, K, F> PushTransport<'_, S, K, F>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    /// Write one queued push frame.
    ///
    /// Taking one frame per call lets the caller bound each write by its own
    /// idle grace instead of one deadline for a whole batch.
    ///
    /// # Errors
    ///
    /// Returns the transport error raised by the write.
    pub(super) async fn write(&mut self, frame: F::Frame) -> io::Result<()> {
        self.frame_sink().send_push(frame).await
    }

    /// Drop fragment buffers whose reassembly deadline has passed.
    ///
    /// The pipeline is borrowed by a write for as long as the write is pending,
    /// so the caller can only run this between frames.
    pub(super) fn purge_expired_fragments(&mut self) { self.pipeline.purge_expired(); }

    /// Borrow the push-writing sink for the duration of one call.
    fn frame_sink(&mut self) -> ActorFrameSink<'_, S, K, F> {
        ActorFrameSink::new(
            self.serializer,
            self.codec,
            &mut *self.sink,
            &mut *self.pipeline,
        )
    }
}

/// One batch of queued pushes and the peer state its write observes.
///
/// Grouped because a push write keeps polling the read half while it is
/// pending, so the frames and the observation state travel together; a
/// positional list of five arguments is easy to pass in the wrong order.
pub(super) struct PushBatch<'a, F, R, I>
where
    F: FrameCodec,
{
    /// Frames taken from the push queues, in delivery order.
    pub(super) frames: Vec<F::Frame>,
    /// Lane the frames came from, absent when both lanes were drained.
    pub(super) priority: Option<PushPriority>,
    /// Read half of the connection's framed transport.
    pub(super) reader: &'a mut R,
    /// Item read during the write, kept for the next dispatch.
    pub(super) pending_frame: &'a mut Option<I>,
    /// Inbound state that keeps expiring while the write is stalled.
    pub(super) expiry: InboundExpiry<'a>,
    /// Cancelled only by a clean EOF, which is permanent for the connection.
    pub(super) peer_eof: &'a CancellationToken,
    /// Cancels when the server stops or this connection ends.
    pub(super) shutdown: &'a CancellationToken,
}

/// Write one batch of queued pushes, observing the peer while it is pending.
///
/// A write to a peer that stopped reading can stay pending indefinitely, so it
/// is bounded twice: by the connection's stop token, and — once the peer can no
/// longer be observed — by the same grace that bounds the actor's own writes.
///
/// The read half keeps being polled while the write is pending. This is the
/// last place that can notice a peer which half-closed its write side and then
/// stopped reading: without it the loop would never observe the close, and the
/// write would pin the connection, its registry entry, and the server drain
/// until shutdown. A frame read here is buffered for the next dispatch rather
/// than dropped, and the read deadline keeps purging expired buffered state
/// while the socket stays silent.
///
/// Reports whether the connection may keep serving its peer. A peer that
/// closed its write half has its remaining queued frames drained first, so a
/// producer told a frame had been accepted still gets it delivered.
///
/// # Errors
///
/// Returns the transport error raised by the write or by the read half.
pub(super) async fn deliver_pushes<S, K, F, R, I>(
    transport: &mut PushTransport<'_, S, K, F>,
    actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    mut batch: PushBatch<'_, F, R, I>,
) -> io::Result<bool>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    R: Stream<Item = io::Result<I>> + Unpin,
{
    let mut peer_closed = batch.peer_eof.is_cancelled();
    for frame in std::mem::take(&mut batch.frames) {
        // A batch can take several idle intervals to write, so the deadline is
        // observed between frames as well as inside each write. The pipeline is
        // borrowed by the transport, so its expired fragment buffers are purged
        // here rather than by the expiry state.
        batch.expiry.expire_silence();
        transport.purge_expired_fragments();
        let written = write_frame(transport, frame, &mut batch, peer_closed).await?;
        peer_closed = written.peer_closed();
        if !written.may_continue() {
            return Ok(false);
        }
    }
    if !peer_closed {
        return Ok(true);
    }
    drain_queued(transport, actor, batch.shutdown).await?;
    Ok(false)
}

/// Deliver frames still queued when the peer closed its write half.
///
/// A peer that closed its write half but stopped reading applies
/// back-pressure that can hold a send pending forever, so each write is
/// bounded by the connection's shutdown token and by the same grace that
/// bounds a response whose peer can no longer be observed. Either bound
/// abandons the remaining frames instead of pinning the connection task and
/// its registry entry.
///
/// Collecting the lanes does not wait for a producer. A lane reports itself
/// exhausted only once its buffer is empty and every permit taken before the
/// close has been released, so awaiting that would let a producer that never
/// sends hold the drain before its first write, with nothing but a stop to end
/// the wait. The lanes are sealed instead and only the frames they already
/// hold are taken, which is exactly the set whose producers were told their
/// pushes had been accepted.
///
/// The grace restarts for every frame, so it measures idleness rather than the
/// time the whole drain has taken. A single deadline for the drain would
/// abandon frames the peer was accepting within the grace.
///
/// # Errors
///
/// Returns the first transport error raised while draining.
pub(super) async fn drain_queued<S, K, F>(
    transport: &mut PushTransport<'_, S, K, F>,
    actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    shutdown: &CancellationToken,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    let pending = actor.take_pending_pushes();
    // Both lanes are drained here, so the batch has no single priority to
    // report and every write is labelled `mixed`.
    let diagnostics = transport.diagnostics.with_priority(pending.priority);
    for frame in pending.frames {
        let write = transport.write(frame);
        tokio::pin!(write);
        if !finish_write(write.as_mut(), shutdown, diagnostics).await? {
            return Ok(());
        }
    }
    Ok(())
}
