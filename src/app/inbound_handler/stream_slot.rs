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
use log::debug;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};
use tokio_util::codec::Framed;

use super::{
    super::{
        StreamingHandler,
        codec_driver::FramePipeline,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
    },
    stream_dispatch::{DispatchContext, Event, read_frame},
    stream_parked::forward_reply,
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::{MessageAssemblyState, MessageKey},
    request::{
        AccountedBodySender,
        DEFAULT_BODY_CHANNEL_CAPACITY,
        QueuedBytes,
        RequestParts,
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
    pub(super) async fn drive_pending(&mut self) -> PendingEvent {
        let Self {
            pending, handler, ..
        } = self;
        match pending {
            Some(pending) => {
                tokio::select! {
                    result = &mut pending.future => PendingEvent::Send(result),
                    outcome = handler.as_mut() => PendingEvent::Handler(outcome),
                }
            }
            // The caller only drives this branch when a send is pending.
            None => PendingEvent::Send(Ok(())),
        }
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

    /// Start a streaming exchange, queueing the first chunk immediately.
    ///
    /// The first chunk is sent on a freshly created channel, so admission
    /// cannot wait: the queue is empty and the channel has spare capacity. A
    /// closed consumer simply leaves the handler to observe an empty body.
    pub(super) async fn start(
        &mut self,
        key: Option<MessageKey>,
        parts: RequestParts,
        chunk: Bytes,
        is_last: bool,
        handler: &StreamingHandler,
        other_retained: usize,
        limit: usize,
    ) {
        let route_id = parts.id();
        let correlation_id = parts.correlation_id();
        let (sender, body) =
            accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, self.queued.clone());
        let future = handler(parts, body);
        let sent = sender
            .clone()
            .send_chunk(chunk, other_retained, limit)
            .await;
        let sender = if sent.is_ok() && !is_last {
            Some(sender)
        } else {
            None
        };
        self.active = Some(ActiveStream {
            key,
            route_id,
            correlation_id,
            sender,
            handler: future,
            pending: None,
        });
    }

    /// Queue one continuation chunk without reading further frames.
    pub(super) fn push_chunk(
        &mut self,
        key: MessageKey,
        chunk: Bytes,
        is_last: bool,
        other_retained: usize,
        limit: usize,
    ) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.key != Some(key) || active.pending.is_some() {
            return false;
        }
        let Some(sender) = active.sender.clone() else {
            return false;
        };
        active.pending = Some(PendingSend {
            future: Box::pin(async move { sender.send_chunk(chunk, other_retained, limit).await }),
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
        assembly: &mut Option<MessageAssemblyState>,
        pipeline: &mut FramePipeline<F::Frame>,
        framed: &mut Framed<W, ConnectionCodec<F>>,
        ctx: &DispatchContext<'_, S, E, F>,
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
        if let (Some(key), Some(state)) = (active.key, assembly.as_mut()) {
            state.discard_streaming(key);
        }
        forward_reply(
            active.route_id,
            active.correlation_id,
            outcome,
            pipeline,
            framed,
            ctx,
        )
        .await
    }

    /// Fail the live body, await its handler, and forward any reply.
    pub(super) async fn close_active<S, E, F, W>(
        &mut self,
        terminal: Option<io::Error>,
        assembly: &mut Option<MessageAssemblyState>,
        pipeline: &mut FramePipeline<F::Frame>,
        framed: &mut Framed<W, ConnectionCodec<F>>,
        ctx: &DispatchContext<'_, S, E, F>,
        wait: Duration,
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
        if let (Some(error), Some(sender)) = (terminal, active.sender.clone()) {
            let _ = timeout(wait, sender.send_terminal(error)).await;
        }
        active.sender = None;
        if let (Some(key), Some(state)) = (active.key, assembly.as_mut()) {
            state.discard_streaming(key);
        }
        let outcome = timeout(wait, &mut active.handler).await.ok().flatten();
        forward_reply(
            active.route_id,
            active.correlation_id,
            outcome,
            pipeline,
            framed,
            ctx,
        )
        .await
    }

    /// Fail the body for an expired streaming assembly.
    pub(super) async fn expire<S, E, F, W>(
        &mut self,
        keys: &[MessageKey],
        assembly: &mut Option<MessageAssemblyState>,
        pipeline: &mut FramePipeline<F::Frame>,
        framed: &mut Framed<W, ConnectionCodec<F>>,
        ctx: &DispatchContext<'_, S, E, F>,
        wait: Duration,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        let is_expired = self
            .active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|key| keys.contains(&key));
        if !is_expired {
            return Ok(());
        }
        self.close_active(
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "request body assembly expired before completion",
            )),
            assembly,
            pipeline,
            framed,
            ctx,
            wait,
        )
        .await
    }

    /// Fail the body for a terminal assembly rejection.
    pub(super) async fn fail<S, E, F, W>(
        &mut self,
        key: MessageKey,
        error: io::Error,
        assembly: &mut Option<MessageAssemblyState>,
        pipeline: &mut FramePipeline<F::Frame>,
        framed: &mut Framed<W, ConnectionCodec<F>>,
        ctx: &DispatchContext<'_, S, E, F>,
        wait: Duration,
    ) -> io::Result<()>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
        Envelope: EncodeWith<S>,
    {
        let matches_active = self
            .active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|active_key| active_key == key);
        if !matches_active {
            debug!("streaming failure for a message with no live handler: key={key:?}");
            return Ok(());
        }
        self.close_active(Some(error), assembly, pipeline, framed, ctx, wait)
            .await
    }

    /// Drop the live exchange immediately after a hard budget breach.
    pub(super) fn abort(&mut self) { self.active = None; }
}
