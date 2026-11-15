//! Dispatch of messages parked behind a live streaming body.
//!
//! A connection admits one streaming request body at a time. Any message that
//! completes while that slot is busy is queued here and dispatched in arrival
//! order once the slot frees, so a client that pipelines requests still sees
//! one reply order regardless of which route each message targets.
//!
//! A buffered message becomes an [`Envelope`] and reuses the normal middleware
//! chain. A message for a streaming route that finished while the slot was
//! busy keeps its routing metadata and assembled body, then runs through its
//! streaming handler with a pre-filled, single-chunk body stream. This keeps
//! one delivery contract for streaming handlers while preserving the ordering
//! guarantee that motivated parking.

use std::{collections::VecDeque, io};

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    super::{
        codec_driver::FramePipeline,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
        frame_handling::{self, ResponseContext},
    },
    stream_dispatch::DispatchContext,
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::MessageAssemblyState,
    request::{DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes, RequestParts, accounted_body_channel},
    serializer::Serializer,
};

/// A completed message held until the dispatch slot frees.
pub(super) enum ParkedMessage {
    /// Complete message for a buffered route.
    Envelope(Envelope),
    /// Complete message for a streaming route, delivered as a pre-filled body.
    Stream {
        /// Routing metadata captured during assembly.
        parts: RequestParts,
        /// Fully assembled request body.
        body: Bytes,
    },
}

/// Return the bytes retained by parked messages.
pub(super) fn parked_bytes(parked: &VecDeque<ParkedMessage>) -> usize {
    parked
        .iter()
        .map(|message| match message {
            ParkedMessage::Envelope(env) => env.payload.len(),
            ParkedMessage::Stream { parts, body } => {
                parts.metadata().len().saturating_add(body.len())
            }
        })
        .sum()
}

/// Dispatch a parked message through its buffered service or streaming handler.
pub(super) async fn dispatch_parked<S, E, F, W>(
    message: ParkedMessage,
    ctx: &DispatchContext<'_, S, E, F>,
    pipeline: &mut FramePipeline<F::Frame>,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    assembly: Option<&MessageAssemblyState>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    match message {
        ParkedMessage::Envelope(env) => {
            let Some(service) = ctx.routes.get(&env.id) else {
                log::warn!(
                    "no handler for message id: id={}, correlation_id={:?}",
                    env.id,
                    env.correlation_id
                );
                return Ok(());
            };
            frame_handling::forward_response(
                env,
                service,
                ResponseContext {
                    serializer: ctx.serializer,
                    framed,
                    pipeline,
                    codec: ctx.codec,
                },
            )
            .await
        }
        ParkedMessage::Stream { parts, body } => {
            dispatch_prefilled(parts, body, ctx, pipeline, framed, assembly).await
        }
    }
}

/// Run a streaming handler with an already assembled body.
async fn dispatch_prefilled<S, E, F, W>(
    parts: RequestParts,
    body: Bytes,
    ctx: &DispatchContext<'_, S, E, F>,
    pipeline: &mut FramePipeline<F::Frame>,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    assembly: Option<&MessageAssemblyState>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let Some(handler) = ctx.streaming_routes.get(&parts.id()) else {
        log::warn!("no streaming handler for message id: id={}", parts.id());
        return Ok(());
    };
    let route_id = parts.id();
    let correlation_id = parts.correlation_id();
    let (sender, stream) =
        accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes::new());
    let response = handler(parts, stream);
    let other_retained = assembly
        .map_or(0, MessageAssemblyState::total_buffered_bytes)
        .saturating_add(pipeline.fragmentation_buffered_bytes());
    let limit = frame_handling::active_aggregate_limit_bytes(ctx.effective_budgets);
    // The queue starts empty, so the empty-queue exception admits the whole
    // body even when it exceeds the remaining aggregate budget. Assembly
    // already enforced the per-message cap.
    let _ = sender.clone().send_chunk(body, other_retained, limit).await;
    drop(sender);
    let payload = response.await;
    forward_reply(route_id, correlation_id, payload, pipeline, framed, ctx).await
}

/// Send a streaming handler's reply through the outbound pipeline.
pub(super) async fn forward_reply<S, E, F, W>(
    route_id: u32,
    correlation_id: Option<u64>,
    payload: Option<Vec<u8>>,
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
    frame_handling::forward_service_response(
        route_id,
        correlation_id,
        payload,
        ResponseContext {
            serializer: ctx.serializer,
            framed,
            pipeline,
            codec: ctx.codec,
        },
    )
    .await
}
