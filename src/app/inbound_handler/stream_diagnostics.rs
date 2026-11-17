//! Structured diagnostics emitted while a request body streams.
//!
//! Each helper emits exactly one [`tracing`] event, and carries identifiers
//! and errors as fields rather than interpolating them into the message. The
//! helpers exist because a tracing macro expands to a level check plus field
//! handling, which counts towards Clippy's cognitive-complexity budget: a
//! function that both branches and logs would exceed it. Naming each event
//! here also keeps the wording and field names consistent across the dispatch
//! modules that raise them.

use tracing::{debug, warn};

use crate::request::{SendChunkError, SendClosed};

/// Report a read that timed out with no frame, leaving the loop to wait again.
pub(super) fn read_timed_out() {
    debug!("read timeout elapsed; continuing to wait for next frame");
}

/// Report a body failure that the handler's channel would not accept in time.
pub(super) fn terminal_delivery_timed_out() {
    debug!("timed out delivering the request body failure");
}

/// Report a body failure whose receiver had already gone away.
pub(super) fn terminal_receiver_gone(error: SendClosed) {
    debug!(
        error = ?error,
        "request body receiver gone before the failure was delivered"
    );
}

/// Report a pre-filled body the handler's channel refused.
pub(super) fn prefilled_not_delivered(error: SendChunkError) {
    debug!(?error, "pre-filled request body was not delivered");
}

/// Report a pre-filled body that did not fit the connection budget in time.
pub(super) fn prefilled_not_admitted() {
    debug!("pre-filled request body was not admitted within the read timeout");
}

/// Report a pre-filled body's terminal failure that never reached its handler.
pub(super) fn prefilled_failure_not_delivered(error: SendClosed) {
    debug!(?error, "pre-filled request body failure was not delivered");
}

/// Report a handler that outlived the connection's wait for its reply.
///
/// The reply is abandoned at this point, so the deadline is the only evidence
/// that the handler did not simply choose to stay silent.
pub(super) fn handler_deadline_exceeded(route_id: u32) {
    warn!(
        route_id,
        "streaming handler exceeded the connection wait; its reply is abandoned"
    );
}
