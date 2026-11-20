//! Accounting for the outcomes recorded while a body is delivered.
//!
//! This module covers the terminal helpers' branches: a failure the handler
//! observed, a failure the bounded wait could not hand over, a failure the
//! handler had already refused, and a completed final chunk delivered as the
//! body's own end. Each branch records exactly one outcome, and a branch that
//! did not complete its delivery must not also record the body as abandoned,
//! so the counts are asserted together rather than one at a time.

use std::future;

use bytes::Bytes;
use rstest::rstest;

use super::{
    BodyClose,
    HandlerWait,
    body_close,
    deliver_final_chunk_then_await,
    deliver_then_await,
};
use crate::{
    app::inbound_handler::{
        stream_active::{HandlerFuture, PendingSend},
        stream_outcome_support::{
            identity,
            paused_block_on,
            queue_one_chunk,
            reading_handler,
            recorded,
            truncated_failure,
            wait,
        },
    },
    metrics::StreamingBodyOutcome,
    request::{QueuedBytes, accounted_body_channel},
};

/// A failure the handler observed is counted as terminal, not abandoned.
#[rstest]
#[test]
fn a_delivered_failure_is_counted_as_terminal() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    let mut handler = reading_handler(body);

    let (settled, counts) = recorded(|| {
        paused_block_on(deliver_then_await(
            wait(),
            truncated_failure(),
            sender,
            &mut handler,
        ))
    });

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Replied(Some(Bytes::from_static(b"failed"))),
        "the handler must observe the failure rather than a clean end of stream"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Terminal),
        1,
        "the delivered failure is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        0,
        "a body whose handler observed the failure is not abandoned"
    );
}

/// A failure that never reached the handler is counted as a delivery timeout.
///
/// The channel is full and the handler never reads, so the bounded wait is the
/// only way the exchange can end. The body stays open until the receiver is
/// dropped afterwards, which proves the wait was spent on channel capacity
/// rather than on a closed channel.
#[rstest]
#[test]
fn a_failure_that_cannot_be_delivered_is_counted_as_a_delivery_timeout() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    let mut handler: HandlerFuture = Box::pin(future::pending());

    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            queue_one_chunk(&sender).await;
            deliver_then_await(wait(), truncated_failure(), sender, &mut handler).await
        })
    });
    drop(body);

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Deadline,
        "an unresponsive handler must be bounded by the wait"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::DeliveryTimeout),
        1,
        "the expired delivery is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Terminal),
        0,
        "a failure nobody observed is not counted as delivered"
    );
}

/// A handler that dropped its body abandons the undelivered failure.
#[rstest]
fn a_failure_refused_by_a_dropped_body_is_counted_as_abandoned() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    drop(body);
    let mut handler: HandlerFuture = Box::pin(future::ready(Some(Bytes::from_static(b"reply"))));

    let (settled, counts) = recorded(|| {
        paused_block_on(deliver_then_await(
            wait(),
            truncated_failure(),
            sender,
            &mut handler,
        ))
    });

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Replied(Some(Bytes::from_static(b"reply"))),
        "the handler had already chosen not to read its body"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "the refused failure is counted once, not once per branch"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Terminal),
        0,
        "a failure the channel refused was not delivered"
    );
}

/// A reply that settles first leaves its body abandoned, counted once.
///
/// The failure needs a channel slot that only this handler could free, so the
/// delivery cannot complete before the reply does. Nothing else reports the
/// body in that case: the delivery branch never completed, and the handler's
/// reply says nothing about the body it abandoned.
#[rstest]
#[test]
fn a_reply_that_settles_first_is_counted_as_abandoned() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    let mut handler: HandlerFuture = Box::pin(future::ready(Some(Bytes::from_static(b"reply"))));

    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            queue_one_chunk(&sender).await;
            deliver_then_await(wait(), truncated_failure(), sender, &mut handler).await
        })
    });
    drop(body);

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Replied(Some(Bytes::from_static(b"reply"))),
        "the reply settles before the full channel can accept the failure"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "the abandoned body is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::DeliveryTimeout),
        0,
        "the delivery never expired; the reply simply won"
    );
}

/// A body completed before the close is delivered whole, not as a truncation.
///
/// A whole message followed by a half-close is a common client shape. The
/// handler's reply is the body it read, so a terminal error being invented on
/// this path would show up as `failed` instead of the body's own bytes.
#[rstest]
#[test]
fn a_completed_final_chunk_reaches_the_handler_whole() {
    let (sender, body) = accounted_body_channel(4, QueuedBytes::new());
    let mut handler = reading_handler(body);
    let pending = PendingSend {
        sender,
        chunk: Bytes::from_static(b"last"),
        is_last: true,
    };

    let (settled, counts) = recorded(|| {
        paused_block_on(deliver_final_chunk_then_await(
            wait(),
            identity(),
            pending,
            &mut handler,
        ))
    });

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Replied(Some(Bytes::from_static(b"last"))),
        "the handler must read the completed body, not a truncation"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Delivered),
        1,
        "the completed body is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        0,
        "a body that completed before the close is not abandoned"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Terminal),
        0,
        "the close has nothing left to truncate"
    );
}

/// A final chunk the handler refused leaves the body abandoned.
#[rstest]
#[test]
fn a_final_chunk_refused_by_a_dropped_body_is_counted_as_abandoned() {
    let (sender, body) = accounted_body_channel(2, QueuedBytes::new());
    drop(body);
    let mut handler: HandlerFuture = Box::pin(future::pending());
    let pending = PendingSend {
        sender,
        chunk: Bytes::from_static(b"last"),
        is_last: true,
    };

    let (settled, counts) = recorded(|| {
        paused_block_on(deliver_final_chunk_then_await(
            wait(),
            identity(),
            pending,
            &mut handler,
        ))
    });

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Deadline,
        "the abandoned handler never answers"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "the refused final chunk is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Delivered),
        0,
        "a chunk the channel refused was not delivered"
    );
}

/// A final chunk whose channel never drains is counted as abandoned.
///
/// The handler never reads, so the chunk cannot take a channel slot and the
/// bounded wait expires. The body it was owed is lost, which is an
/// abandonment rather than a delivery.
#[rstest]
#[test]
fn a_final_chunk_that_cannot_be_delivered_is_counted_as_abandoned() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    let mut handler: HandlerFuture = Box::pin(future::pending());

    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            queue_one_chunk(&sender).await;
            let pending = PendingSend {
                sender,
                chunk: Bytes::from_static(b"last"),
                is_last: true,
            };
            deliver_final_chunk_then_await(wait(), identity(), pending, &mut handler).await
        })
    });
    drop(body);

    assert_eq!(
        settled.expect("the exchange runs on a paused runtime"),
        HandlerWait::Deadline,
        "an unresponsive handler must be bounded by the wait"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "the undelivered final chunk is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Delivered),
        0,
        "a chunk that never took a channel slot was not delivered"
    );
}

/// A pending final chunk closes the body even though the exchange failed.
#[rstest]
#[test]
fn a_pending_final_chunk_closes_the_body_instead_of_failing_it() {
    let (sender, _body) = accounted_body_channel(1, QueuedBytes::new());
    let pending = PendingSend {
        sender: sender.clone(),
        chunk: Bytes::from_static(b"last"),
        is_last: true,
    };

    let close = body_close(
        Some(pending),
        Some(std::io::Error::other("peer went away")),
        Some(sender),
        identity(),
    );

    assert!(
        matches!(close, BodyClose::FinalChunk(_)),
        "a chunk that ends the body leaves nothing to truncate"
    );
}

/// A pending continuation chunk leaves the body truncated.
#[rstest]
#[test]
fn a_pending_continuation_chunk_leaves_the_body_truncated() {
    let (sender, _body) = accounted_body_channel(1, QueuedBytes::new());
    let pending = PendingSend {
        sender: sender.clone(),
        chunk: Bytes::from_static(b"more"),
        is_last: false,
    };

    let close = body_close(
        Some(pending),
        Some(std::io::Error::other("peer went away")),
        Some(sender),
        identity(),
    );

    assert!(
        matches!(close, BodyClose::Fail { .. }),
        "the handler is owed the reason its body stopped"
    );
}

/// A body with nothing left to deliver only awaits its handler's reply.
#[rstest]
#[test]
fn a_body_with_nothing_left_to_deliver_only_awaits_its_reply() {
    let close = body_close(
        None,
        Some(std::io::Error::other("peer went away")),
        None,
        identity(),
    );

    assert!(
        matches!(close, BodyClose::AwaitReply),
        "no open body and no pending chunk means no delivery remains"
    );
}
