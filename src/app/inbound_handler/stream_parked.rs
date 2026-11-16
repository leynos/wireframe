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
use log::debug;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::{self, ResponseContext},
    },
    stream_handles::{ChunkBudget, DispatchHandles},
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    request::{DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes, RequestParts, accounted_body_channel},
    serializer::Serializer,
};

#[cfg(test)]
#[path = "stream_parked_tests.rs"]
mod tests;

/// Maximum number of completed messages parked behind a live streaming body.
///
/// Parking exists so a pipelining peer keeps its reply order, not so a peer
/// that never finishes its body can accumulate work indefinitely. The
/// byte-based memory budget cannot see this queue grow when the parked payloads
/// are small or empty, so the entry count is the limit that actually bounds it.
pub(super) const MAX_PARKED_MESSAGES: usize = 64;

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

/// Park one completed message, refusing to grow the queue past its cap.
///
/// # Errors
///
/// Returns an invalid-data error once [`MAX_PARKED_MESSAGES`] entries are
/// already waiting. The caller ends the connection in that case, because the
/// alternative is to absorb messages from a peer that never completes the body
/// holding the dispatch slot.
pub(super) fn park(parked: &mut VecDeque<ParkedMessage>, message: ParkedMessage) -> io::Result<()> {
    if parked.len() >= MAX_PARKED_MESSAGES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too many messages parked behind a streaming request body",
        ));
    }
    parked.push_back(message);
    Ok(())
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
///
/// `parked` is the queue still waiting behind this message. Its bytes stay
/// reserved while the message is answered, so a pre-filled body is admitted
/// against the same aggregate account as every other retained byte.
pub(super) async fn dispatch_parked<S, E, F, W>(
    message: ParkedMessage,
    parked: &VecDeque<ParkedMessage>,
    handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    match message {
        ParkedMessage::Envelope(env) => forward_buffered(env, handles).await,
        ParkedMessage::Stream { parts, body } => {
            dispatch_prefilled(parts, body, parked, handles).await
        }
    }
}

/// Run a parked buffered envelope through its middleware chain.
async fn forward_buffered<S, E, F, W>(
    env: Envelope,
    handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let Some(service) = handles.ctx.routes.get(&env.id) else {
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
            serializer: handles.ctx.serializer,
            framed: handles.framed,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
        },
    )
    .await
}

/// Run a streaming handler with an already assembled body.
async fn dispatch_prefilled<S, E, F, W>(
    parts: RequestParts,
    body: Bytes,
    parked: &VecDeque<ParkedMessage>,
    handles: DispatchHandles<'_, S, E, F, W>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: EncodeWith<S>,
{
    let Some(streaming_handler) = handles.ctx.streaming_routes.get(&parts.id()) else {
        log::warn!("no streaming handler for message id: id={}", parts.id());
        return Ok(());
    };
    let route_id = parts.id();
    let correlation_id = parts.correlation_id();
    let (sender, stream) =
        accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes::new());
    let response = streaming_handler(parts, stream);
    let budget = ChunkBudget::measure(&handles, parked);
    // The queue starts empty, so the empty-queue exception admits the whole
    // body even when it exceeds the remaining aggregate budget. Assembly
    // already enforced the per-message cap.
    if let Err(error) = sender
        .clone()
        .send_chunk(body, budget.other_retained, budget.limit)
        .await
    {
        debug!("pre-filled request body was not delivered: {error:?}");
    }
    drop(sender);
    let payload = response.await;
    forward_reply(route_id, correlation_id, payload, handles).await
}

/// Send a streaming handler's reply through the outbound pipeline.
pub(super) async fn forward_reply<S, E, F, W>(
    route_id: u32,
    correlation_id: Option<u64>,
    payload: Option<Vec<u8>>,
    handles: DispatchHandles<'_, S, E, F, W>,
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
            serializer: handles.ctx.serializer,
            framed: handles.framed,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
        },
    )
    .await
}
