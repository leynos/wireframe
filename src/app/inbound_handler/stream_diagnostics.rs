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

use std::time::Duration;

use tracing::{debug, warn};

use super::stream_active::BodyIdentity;
use crate::{
    message_assembler::MessageKey,
    metrics::StreamingBodyOutcome,
    request::{SendChunkError, SendClosed},
};

/// Error category for a body abandoned before it completed.
const ABANDONED_BODY: &str = "abandoned_body";

/// Error category for a handler that dropped its body stream.
const CLOSED: &str = "closed";

/// Error category for a wait that expired before a chunk was admitted.
///
/// The three categories that name a recorded body outcome are taken from the
/// outcome itself, so the counter series and the event cannot drift apart.
const ADMISSION_TIMEOUT: &str = StreamingBodyOutcome::AdmissionTimeout.as_str();

/// Error category for a wait that expired before a failure was delivered.
const DELIVERY_TIMEOUT: &str = StreamingBodyOutcome::DeliveryTimeout.as_str();

/// Error category for a failure with no live handler to receive it.
const NO_HANDLER: &str = StreamingBodyOutcome::NoHandler.as_str();

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

/// A delivery the handler's closed channel ended.
///
/// The terminal failure and the refused final chunk differ only in what was
/// being delivered and which error type reported the closure, so both are one
/// value here rather than two copies of the same field list.
#[derive(Clone, Copy, Debug)]
enum ClosedDelivery {
    /// The terminal failure could not be handed to the handler.
    TerminalFailure,
    /// The final chunk was refused, so the body is abandoned.
    FinalChunk,
}

impl ClosedDelivery {
    /// Operation, bounded category, and message for this closure.
    const fn describe(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::TerminalFailure => (
                TERMINAL_BODY,
                RECEIVER_GONE,
                "request body receiver gone before the failure was delivered",
            ),
            Self::FinalChunk => (
                CHUNK_DELIVERY,
                ABANDONED_BODY,
                "final request-body chunk refused; the body is abandoned",
            ),
        }
    }
}

/// Report a delivery the handler's closed channel ended.
fn channel_closed(identity: BodyIdentity, delivery: ClosedDelivery, error: &dyn std::fmt::Debug) {
    let (operation, category, message) = delivery.describe();
    debug!(
        operation,
        message_id = identity.message_id,
        correlation_id = ?identity.correlation_id,
        error_category = category,
        error = ?error,
        "{message}"
    );
}

/// Report a body failure whose receiver had already gone away.
pub(super) fn terminal_receiver_gone(identity: BodyIdentity, error: SendClosed) {
    channel_closed(identity, ClosedDelivery::TerminalFailure, &error);
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

/// Report a final chunk the handler's channel refused.
///
/// A final chunk is the last thing a body sends, so nothing follows it to
/// report what became of the body: this event and the `abandoned` counter
/// increment recorded beside it are the whole signal. The level is `debug`
/// because the handler abandoned the body by dropping its stream.
pub(super) fn final_chunk_refused(identity: BodyIdentity, error: SendChunkError) {
    channel_closed(identity, ClosedDelivery::FinalChunk, &error);
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

/// Report a body chunk that could not be admitted within the connection's wait.
///
/// The chunk is retried on every step, so this is not a failure on its own; it
/// is the point where a handler that never reads becomes visible, because only
/// the handler can make room in its own channel.
pub(super) fn admission_timed_out(identity: BodyIdentity, wait: Duration) {
    report_admission(identity, wait, false);
}

/// Report a body abandoned because its admission made no progress.
///
/// The stall bound is longer than one wait: a chunk that cannot be admitted is
/// retried, and this is the accumulated interval that ends the body so the loop
/// can reach the peer again.
pub(super) fn body_stalled(identity: BodyIdentity, elapsed: Duration) {
    report_admission(identity, elapsed, true);
}

/// Emit the one event an admission wait produces.
///
/// Both endings share the operation, identity, and category; they differ in
/// which field names the interval and in whether the body is still alive.
fn report_admission(identity: BodyIdentity, interval: Duration, is_stalled: bool) {
    let millis = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let BodyIdentity {
        message_id,
        correlation_id,
    } = identity;
    if is_stalled {
        warn!(
            operation = CHUNK_DELIVERY,
            message_id,
            correlation_id = ?correlation_id,
            error_category = ADMISSION_TIMEOUT,
            elapsed_ms = millis,
            "request body admission made no progress"
        );
    } else {
        warn!(
            operation = CHUNK_DELIVERY,
            message_id,
            correlation_id = ?correlation_id,
            error_category = ADMISSION_TIMEOUT,
            wait_ms = millis,
            "request body chunk waited for admission without settling"
        );
    }
}

/// Report a final chunk the handler did not admit before the shared deadline.
///
/// The body is abandoned at this point and its reply suppressed, so no later
/// record of it can exist.
pub(super) fn final_chunk_timed_out(identity: BodyIdentity) {
    warn!(
        operation = CHUNK_DELIVERY,
        message_id = identity.message_id,
        correlation_id = ?identity.correlation_id,
        error_category = ABANDONED_BODY,
        "final request body chunk was not admitted before the deadline"
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

#[cfg(test)]
#[path = "stream_diagnostics_tests.rs"]
mod tests;
