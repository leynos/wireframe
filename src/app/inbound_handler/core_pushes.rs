//! Push delivery steps of the inbound connection loop.
//!
//! A push batch is taken from the response actor and written through the
//! connection's own sink, and the lanes are drained once the peer stops. These
//! are inherent methods on [`ConnectionLoop`] because each one drives the
//! loop's sink, reader, buffered inbound state and response actor, and they
//! live in their own file so the loop's definition stays within the module
//! size limit.

use std::io;

use futures::StreamExt;

use super::{
    AsyncRead,
    AsyncWrite,
    ConnectionLoop,
    DecodeWith,
    EncodeWith,
    Envelope,
    FrameCodec,
    FrameMetadata,
    InboundExpiry,
    Packet,
    PendingPushes,
    PushBatch,
    PushTransport,
    Serializer,
    deliver_pushes,
    drain_queued,
    hold_or_end,
};

impl<S, E, F, W> ConnectionLoop<'_, S, E, F, W>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Write queued push frames through the connection's transport sink.
    ///
    /// A write to a peer that stopped reading can stay pending indefinitely,
    /// so it is bounded by the shutdown token and, once the peer can no longer
    /// be observed, by the idle grace. The read half keeps being polled while
    /// the write runs, because this is the last place that can notice a peer
    /// which half-closed and then stopped reading.
    pub(super) async fn write_pushes(
        &mut self,
        pending: PendingPushes<F::Frame>,
    ) -> io::Result<bool> {
        let diagnostics = self.response_actor.push_diagnostics();
        let mut transport = PushTransport {
            serializer: self.serializer,
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
            diagnostics,
        };
        // A push write keeps reading so a half-closed peer is noticed. A read
        // that reports the end of the stream ends the connection instead of
        // being buffered, so a truncation during a stalled write is reported.
        let recovery = &self.recovery;
        let mut watched = (&mut self.reader)
            .map(|read: io::Result<_>| read.and_then(|it| hold_or_end(it, recovery)));
        let batch = PushBatch {
            frames: pending.frames,
            priority: pending.priority,
            reader: &mut watched,
            pending_frame: &mut self.inbound.pending,
            expiry: InboundExpiry::new(
                &mut self.inbound.message_assembly,
                &mut self.inbound.read_deadline,
                self.read_timeout_ms,
            ),
            peer_eof: &self.peer_eof,
            shutdown: &self.shutdown,
        };
        deliver_pushes(&mut transport, &mut self.response_actor.actor, batch).await
    }

    /// Deliver frames still queued when the peer stopped writing.
    pub(super) async fn drain_pushes(&mut self) -> io::Result<()> {
        let diagnostics = self.response_actor.push_diagnostics();
        let mut transport = PushTransport {
            serializer: self.serializer,
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
            diagnostics,
        };
        drain_queued(
            &mut transport,
            &mut self.response_actor.actor,
            &self.shutdown,
        )
        .await
    }
}
