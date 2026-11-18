//! Structured diagnostics emitted while a request body streams.
//!
//! Each helper emits exactly one [`tracing`] event, and carries identifiers
//! and errors as fields rather than interpolating them into the message. The
//! helpers exist because a tracing macro expands to a level check plus field
//! handling, which counts towards Clippy's cognitive-complexity budget: a
//! function that both branches and logs would exceed it. Naming each event
//! here also keeps the wording and field names consistent across the dispatch
//! modules that raise them.
//!
//! Every event names the operation it belongs to, the message it concerns, and
//! a bounded error category, so an operator can filter one kind of body
//! failure across connections. Payload bytes are never logged; the message
//! identifier and the category carry the whole diagnosis.

use tracing::{debug, warn};

use super::stream_active::BodyIdentity;
use crate::{
    message_assembler::MessageKey,
    request::{SendChunkError, SendClosed},
};

/// Error category for a body abandoned before it completed.
const ABANDONED_BODY: &str = "abandoned_body";

/// Error category for a handler that dropped its body stream.
const CLOSED: &str = "closed";

/// Error category for a wait that expired before a chunk was admitted.
const ADMISSION_TIMEOUT: &str = "admission_timeout";

/// Error category for a wait that expired before a failure was delivered.
const DELIVERY_TIMEOUT: &str = "delivery_timeout";

/// Error category for a failure with no live handler to receive it.
const NO_HANDLER: &str = "no_handler";

/// Error category for a receiver that had already gone away.
const RECEIVER_GONE: &str = "receiver_gone";

/// Error category for a handler reply the connection stopped waiting for.
const REPLY_ABANDONED: &str = "reply_abandoned";

/// Operation name for a body delivered incrementally.
const CHUNK_DELIVERY: &str = "chunk_delivery";

/// Operation name for a body delivered as one assembled chunk.
const PREFILLED_BODY: &str = "prefilled_body";

/// Operation name for a terminal body failure.
const TERMINAL_BODY: &str = "terminal_body";

/// Operation name for opening a streaming exchange.
const STREAM_START: &str = "stream_start";

/// Report a read that timed out with no frame, leaving the loop to wait again.
pub(super) fn read_timed_out() {
    debug!("read timeout elapsed; continuing to wait for next frame");
}

/// Report a body failure that the handler's channel would not accept in time.
///
/// A handler that never reads is the usual cause: its channel is full of body
/// chunks, and only the handler can free a slot, so the failure cannot be
/// queued behind them.
pub(super) fn terminal_delivery_timed_out(identity: BodyIdentity) {
    warn!(
        operation = TERMINAL_BODY,
        message_id = identity.message_id,
        correlation_id = ?identity.correlation_id,
        error_category = DELIVERY_TIMEOUT,
        "timed out delivering the request body failure"
    );
}

/// Report a body failure whose receiver had already gone away.
pub(super) fn terminal_receiver_gone(identity: BodyIdentity, error: SendClosed) {
    debug!(
        operation = TERMINAL_BODY,
        message_id = identity.message_id,
        correlation_id = ?identity.correlation_id,
        error_category = RECEIVER_GONE,
        ?error,
        "request body receiver gone before the failure was delivered"
    );
}

/// Report a pre-filled body the handler's channel refused.
pub(super) fn prefilled_not_delivered(message_id: u32, error: SendChunkError) {
    debug!(
        operation = PREFILLED_BODY,
        message_id,
        error_category = CLOSED,
        ?error,
        "pre-filled request body was not delivered"
    );
}

/// Report a pre-filled body that did not fit the connection budget in time.
///
/// The body is failed terminally after this event, so the warning marks a
/// message the connection could not retain rather than one it dropped.
pub(super) fn prefilled_not_admitted(message_id: u32) {
    warn!(
        operation = PREFILLED_BODY,
        message_id,
        error_category = ADMISSION_TIMEOUT,
        "pre-filled request body was not admitted within the read timeout"
    );
}

/// Report a pre-filled body's terminal failure that never reached its handler.
pub(super) fn prefilled_failure_not_delivered(message_id: u32, error: SendClosed) {
    debug!(
        operation = PREFILLED_BODY,
        message_id,
        error_category = RECEIVER_GONE,
        ?error,
        "pre-filled request body failure was not delivered"
    );
}

/// Report a frame for a streaming route with no registered handler.
pub(super) fn no_streaming_handler(message_id: u32) {
    warn!(
        operation = STREAM_START,
        message_id,
        error_category = NO_HANDLER,
        "no streaming handler for message id"
    );
}

/// Report a continuation chunk dropped because its body was abandoned.
pub(super) fn abandoned_chunk(key: MessageKey) {
    debug!(
        operation = CHUNK_DELIVERY,
        ?key,
        error_category = ABANDONED_BODY,
        "dropping chunk for abandoned streaming message"
    );
}

/// Report a terminal body failure with no live handler to receive it.
///
/// The handler may have finished and released the slot between the failure
/// being decoded and the loop acting on it, so this is a warning about a
/// message that will never be answered rather than an error.
pub(super) fn no_live_handler(key: MessageKey) {
    warn!(
        operation = TERMINAL_BODY,
        ?key,
        error_category = NO_HANDLER,
        "streaming failure for a message with no live handler"
    );
}

/// Report a handler that outlived the connection's wait for its reply.
///
/// The reply is abandoned at this point, so the deadline is the only evidence
/// that the handler did not simply choose to stay silent.
pub(super) fn handler_deadline_exceeded(route_id: u32) {
    warn!(
        operation = TERMINAL_BODY,
        message_id = route_id,
        error_category = REPLY_ABANDONED,
        "streaming handler exceeded the connection wait; its reply is abandoned"
    );
}
