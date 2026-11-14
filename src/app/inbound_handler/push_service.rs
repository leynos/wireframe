//! Delivery of queued server pushes alongside inbound frame reading.
//!
//! The inbound loop must not wait for the next client frame before it can
//! deliver a server push, and a busy push stream must not starve inbound
//! traffic. Both sources are therefore polled in one biased `tokio::select!`:
//! pushes win ties, matching the connection actor's own priority ordering, and
//! the read arm keeps its own deadline so a saturated push lane cannot defer
//! the read indefinitely.

use futures::{Sink, Stream};
use tokio::io;
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        codec_driver::{ActorFrameSink, FramePipeline},
        envelope::Envelope,
    },
    frames::InboundFrames,
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PushSink},
    serializer::Serializer,
};

/// Outcome of waiting for the next unit of work on a connection.
pub(super) enum StreamStep<F: FrameCodec> {
    /// Frames taken from the connection's push queues, in priority order.
    Pushes(Vec<F::Frame>),
    /// One inbound frame read from the client.
    Frame(F::Frame),
    /// Every push lane closed and drained, so only the read remains.
    Idle,
    /// The client closed its write half at a frame boundary, or the server
    /// stopped serving this connection.
    Closed,
}

/// Wait for queued pushes or the next inbound frame, whichever happens first.
///
/// Push lanes are only polled while they are open; once the last handle is
/// dropped the select degenerates to a plain timed read instead of spinning on
/// a permanently ready `None`. The read arm is entered without the push arm
/// being ready, so its own read timeout and shutdown observation still apply on
/// every pass.
///
/// # Errors
///
/// Returns the transport error reported by the reader.
pub(super) async fn next_stream_step<F, R>(
    actor: &mut ConnectionActor<Envelope, (), F::Frame>,
    inbound: &mut InboundFrames<F::Frame>,
    reader: &mut R,
    read_timeout_ms: u64,
    shutdown: &CancellationToken,
) -> io::Result<StreamStep<F>>
where
    F: FrameCodec,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
{
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
