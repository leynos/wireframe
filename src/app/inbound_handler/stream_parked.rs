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

use std::{
    collections::{HashMap, VecDeque},
    io,
};

use bytes::Bytes;
use futures::Sink;
use tokio::time::{Instant, timeout_at};
use tracing::warn;

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch, ResponseContext},
    },
    stream_diagnostics,
    stream_handles::{ChunkBudget, DispatchHandles},
    stream_terminal::{Delivery, HandlerWait, race_delivery},
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    metrics::StreamingBodyOutcome,
    request::{
        AccountedBodySender,
        DEFAULT_BODY_CHANNEL_CAPACITY,
        RequestParts,
        accounted_body_channel,
    },
    serializer::Serializer,
};

#[cfg(test)]
#[path = "stream_parked_tests.rs"]
mod tests;

#[cfg(test)]
#[cfg(feature = "metrics")]
#[path = "stream_outcome_parked_tests.rs"]
mod outcome_tests;

#[cfg(test)]
#[path = "stream_parked_accounting_tests.rs"]
mod accounting_tests;

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

/// Completed messages waiting for the dispatch slot.
///
/// The queue is pure storage. It neither publishes the process-wide
/// parked-bodies gauge nor releases anything when it drops: the commands that
/// park, drain and tear down messages own that accounting, so the effect is
/// visible at the operation that causes it rather than hidden inside a
/// collection.
#[derive(Default)]
pub(super) struct ParkedQueue {
    /// Messages in arrival order.
    messages: VecDeque<ParkedMessage>,
}

impl ParkedQueue {
    /// Create an empty queue.
    pub(super) const fn new() -> Self {
        Self {
            messages: VecDeque::new(),
        }
    }

    /// Return the number of queued messages.
    pub(super) fn len(&self) -> usize { self.messages.len() }

    /// Iterate over the queued messages in arrival order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &ParkedMessage> { self.messages.iter() }

    /// Remove the oldest message.
    ///
    /// The caller reports the removal to the parked-bodies gauge, because only
    /// it knows whether the message was dispatched or dropped.
    pub(super) fn pop_front(&mut self) -> Option<ParkedMessage> { self.messages.pop_front() }

    /// Append one message.
    fn push_back(&mut self, message: ParkedMessage) { self.messages.push_back(message); }
}

/// Park one completed message, refusing to grow the queue past its cap.
///
/// The queue and this command are pure storage: reporting the new parked count
/// belongs to the connection state that owns the queue and the gauge sink.
///
/// # Errors
///
/// Returns an invalid-data error once [`MAX_PARKED_MESSAGES`] entries are
/// already waiting. The caller ends the connection in that case, because the
/// alternative is to absorb messages from a peer that never completes the body
/// holding the dispatch slot.
pub(super) fn park(parked: &mut ParkedQueue, message: ParkedMessage) -> io::Result<()> {
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
pub(super) fn parked_bytes(parked: &ParkedQueue) -> usize {
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
pub(super) async fn dispatch_parked<S, E, K, F>(
    message: ParkedMessage,
    parked: &ParkedQueue,
    handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
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
async fn forward_buffered<S, E, K, F>(
    env: Envelope,
    handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let Some(service) = handles.ctx.routes.get(&env.id) else {
        warn!(
            message_id = env.id,
            correlation_id = ?env.correlation_id,
            "no handler for message id"
        );
        return Ok(Dispatch::Continue);
    };
    frame_handling::forward_response(
        env,
        service,
        ResponseContext {
            serializer: handles.ctx.serializer,
            sink: handles.sink,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
            actor: handles.actor,
        },
    )
    .await
}

/// Look up the streaming handler a parked message is addressed to.
///
/// The lookup is pure: a miss reports nothing and counts nothing, so the
/// caller that drops the message is the one place the refusal is recorded.
pub(super) fn lookup_streaming_handler(
    streaming_routes: &HashMap<u32, StreamingHandler>,
    route_id: u32,
) -> Option<StreamingHandler> {
    streaming_routes.get(&route_id).cloned()
}

/// Refuse a parked message whose route has no streaming handler.
///
/// The refusal is a command in its own right: the message is dropped, the
/// event and the counter carry one `no_handler` vocabulary shared with the live
/// start path, and the pure lookup above stays free of both.
pub(super) fn refuse_unrouted(route_id: u32) {
    stream_diagnostics::no_streaming_handler(route_id);
    StreamingBodyOutcome::NoHandler.record();
}

/// One assembled body waiting to reach a parked streaming handler.
pub(super) struct PrefilledOffer {
    /// The whole assembled request body.
    pub(super) body: Bytes,
    /// Route identifier of the message, carried into diagnostics.
    pub(super) message_id: u32,
}

/// Run a streaming handler with an already assembled body.
async fn dispatch_prefilled<S, E, K, F>(
    parts: RequestParts,
    body: Bytes,
    parked: &ParkedQueue,
    handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let route_id = parts.id();
    let Some(streaming_handler) = lookup_streaming_handler(handles.ctx.streaming_routes, route_id)
    else {
        // A route with no streaming handler cannot answer, so the message is
        // refused rather than left waiting for a handler that does not exist.
        refuse_unrouted(route_id);
        return Ok(Dispatch::Continue);
    };
    let correlation_id = parts.correlation_id();
    let (sender, stream) =
        accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, handles.queued.clone());
    let mut response = streaming_handler(parts, stream);
    let budget = ChunkBudget::measure(&handles, parked);
    let offer = PrefilledOffer {
        body,
        message_id: route_id,
    };
    // The delivery and the reply share one deadline, and the handler is polled
    // while the delivery waits: only the handler can release capacity that the
    // shared account is missing, so awaiting the delivery alone would time out
    // a body the handler was about to make room for.
    let deadline = Instant::now() + handles.wait;
    let settled = race_delivery(
        deadline,
        deliver_prefilled(sender, offer, budget, deadline),
        &mut response,
        StreamingBodyOutcome::Abandoned,
    )
    .await;
    let payload = parked_reply(settled, route_id);
    forward_reply(route_id, correlation_id, payload, handles).await
}

/// Turn a settled parked race into the reply the connection may forward.
///
/// A handler that outlived the wait produced no answer, and the warning is the
/// only evidence that it did not simply choose to stay silent.
fn parked_reply(settled: HandlerWait, route_id: u32) -> Option<Bytes> {
    match settled {
        HandlerWait::Replied(reply) => reply,
        HandlerWait::Deadline => {
            stream_diagnostics::handler_deadline_exceeded(route_id);
            None
        }
    }
}

/// Queue a fully assembled body as the only chunk of its stream.
///
/// The sender is owned rather than borrowed: finishing this future drops it,
/// which is what closes the body, so a handler polled alongside the delivery
/// observes the end of its stream and can reply.
///
/// The queue starts empty, so the empty-queue exception admits the whole body
/// even when it exceeds the remaining aggregate budget; assembly already
/// enforced the per-message cap. The wait is still bounded, because the shared
/// byte account can hold bytes an earlier body never released, and those bytes
/// cannot be reclaimed while this call blocks the loop that would answer the
/// messages behind this one. A body that cannot be admitted becomes a terminal
/// error, so the handler learns the upload was incomplete instead of waiting
/// for a chunk that will never arrive.
async fn deliver_prefilled(
    sender: AccountedBodySender,
    offer: PrefilledOffer,
    budget: ChunkBudget,
    deadline: Instant,
) -> Delivery {
    let PrefilledOffer { body, message_id } = offer;
    let sent = timeout_at(
        deadline,
        sender
            .clone()
            .send_chunk(body, budget.other_retained, budget.limit),
    )
    .await;
    match sent {
        Ok(Ok(())) => {
            StreamingBodyOutcome::Delivered.record();
            Delivery::Accepted
        }
        Ok(Err(error)) => {
            stream_diagnostics::prefilled_not_delivered(message_id, error);
            StreamingBodyOutcome::Abandoned.record();
            Delivery::ReceiverGone
        }
        Err(_) => {
            refuse_prefilled(&sender, message_id).await;
            Delivery::TimedOut
        }
    }
}

/// Report a body the connection could not retain, and fail it terminally.
///
/// Without the terminal error the handler would wait for a chunk that will
/// never arrive, because the whole body was refused rather than split.
async fn refuse_prefilled(sender: &AccountedBodySender, message_id: u32) {
    stream_diagnostics::prefilled_not_admitted(message_id);
    StreamingBodyOutcome::AdmissionTimeout.record();
    let error = io::Error::other("request body could not be retained within the connection budget");
    if let Err(closed) = sender.clone().send_terminal(error).await {
        stream_diagnostics::prefilled_failure_not_delivered(message_id, closed);
        StreamingBodyOutcome::Abandoned.record();
    }
}

/// Send a streaming handler's reply through the outbound pipeline.
///
/// The handler produces [`Bytes`], while the middleware and envelope reply
/// path still carries `Vec<u8>` pending the ADR 009 migration, so the payload
/// is converted once at this boundary.
pub(super) async fn forward_reply<S, E, K, F>(
    route_id: u32,
    correlation_id: Option<u64>,
    payload: Option<Bytes>,
    handles: DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    frame_handling::forward_service_response(
        route_id,
        correlation_id,
        payload,
        ResponseContext {
            serializer: handles.ctx.serializer,
            sink: handles.sink,
            pipeline: handles.pipeline,
            codec: handles.ctx.codec,
            actor: handles.actor,
        },
    )
    .await
}
