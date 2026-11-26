//! Writing queued push batches through a streaming connection's transport.
//!
//! A write to a peer that stopped reading can stay pending indefinitely, so
//! every push batch is bounded by the shutdown token and, once the peer can no
//! longer be observed, by the actor's idle grace. The read half keeps being
//! polled while the write runs, because this is the last place that can notice
//! a peer which half-closed and then stopped reading.
//!
//! The work lives apart from [`super::stream_dispatch`] so that module stays
//! the state machine — which event arrived and what it means — while this one
//! owns what a push batch costs to write.

use std::io;

use futures::{Sink, Stream, StreamExt};

use super::{
    super::{
        PushWriteDiagnostics,
        envelope::{Envelope, Packet},
        recovery_codec::DecodedItem,
    },
    frames::InboundExpiry,
    push_service::{PushBatch, PushTransport, deliver_pushes, drain_queued},
    recovery::hold_or_end,
    stream_actions::finish_step,
    stream_dispatch::Connection,
    stream_handles::LoopState,
    stream_observed::ObservedAction,
    stream_types::{Event, Step},
};
use crate::{
    codec::FrameCodec,
    connection::{ConnectionActor, PendingPushes},
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    request::QueuedBytes,
    serializer::Serializer,
};

impl<S, E, R, K, F> Connection<'_, S, E, R, K, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<DecodedItem<F::Frame>>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Write one batch of queued pushes through the connection's transport.
    ///
    /// A write to a peer that stopped reading can stay pending indefinitely, so
    /// it is bounded by the shutdown token and, once the peer can no longer be
    /// observed, by the idle grace. The read half keeps being polled while the
    /// write runs, because this is the last place that can notice a peer which
    /// half-closed and then stopped reading.
    ///
    /// The write also keeps the inbound read deadline expiring, and an expired
    /// streaming assembly is failed here rather than inside the write: the
    /// write holds the reader, so the reply a failed body owes can only be
    /// forwarded once the batch has been written.
    pub(super) async fn write_pushes(
        &mut self,
        state: &mut LoopState,
        pushes: PendingPushes<F::Frame>,
        queued: QueuedBytes,
    ) -> io::Result<Step> {
        let read_timeout_ms = self.ctx.read_timeout_ms;
        let serializer = self.ctx.serializer;
        let codec = self.ctx.codec;
        let shutdown = self.shutdown;
        let peer_eof = self.peer_eof;
        let mut evicted = Vec::new();
        let written = {
            let Connection {
                reader,
                sink,
                inbound,
                actor,
                recovery,
                ..
            } = self;
            // A read made while the write is pending is buffered for the next
            // dispatch, unless it reports the end of the stream: that ends the
            // connection now, so a truncation is observed even mid-write.
            let held_reader = &mut **reader;
            let mut held = held_reader.map(|read: io::Result<DecodedItem<F::Frame>>| {
                read.and_then(|item| hold_or_end(item, recovery))
            });
            let mut transport = PushTransport {
                serializer,
                codec,
                sink,
                pipeline: &mut inbound.pipeline,
                diagnostics: push_diagnostics::<F>(&**actor),
            };
            let batch = PushBatch {
                frames: pushes.frames,
                priority: pushes.priority,
                reader: &mut held,
                pending_frame: &mut inbound.pending,
                expiry: InboundExpiry::new(
                    &mut inbound.message_assembly,
                    &mut inbound.read_deadline,
                    read_timeout_ms,
                    &mut evicted,
                ),
                peer_eof,
                shutdown,
            };
            deliver_pushes(&mut transport, &mut **actor, batch).await?
        };
        // The reader is owed the next pass, so a continuously ready push lane
        // cannot win every biased select and starve the client's frames.
        state.serve_read_first = true;
        if !written {
            // A push abandoned mid-frame leaves a partial frame on the wire, so
            // the exchange cannot be resumed: abort the live body and close
            // instead of writing an expiry reply after the broken frame.
            state.streams.abort();
            return Ok(Step::Closed);
        }
        if evicted.is_empty() {
            return Ok(Step::Continue);
        }
        let dispatch = self
            .dispatch_observed(state, queued, ObservedAction::Expire(evicted))
            .await?;
        Ok(finish_step(dispatch))
    }

    /// Deliver frames still queued when the peer stopped writing.
    pub(super) async fn drain_pushes(&mut self) -> io::Result<()> {
        let serializer = self.ctx.serializer;
        let codec = self.ctx.codec;
        let shutdown = self.shutdown;
        let Connection {
            sink,
            inbound,
            actor,
            ..
        } = self;
        let mut transport = PushTransport {
            serializer,
            codec,
            sink,
            pipeline: &mut inbound.pipeline,
            diagnostics: push_diagnostics::<F>(&**actor),
        };
        drain_queued(&mut transport, &mut **actor, shutdown).await
    }
}

/// Classify one push-lane poll for the loop.
pub(super) fn push_event<T>(pushes: Option<PendingPushes<T>>) -> Event<T> {
    pushes.map_or(Event::LanesIdle, Event::Pushes)
}

/// Identity this connection reports when it abandons a push write.
const fn push_diagnostics<F>(
    actor: &ConnectionActor<Envelope, (), F::Frame>,
) -> PushWriteDiagnostics
where
    F: FrameCodec,
{
    PushWriteDiagnostics::new(actor.connection_id(), actor.peer_addr())
}
