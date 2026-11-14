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

use futures::{Sink, Stream, StreamExt};
use log::warn;
use tokio::io;
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        codec_driver::{ActorFrameSink, FramePipeline},
        envelope::Envelope,
    },
    frames::{InboundFrames, NextFrame},
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PEER_CLOSED_RESPONSE_GRACE, PushSink, write_before_bound},
    serializer::Serializer,
};

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
        next = inbound.next(reader, read_timeout_ms, shutdown) => Ok(next.map_or(
            StreamStep::Closed,
            StreamStep::Frame,
        )),
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
    /// Write queued push frames in the order the actor produced them.
    ///
    /// # Errors
    ///
    /// Returns the first transport error; remaining frames are dropped and the
    /// connection ends.
    pub(super) async fn write(&mut self, frames: Vec<F::Frame>) -> io::Result<()> {
        let mut sink = self.frame_sink();
        for frame in frames {
            sink.send_push(frame).await?;
        }
        Ok(())
    }

    /// Deliver frames already queued when the client closed its write half.
    ///
    /// The actor closes both push lanes afterwards, so a push attempted after
    /// the drain is rejected by the queue rather than silently accepted and
    /// never written.
    ///
    /// # Errors
    ///
    /// Returns the first transport error raised while draining.
    pub(super) async fn drain(
        &mut self,
        actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    ) -> io::Result<()> {
        let mut sink = self.frame_sink();
        actor.drain_pending_pushes(&mut sink).await
    }

    /// Borrow the push-writing sink for the duration of one call.
    const fn frame_sink(&mut self) -> ActorFrameSink<'_, S, K, F> {
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
    batch: PushBatch<'_, F, R>,
) -> io::Result<bool>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
{
    let PushBatch {
        frames,
        reader,
        pending_frame,
        peer_eof,
        shutdown,
    } = batch;
    // The write borrows the transport, and the drain below needs it again, so
    // the whole observation phase is scoped to release that borrow.
    let (bound, peer_closed) = {
        let write = transport.write(frames);
        tokio::pin!(write);
        let mut peer_closed = peer_eof.is_cancelled();
        // The rule `handle_watching_peer` applies: a buffered frame or an
        // earlier EOF means the read half is not polled, so the write is
        // bounded by the grace from its first poll instead.
        if pending_frame.is_none() && !peer_closed {
            tokio::select! {
                biased;
                // A ready write is preferred: a frame the transport accepts
                // without blocking is still delivered, and only a write that
                // would block is abandoned.
                result = &mut write => return result.map(|()| true),
                () = shutdown.cancelled() => return Ok(false),
                next = reader.next() => match next {
                    Some(Ok(frame)) => *pending_frame = Some(frame),
                    Some(Err(error)) => return Err(error),
                    None => {
                        peer_eof.cancel();
                        peer_closed = true;
                    }
                },
            }
        }
        let bound = write_before_bound(
            &mut write,
            shutdown,
            tokio::time::sleep(PEER_CLOSED_RESPONSE_GRACE),
        )
        .await?;
        (bound, peer_closed)
    };
    if !bound.is_finished() {
        if bound.is_grace_expired() {
            warn!("abandoning a push write: the peer stopped reading");
        }
        return Ok(false);
    }
    if !peer_closed {
        return Ok(true);
    }
    drain_queued(transport, actor, shutdown).await?;
    Ok(false)
}

/// Deliver frames still queued when the peer closed its write half.
///
/// A peer that closed its write half but stopped reading applies
/// back-pressure that can hold a send pending forever. The drain is therefore
/// bounded by the connection's shutdown token and by the same grace that
/// bounds a response whose peer can no longer be observed; either bound
/// abandons the remaining frames instead of pinning the connection task and
/// its registry entry.
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
    let drain = transport.drain(actor);
    tokio::select! {
        biased;
        result = drain => result,
        () = shutdown.cancelled() => Ok(()),
        () = tokio::time::sleep(PEER_CLOSED_RESPONSE_GRACE) => {
            warn!("abandoning queued pushes: the peer stopped reading");
            Ok(())
        }
    }
}
