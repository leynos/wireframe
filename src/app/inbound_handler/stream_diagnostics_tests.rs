//! The documented vocabulary of streaming request-body diagnostics.
//!
//! Each helper in [`stream_diagnostics`](super::stream_diagnostics) emits one
//! event, and the users' guide and ADR 018 quote the message and error
//! category of every one. An operator filters on those two fields, so a
//! renamed message or a category that drifts from its counter label would
//! leave the guide describing events the code no longer emits.
//!
//! The paths that raise the events are covered by their own tests, which
//! assert the event where the state is constructible: a refused final chunk,
//! an unaffordable pre-filled body, and a handler that outlives the reply
//! wait. This module covers the rest of the vocabulary directly, including the
//! cases whose connection state is awkward to build in a unit test.

use tracing_test::traced_test;

use super::{
    abandoned_chunk,
    final_chunk_refused,
    handler_deadline_exceeded,
    no_live_handler,
    no_streaming_handler,
    prefilled_failure_not_delivered,
    prefilled_not_admitted,
    prefilled_not_delivered,
    read_timed_out,
    terminal_delivery_timed_out,
    terminal_receiver_gone,
};
use crate::{
    app::inbound_handler::stream_active::BodyIdentity,
    message_assembler::MessageKey,
    request::{SendChunkError, SendClosed},
};

/// Message and bounded `error_category` every helper must emit.
///
/// The category names a body outcome, a delivery outcome, or the closed
/// channel that ended the delivery; payload bytes never appear in either
/// field, so an operator can filter on them without application data.
const EVENTS: [(&str, &str); 10] = [
    (
        "timed out delivering the request body failure",
        "delivery_timeout",
    ),
    (
        "request body receiver gone before the failure was delivered",
        "receiver_gone",
    ),
    ("pre-filled request body was not delivered", "closed"),
    (
        "pre-filled request body was not admitted",
        "admission_timeout",
    ),
    (
        "pre-filled request body failure was not delivered",
        "receiver_gone",
    ),
    ("no streaming handler for message id", "no_handler"),
    (
        "dropping chunk for abandoned streaming message",
        "abandoned_body",
    ),
    (
        "streaming failure for a message with no live handler",
        "no_handler",
    ),
    (
        "streaming handler exceeded the connection wait",
        "reply_abandoned",
    ),
    ("final request-body chunk refused", "abandoned_body"),
];

/// Identity every event in this module reports under.
fn identity() -> BodyIdentity {
    BodyIdentity {
        message_id: 7,
        correlation_id: Some(11),
    }
}

/// Emit one event from every helper in the vocabulary.
fn emit_every_event() {
    let identity = identity();
    let key = MessageKey(62);
    read_timed_out();
    terminal_delivery_timed_out(identity);
    terminal_receiver_gone(identity, SendClosed);
    prefilled_not_delivered(1, SendChunkError::Closed);
    prefilled_not_admitted(2);
    prefilled_failure_not_delivered(3, SendClosed);
    no_streaming_handler(4);
    abandoned_chunk(key);
    no_live_handler(key);
    handler_deadline_exceeded(5);
    final_chunk_refused(identity, SendChunkError::Closed);
}

/// Every helper emits the message and category the guide documents.
///
/// A single missing event is named in the failure, so a renamed message says
/// which one drifted rather than only that the catalogue is incomplete.
#[traced_test]
#[test]
fn every_diagnostic_carries_its_documented_message_and_category() {
    emit_every_event();

    logs_assert(|lines: &[&str]| {
        let missing = EVENTS.iter().find(|(message, category)| {
            !lines
                .iter()
                .any(|line| line.contains(message) && line.contains(category))
        });
        match missing {
            None => Ok(()),
            Some((message, category)) => {
                Err(format!("no event carries {message:?} with {category:?}"))
            }
        }
    });
}

/// A read that timed out reports the wait rather than a body outcome.
///
/// No body reached a verdict here, so the event must carry no
/// `error_category`: one would let an operator count a healthy read timeout as
/// a body failure.
#[traced_test]
#[test]
fn a_read_timeout_is_reported_without_a_body_category() {
    read_timed_out();

    logs_assert(|lines: &[&str]| {
        let timeout = lines
            .iter()
            .find(|line| line.contains("DEBUG") && line.contains("read timeout elapsed"))
            .ok_or_else(|| "the read timeout is not reported at debug level".to_owned())?;
        if timeout.contains("error_category") {
            return Err(format!(
                "a read timeout has no body outcome to categorize: {timeout}"
            ));
        }
        Ok(())
    });
}

/// One event a diagnostics test expects to find in the captured log.
///
/// The four fields travel together because they identify the same event, and a
/// test that checked one without the others would let the rest drift.
struct Expected<'a> {
    /// Level the event must be logged at.
    level: &'a str,
    /// Text the event message must carry.
    message: &'a str,
    /// Value of the event's `operation` field.
    operation: &'a str,
    /// Value of the event's `error_category` field.
    category: &'a str,
}

/// Return whether `lines` hold one event matching `expected`.
///
/// The operation and category are quoted string fields, so they are matched
/// with their quotes: filtering on the bare word would also match a message
/// that merely mentions it.
fn reports(lines: &[&str], expected: &Expected<'_>) -> bool {
    lines.iter().any(|line| {
        line.contains(expected.level)
            && line.contains(expected.message)
            && line.contains(&format!("operation=\"{}\"", expected.operation))
            && line.contains(&format!("error_category=\"{}\"", expected.category))
    })
}

/// The deadline warning keeps both its level and its operation.
///
/// The level is what an operator's alerting filters on, and the operation
/// separates a body failure from the reply wait that follows it, so neither
/// may drift with the message text.
#[traced_test]
#[test]
fn the_handler_deadline_warning_keeps_its_level_and_operation() {
    handler_deadline_exceeded(7);

    logs_assert(|lines: &[&str]| {
        reports(
            lines,
            &Expected {
                level: "WARN",
                message: "streaming handler exceeded the connection wait",
                operation: "terminal_body",
                category: "reply_abandoned",
            },
        )
        .then_some(())
        .ok_or_else(|| "the abandoned reply is not a terminal-body warning".to_owned())
    });
}

/// A refused final chunk keeps its debug level and chunk-delivery operation.
#[traced_test]
#[test]
fn the_final_chunk_refusal_keeps_its_level_and_operation() {
    final_chunk_refused(identity(), SendChunkError::Closed);

    logs_assert(|lines: &[&str]| {
        reports(
            lines,
            &Expected {
                level: "DEBUG",
                message: "final request-body chunk refused",
                operation: "chunk_delivery",
                category: "abandoned_body",
            },
        )
        .then_some(())
        .ok_or_else(|| "the refused final chunk is not a chunk-delivery event".to_owned())
    });
}
