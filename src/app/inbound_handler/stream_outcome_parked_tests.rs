//! Accounting for the outcomes recorded while a completed body is dispatched.
//!
//! A message that finished while the slot was busy reaches its streaming
//! handler as a pre-filled body, and that delivery has its own outcomes: the
//! body arrived, the handler had gone, the body could not be retained, or the
//! route has no handler at all. Each decision is made in this module, and none
//! is observable from the connection's behaviour, so each count is read back
//! here.

use std::collections::HashMap;

use bytes::Bytes;
use futures::StreamExt;
use tokio::time::Instant;

use super::{
    PrefilledOffer,
    deliver_prefilled,
    lookup_streaming_handler,
    refuse_prefilled,
    refuse_unrouted,
};
use crate::{
    app::inbound_handler::{
        stream_handles::ChunkBudget,
        stream_outcome_support::{counted, paused_block_on, reading_route, recorded, wait},
    },
    metrics::StreamingBodyOutcome,
    request::{QueuedBytes, accounted_body_channel},
};

/// Route identifier the pre-filled scenarios deliver to.
const PARKED_MESSAGE_ID: u32 = 10;

/// The pre-filled body every scenario delivers.
const fn offer() -> PrefilledOffer {
    PrefilledOffer {
        body: Bytes::from_static(b"abcd"),
        message_id: PARKED_MESSAGE_ID,
    }
}

/// The deadline one delivery runs under.
///
/// Paused time makes the length irrelevant to the assertion; the delivery and
/// the handler simply share one instant.
fn deadline() -> Instant { Instant::now() + wait() }

/// A budget that admits the whole body into an empty account.
const fn open_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: usize::MAX,
    }
}

/// A budget too small for the body beside the bytes an earlier body holds.
const fn tight_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 6,
    }
}

/// A pre-filled body that arrived is counted as delivered.
#[test]
fn an_affordable_prefilled_body_is_counted_as_delivered() {
    let (sender, mut body) = accounted_body_channel(1, QueuedBytes::new());

    let (observed, counts) = recorded(|| {
        paused_block_on(async {
            deliver_prefilled(sender, offer(), open_budget(), deadline()).await;
            body.next().await
        })
    });

    let chunk = observed
        .expect("the delivery runs on a paused runtime")
        .expect("the whole body should arrive as one chunk")
        .expect("the chunk is not an error");
    assert_eq!(chunk.as_ref(), b"abcd", "the whole body is delivered");
    assert_eq!(
        counts.count(StreamingBodyOutcome::Delivered),
        1,
        "the delivered body is counted once"
    );
}

/// A handler that dropped its stream abandons the pre-filled body.
#[test]
fn a_prefilled_body_refused_by_a_dropped_handler_is_abandoned() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    drop(body);

    let (_, abandoned) = counted(StreamingBodyOutcome::Abandoned, || {
        paused_block_on(deliver_prefilled(
            sender,
            offer(),
            open_budget(),
            deadline(),
        ))
    });

    assert_eq!(abandoned, 1, "the body the handler refused is counted once");
}

/// A body the connection cannot retain is counted as an admission timeout.
///
/// The queue is non-empty and the aggregate budget cannot take the body, so
/// the bounded wait expires. Counting it as an abandonment would blame the
/// handler, which never had a chance to read anything.
#[test]
fn an_unaffordable_prefilled_body_is_counted_as_an_admission_timeout() {
    let (observed, counts) = recorded(|| {
        paused_block_on(async {
            let queued = QueuedBytes::new();
            let (holder, _held) = accounted_body_channel(1, queued.clone());
            holder
                .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
                .await
                .expect("an empty queue admits one chunk");
            let (sender, mut body) = accounted_body_channel(1, queued);
            deliver_prefilled(sender, offer(), tight_budget(), deadline()).await;
            body.next().await
        })
    });

    let item = observed
        .expect("the delivery runs on a paused runtime")
        .expect("the refused body still yields a terminal item");
    assert!(item.is_err(), "the handler reads an error, not a body");
    assert_eq!(
        counts.count(StreamingBodyOutcome::AdmissionTimeout),
        1,
        "the expired admission is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        0,
        "the handler did not abandon a body it never received"
    );
}

/// A refused body whose failure cannot be delivered is also abandoned.
///
/// The two outcomes describe different things and both are true here: the
/// connection could not retain the body, and the handler had already gone, so
/// the terminal error had no receiver. Reporting only the first would hide the
/// loss; reporting only the second would blame the handler for the budget.
#[test]
fn a_refused_body_whose_failure_is_not_delivered_is_also_abandoned() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    drop(body);

    let (_, counts) = recorded(|| paused_block_on(refuse_prefilled(&sender, PARKED_MESSAGE_ID)));

    assert_eq!(
        counts.count(StreamingBodyOutcome::AdmissionTimeout),
        1,
        "the body that could not be retained is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "the undeliverable terminal error is counted once"
    );
}

/// A parked message for a route with no handler is counted as no handler.
///
/// The count belongs to the refusal command, not to the lookup: the message is
/// dropped at the decision, and an operator sees the same `no_handler`
/// vocabulary the live start path records.
#[test]
fn a_missing_parked_handler_is_counted_as_no_handler() {
    let routes: HashMap<u32, _> = HashMap::new();

    let (missing, counts) = recorded(|| {
        refuse_unrouted(PARKED_MESSAGE_ID);
        lookup_streaming_handler(&routes, PARKED_MESSAGE_ID)
    });

    assert!(missing.is_none(), "no handler was registered");
    assert_eq!(
        counts.count(StreamingBodyOutcome::NoHandler),
        1,
        "the refusal is counted once"
    );
}

/// The lookup alone records nothing, whatever it finds.
///
/// A read-named helper that counted a miss would make every caller record the
/// outcome a second time and would report a refusal where no message was
/// dropped.
#[test]
fn the_lookup_alone_does_not_count_a_miss() {
    let mut routes = HashMap::new();

    let (missing, counts) = recorded(|| lookup_streaming_handler(&routes, PARKED_MESSAGE_ID));

    assert!(missing.is_none(), "no handler was registered");
    assert_eq!(
        counts.count(StreamingBodyOutcome::NoHandler),
        0,
        "a lookup must not record the outcome its caller owns"
    );

    routes.insert(PARKED_MESSAGE_ID, reading_route());
    let (found, resolved) = recorded(|| lookup_streaming_handler(&routes, PARKED_MESSAGE_ID));

    assert!(found.is_some(), "the registered handler is resolved");
    assert_eq!(
        resolved.count(StreamingBodyOutcome::NoHandler),
        0,
        "a resolved handler is not a missing one"
    );
}
