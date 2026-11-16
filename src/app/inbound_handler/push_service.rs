//! Delivery of queued server pushes alongside inbound frame reading.
//!
//! The inbound loop must not wait for the next client frame before it can
//! deliver a server push, and a busy push stream must not starve inbound
//! traffic. Both sources are therefore polled in one biased `tokio::select!`:
//! pushes win ties, matching the connection actor's own priority ordering.
//!
//! Bias alone would starve the client, because a continuously ready push lane
//! wins every pass. Two rules bound that: a frame buffered during dispatch is
//! always served first, and after each push batch the read gets one
//! non-blocking attempt, so an inbound frame waits behind at most one batch.

use futures::{Sink, Stream};
use tokio::io;
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        codec_driver::{ActorFrameSink, FramePipeline},
        envelope::Envelope,
    },
    frames::{InboundFrames, NextFrame},
    push_write::{finish_write, write_frame},
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PushSink},
    serializer::Serializer,
};

#[cfg(test)]
#[path = "push_service_tests.rs"]
mod tests;

/// Outcome of waiting for the next unit of work on a connection.
pub(super) enum StreamStep<F: FrameCodec> {
    /// Frames taken from the connection's push queues, in priority order.
    Pushes(Vec<F::Frame>),
    /// One inbound frame read from the client.
    Frame(F::Frame),
    /// Every push lane closed, so only the inbound read remains.
    Idle,
    /// The client closed its write half at a frame boundary, or the server
    /// stopped serving this connection.
    Closed,
}

/// Connection state one wait needs beyond the actor.
///
/// Grouped because the wait needs the buffered frame, the reader, and the
/// shutdown token together, and a positional list of that length is easy to
/// pass in the wrong order.
pub(super) struct StreamWait<'a, F, R>
where
    F: FrameCodec,
{
    /// Buffered inbound frame and the state its read timeout purges.
    pub(super) inbound: &'a mut InboundFrames<F::Frame>,
    /// Read half of the connection's framed transport.
    pub(super) reader: &'a mut R,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
    /// Token whose cancellation stops the read.
    pub(super) shutdown: &'a CancellationToken,
    /// Whether the inbound read gets one non-blocking attempt before pushes.
    pub(super) read_first: bool,
}

/// Wait for queued pushes or the next inbound frame, whichever happens first.
///
/// Push lanes are only polled while they are open; once the last handle is
/// dropped the select degenerates to a plain read instead of spinning on a
/// permanently ready `None`.
///
/// The read is tried before the push lanes when a frame is buffered or when the
/// previous wait was served from the queues. Without that, a continuously ready
/// push lane would win every biased pass and an inbound frame could wait
/// indefinitely behind pushes.
///
/// # Errors
///
/// Returns the transport error reported by the reader.
pub(super) async fn next_stream_step<F, R>(
    actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    wait: StreamWait<'_, F, R>,
) -> io::Result<StreamStep<F>>
where
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
{
    let StreamWait {
        inbound,
        reader,
        read_timeout_ms,
        shutdown,
        read_first,
    } = wait;
    let should_try_read = read_first || inbound.pending.is_some();
    if should_try_read {
        match inbound.try_next(reader).await? {
            NextFrame::Ready(frame) => return Ok(StreamStep::Frame(frame)),
            NextFrame::Closed => return Ok(StreamStep::Closed),
            NextFrame::TimedOut => {}
        }
    }

    let push_lanes_open = actor.has_open_push_lanes();
    tokio::select! {
        biased;

        pushes = actor.next_pushes(), if push_lanes_open => Ok(pushes.map_or(
            StreamStep::Idle,
            StreamStep::Pushes,
        )),
        next = inbound.next(reader, read_timeout_ms, shutdown) => {
            let frame = next?;
            Ok(frame.map_or(StreamStep::Closed, StreamStep::Frame))
        }
    }
}

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
pub(super) struct PushBatch<'a, F, R>
where
    F: FrameCodec,
{
    /// Frames taken from the push queues, in delivery order.
    pub(super) frames: Vec<F::Frame>,
    /// Read half of the connection's framed transport.
    pub(super) reader: &'a mut R,
    /// Frame read during the write, kept for the next dispatch.
    pub(super) pending_frame: &'a mut Option<F::Frame>,
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
/// than dropped.
///
/// Reports whether the connection may keep serving its peer. A peer that
/// closed its write half has its remaining queued frames drained first, so a
/// producer told a frame had been accepted still gets it delivered.
///
/// # Errors
///
/// Returns the transport error raised by the write or by the read half.
pub(super) async fn deliver_pushes<S, K, F, R>(
    transport: &mut PushTransport<'_, S, K, F>,
    actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    mut batch: PushBatch<'_, F, R>,
) -> io::Result<bool>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
{
    let mut peer_closed = batch.peer_eof.is_cancelled();
    for frame in std::mem::take(&mut batch.frames) {
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
/// back-pressure that can hold a send pending forever. Collecting the lanes is
/// therefore raced against the connection's shutdown token, and each write is
/// bounded by that token and by the same grace that bounds a response whose
/// peer can no longer be observed. Any of these bounds abandons the remaining
/// frames instead of pinning the connection task and its registry entry.
///
/// The race prefers a collection that is ready now, because those frames are
/// already buffered and no write is pending when the stop is observed: dropping
/// them would abandon output a still-readable peer can take. The token cuts
/// short only a collection that is still pending. That case needs bounding
/// because a closed lane reports itself exhausted only once its buffer is empty
/// and every permit taken before the close has been released, so a producer
/// that never releases one would otherwise keep this await pending without ever
/// reaching a write. Each selected frame is then written under its own bound,
/// which is what limits an abandoned drain.
///
/// The grace restarts for every frame, so it measures idleness rather than the
/// time the whole drain has taken. A lane can still hold per-frame
/// reservations from a slow producer, and one deadline for the drain would
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
    // A ready collection wins: its frames are already buffered, so a stop does
    // not discard them. A collection still waiting on a producer is abandoned
    // instead, which is what keeps that producer from pinning the connection.
    let frames = tokio::select! {
        biased;
        frames = actor.take_pending_pushes() => frames,
        () = shutdown.cancelled() => return Ok(()),
    };
    for frame in frames {
        let write = transport.write(frame);
        tokio::pin!(write);
        if !finish_write(write.as_mut(), shutdown).await? {
            return Ok(());
        }
    }
    Ok(())
}
