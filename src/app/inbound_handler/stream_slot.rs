//! Connection-owned streaming state: the dispatch slot and byte account.
//!
//! The slot owns everything that outlives a single inbound frame while a
//! request body streams: the handler future, the byte-accounted sender, and at
//! most one in-flight chunk admission. Keeping them together means the loop
//! never has to reconstruct a partially consumed exchange, and dropping the
//! slot cancels the handler and releases queued bytes in one step.

use std::{io, pin::Pin, time::Duration};

use bytes::Bytes;
use futures::Future;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};
use tokio_util::codec::Framed;
use tracing::debug;

use super::{
    super::{
        StreamingHandler,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
    },
    stream_dispatch::{Event, read_frame},
    stream_handles::{ChunkBudget, ChunkOffer, DispatchHandles, FirstChunk},
    stream_parked::forward_reply,
    stream_terminal::deliver_then_await,
};
#[cfg(test)]
#[path = "stream_slot_tests.rs"]
mod tests;

use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::{MessageAssemblyState, MessageKey, StreamingAssemblyId},
    request::{
        AccountedBodySender,
        DEFAULT_BODY_CHANNEL_CAPACITY,
        QueuedBytes,
        SendChunkError,
        accounted_body_channel,
    },
    serializer::Serializer,
};

/// Future returned by a streaming route handler.
pub(super) type HandlerFuture = Pin<Box<dyn Future<Output = Option<Bytes>> + Send + 'static>>;

/// One step while a chunk admission is in flight.
pub(super) enum PendingEvent {
    /// The chunk was queued, or the consumer had gone away.
    Send(Result<(), SendChunkError>),
    /// The handler resolved first; the admission is cancelled.
    Handler(Option<Bytes>),
    /// The bounded wait elapsed with the admission still queued.
    Idle,
}

/// The one live streaming exchange on a connection.
pub(super) struct ActiveStream {
    /// Assembly key used to match continuation frames.
    key: Option<MessageKey>,
    /// Identity of the partial this exchange owns, absent for a single-frame
    /// message or when no assembler participates.
    assembly: Option<StreamingAssemblyId>,
    /// Route identifier used to address the reply.
    route_id: u32,
    /// Correlation identifier inherited from the first frame.
    correlation_id: Option<u64>,
    /// Body sender; `None` once the body is complete or abandoned.
    sender: Option<AccountedBodySender>,
    /// Handler awaiting the body.
    handler: HandlerFuture,
    /// Chunk admission in flight.
    pub(super) pending: Option<PendingSend>,
}

/// One chunk waiting for byte admission and channel capacity.
pub(super) struct PendingSend {
    /// Admission future; dropping it cancels the send safely.
    future: Pin<Box<dyn Future<Output = Result<(), SendChunkError>> + Send + 'static>>,
    /// Whether this chunk also completes the message.
    is_last: bool,
}

impl ActiveStream {
    /// Drive the in-flight chunk admission alongside the handler.
    ///
    /// The wait is bounded by `wait` so the caller regains control even when
    /// neither side makes progress. Inbound frames cannot be read while an
    /// admission is queued, because a continuation for the same message would
    /// be mistaken for an abandoned body, so the timeout is what keeps assembly
    /// expiry and the memory budget observable instead.
    pub(super) async fn drive_pending(&mut self, wait: Duration) -> PendingEvent {
        let Self {
            pending, handler, ..
        } = self;
        let Some(send) = pending else {
            // The caller only drives this branch when a send is pending.
            return PendingEvent::Send(Ok(()));
        };
        let exchange = async {
            tokio::select! {
                result = &mut send.future => PendingEvent::Send(result),
                outcome = handler.as_mut() => PendingEvent::Handler(outcome),
            }
        };
        timeout(wait, exchange).await.unwrap_or(PendingEvent::Idle)
    }

    /// Drive the handler alongside the inbound socket.
    pub(super) async fn drive_open<F, W>(
        &mut self,
        framed: &mut Framed<W, ConnectionCodec<F>>,
        wait: Duration,
    ) -> Event<F::Frame>
    where
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let Self { handler, .. } = self;
        tokio::select! {
            outcome = handler.as_mut() => Event::Handler(outcome),
            event = read_frame::<F, W>(framed, wait) => event,
        }
    }
}

/// Connection-owned streaming state: the dispatch slot and byte account.
pub(super) struct ConnectionStreams {
    /// Bytes queued for the live body and released as the consumer reads.
    queued: QueuedBytes,
    /// The single live streaming exchange, if any.
    pub(super) active: Option<ActiveStream>,
}

impl ConnectionStreams {
    /// Create an empty slot with its own byte account.
    pub(super) fn new() -> Self {
        Self {
            queued: QueuedBytes::new(),
            active: None,
        }
    }

    /// Return whether a streaming body currently owns the slot.
    pub(super) const fn is_live(&self) -> bool { self.active.is_some() }

    /// Return the bytes queued for the consumer.
    pub(super) fn retained_bytes(&self) -> usize { self.queued.queued() }

    /// Return whether a chunk admission is in flight.
    pub(super) fn has_pending_send(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.pending.is_some())
    }

    /// Drive the in-flight chunk admission on the live exchange.
    pub(super) async fn drive_pending(&mut self, wait: Duration) -> PendingEvent {
        match self.active.as_mut() {
            Some(active) => active.drive_pending(wait).await,
            None => PendingEvent::Send(Ok(())),
        }
    }

    /// Start a streaming exchange and queue its first chunk.
    ///
    /// The first chunk becomes a pending admission rather than being awaited
    /// here. Admission is decided against the connection-wide byte account, so
    /// awaiting it would stall the loop on bytes this step cannot release:
    /// messages parked behind the body are only dispatched once the slot frees,
    /// and bytes an earlier body still holds are only released by a handler
    /// this call has not polled. Registering the slot first lets the loop drive
    /// the admission alongside the handler and the inbound socket, exactly as
    /// it drives every continuation chunk.
    ///
    /// A closed consumer simply leaves the handler to observe an empty body.
    pub(super) fn start(
        &mut self,
        first: FirstChunk,
        handler: &StreamingHandler,
        budget: ChunkBudget,
    ) {
        let FirstChunk {
            key,
            assembly,
            parts,
            chunk,
            is_last,
        } = first;
        let route_id = parts.id();
        let correlation_id = parts.correlation_id();
        let (sender, body) =
            accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, self.queued.clone());
        let handler_future = handler(parts, body);
        let admission = sender
            .clone()
            .send_chunk(chunk, budget.other_retained, budget.limit);
        // The final chunk still has to reach the handler, so the admission owns
        // its own sender clone until the loop records the result.
        let live_sender = (!is_last).then_some(sender);
        self.active = Some(ActiveStream {
            key,
            assembly,
            route_id,
            correlation_id,
            sender: live_sender,
            handler: handler_future,
            pending: Some(PendingSend {
                future: Box::pin(admission),
                is_last,
            }),
        });
    }

    /// Queue one continuation chunk without reading further frames.
    pub(super) fn push_chunk(&mut self, offer: ChunkOffer, budget: ChunkBudget) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.key != Some(offer.key) || active.pending.is_some() {
            return false;
        }
        let Some(sender) = active.sender.clone() else {
            return false;
        };
        let ChunkOffer { chunk, is_last, .. } = offer;
        active.pending = Some(PendingSend {
            future: Box::pin(async move {
                sender
                    .send_chunk(chunk, budget.other_retained, budget.limit)
                    .await
            }),
            is_last,
        });
        true
    }

    /// Record an admission result and close the body after the final chunk.
    pub(super) fn complete_send(&mut self, result: Result<(), SendChunkError>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(pending) = active.pending.take() else {
            return;
        };
        if pending.is_last || result.is_err() {
            active.sender = None;
        }
    }

    /// Finish the exchange after the handler resolved on its own.
    pub(super) async fn finish_active<S, E, F, W>(
        &mut self,
        outcome: Option<Bytes>,
        handles: DispatchHandles<'_, S, E, F, W>,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        // Cancelling the admission is safe: a reserved permit that never sent
        // releases itself, and a queued chunk is dropped with its guard.
        active.pending = None;
        active.sender = None;
        discard_streaming(active.assembly, handles.assembly);
        forward_reply(active.route_id, active.correlation_id, outcome, handles).await
    }

    /// Fail the live body, await its handler, and forward any reply.
    pub(super) async fn close_active<S, E, F, W>(
        &mut self,
        terminal: Option<io::Error>,
        handles: DispatchHandles<'_, S, E, F, W>,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        active.pending = None;
        let sender = active.sender.take();
        discard_streaming(active.assembly, handles.assembly);
        let outcome = match (terminal, sender) {
            (Some(error), Some(open_sender)) => {
                deliver_then_await(handles.wait, error, open_sender, &mut active.handler).await
            }
            _ => timeout(handles.wait, &mut active.handler)
                .await
                .ok()
                .flatten(),
        };
        forward_reply(active.route_id, active.correlation_id, outcome, handles).await
    }

    /// Fail the body for an expired streaming assembly.
    ///
    /// The evicted assemblies are matched by identity, not by key: a
    /// replacement may already have reused the key, and it must keep its
    /// own body.
    pub(super) async fn expire<S, E, F, W>(
        &mut self,
        ids: &[StreamingAssemblyId],
        handles: DispatchHandles<'_, S, E, F, W>,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        if !self.owns_assembly(ids) {
            return Ok(());
        }
        self.close_active(
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "request body assembly expired before completion",
            )),
            handles,
        )
        .await
    }

    /// Fail the body for a terminal assembly rejection.
    pub(super) async fn fail<S, E, F, W>(
        &mut self,
        key: MessageKey,
        error: io::Error,
        handles: DispatchHandles<'_, S, E, F, W>,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        if !self.is_key_live(std::slice::from_ref(&key)) {
            debug!(?key, "streaming failure for a message with no live handler");
            return Ok(());
        }
        self.close_active(Some(error), handles).await
    }

    /// Drop the live exchange immediately after a hard budget breach.
    pub(super) fn abort(&mut self) { self.active = None; }

    /// Return whether the slot owns one of the evicted assemblies.
    fn owns_assembly(&self, ids: &[StreamingAssemblyId]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.assembly)
            .is_some_and(|id| ids.contains(&id))
    }

    /// Return whether the slot is live for one of `keys`.
    fn is_key_live(&self, keys: &[MessageKey]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|key| keys.contains(&key))
    }
}

/// Switch one streaming assembly to discard mode if state is present.
///
/// The exchange names the assembly it owns, so an assembly that reused the key
/// after this one expired keeps receiving its frames.
fn discard_streaming(id: Option<StreamingAssemblyId>, assembly: &mut Option<MessageAssemblyState>) {
    if let (Some(owned), Some(assembly_state)) = (id, assembly.as_mut()) {
        assembly_state.discard_streaming_if(owned);
    }
}
