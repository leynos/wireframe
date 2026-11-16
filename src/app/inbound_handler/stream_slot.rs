//! Connection-owned streaming state: the dispatch slot and byte account.
//!
//! The slot owns everything that outlives a single inbound frame while a
//! request body streams: the handler future, the byte-accounted sender, and at
//! most one in-flight chunk admission. Keeping them together means the loop
//! never has to reconstruct a partially consumed exchange, and dropping the
//! slot cancels the handler and releases queued bytes in one step.

use std::{io, pin::Pin, time::Duration};

use futures::Future;
use log::debug;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};
use tokio_util::codec::Framed;

use super::{
    super::{
        StreamingHandler,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
    },
    stream_dispatch::{Event, read_frame},
    stream_handles::{ChunkBudget, ChunkOffer, DispatchHandles, FirstChunk},
    stream_parked::forward_reply,
};
#[cfg(test)]
#[path = "stream_slot_tests.rs"]
mod tests;

use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::{MessageAssemblyState, MessageKey},
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
pub(super) type HandlerFuture = Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'static>>;

/// One step while a chunk admission is in flight.
pub(super) enum PendingEvent {
    /// The chunk was queued, or the consumer had gone away.
    Send(Result<(), SendChunkError>),
    /// The handler resolved first; the admission is cancelled.
    Handler(Option<Vec<u8>>),
    /// The bounded wait elapsed with the admission still queued.
    Idle,
}

/// The one live streaming exchange on a connection.
pub(super) struct ActiveStream {
    /// Assembly key used to discard remaining frames after abandonment.
    key: Option<MessageKey>,
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

    /// Start a streaming exchange, queueing the first chunk immediately.
    ///
    /// The first chunk is sent on a freshly created channel, so admission
    /// cannot wait: the queue is empty and the channel has spare capacity. A
    /// closed consumer simply leaves the handler to observe an empty body.
    pub(super) async fn start(
        &mut self,
        first: FirstChunk,
        handler: &StreamingHandler,
        budget: ChunkBudget,
    ) {
        let FirstChunk {
            key,
            parts,
            chunk,
            is_last,
        } = first;
        let route_id = parts.id();
        let correlation_id = parts.correlation_id();
        let (sender, body) =
            accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, self.queued.clone());
        let handler_future = handler(parts, body);
        let sent = sender
            .clone()
            .send_chunk(chunk, budget.other_retained, budget.limit)
            .await;
        let live_sender = if sent.is_ok() && !is_last {
            Some(sender)
        } else {
            None
        };
        self.active = Some(ActiveStream {
            key,
            route_id,
            correlation_id,
            sender: live_sender,
            handler: handler_future,
            pending: None,
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
        outcome: Option<Vec<u8>>,
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
        discard_streaming(active.key, handles.assembly);
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
        discard_streaming(active.key, handles.assembly);
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
    pub(super) async fn expire<S, E, F, W>(
        &mut self,
        keys: &[MessageKey],
        handles: DispatchHandles<'_, S, E, F, W>,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        if !self.is_key_live(keys) {
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
            debug!("streaming failure for a message with no live handler: key={key:?}");
            return Ok(());
        }
        self.close_active(Some(error), handles).await
    }

    /// Drop the live exchange immediately after a hard budget breach.
    pub(super) fn abort(&mut self) { self.active = None; }

    /// Return whether the slot is live for one of `keys`.
    fn is_key_live(&self, keys: &[MessageKey]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|key| keys.contains(&key))
    }
}

/// Switch one streaming key to discard mode if assembly state is present.
fn discard_streaming(key: Option<MessageKey>, assembly: &mut Option<MessageAssemblyState>) {
    if let (Some(live_key), Some(assembly_state)) = (key, assembly.as_mut()) {
        assembly_state.discard_streaming(live_key);
    }
}

/// Deliver a terminal body error while the handler drains, then await it.
///
/// The error needs a slot on the same bounded channel that carries body chunks,
/// so delivering it before polling the handler would deadlock whenever that
/// channel is full: only the handler can free a slot, and it is not running.
/// The timeout on the delivery would then drop the error, the sender would
/// close, and a truncated body would look like a clean end of stream.
///
/// Polling both together lets the handler take the queued chunks and then
/// observe the failure. The wait bounds the whole exchange, so a handler that
/// never reads still cannot hold the connection open.
async fn deliver_then_await(
    wait: Duration,
    error: io::Error,
    sender: AccountedBodySender,
    handler: &mut HandlerFuture,
) -> Option<Vec<u8>> {
    let delivery = deliver_terminal(wait, error, sender);
    tokio::pin!(delivery);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut is_delivered = false;
    loop {
        tokio::select! {
            () = &mut delivery, if !is_delivered => is_delivered = true,
            outcome = handler.as_mut() => return outcome,
            () = &mut deadline => return None,
        }
    }
}

/// Tell the handler that its body failed, without waiting indefinitely.
async fn deliver_terminal(wait: Duration, error: io::Error, sender: AccountedBodySender) {
    match timeout(wait, sender.send_terminal(error)).await {
        Ok(Ok(())) => {}
        Ok(Err(send_error)) => {
            debug!("request body receiver gone before the failure was delivered: {send_error:?}");
        }
        Err(_) => debug!("timed out delivering the request body failure"),
    }
}
