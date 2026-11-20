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
    FinalDelivery,
    HandlerWait,
    body_close,
    deliver_final_chunk_then_await,
    deliver_then_await,
};
use crate::{
    app::inbound_handler::{
        stream_active::{HandlerFuture, PendingSend},
        stream_handles::ChunkBudget,
        stream_outcome_support::{
            Expectation,
            assert_exchange,
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

/// A budget that admits any final chunk, so a case exercises channel capacity.
fn unbounded_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: usize::MAX,
    }
}

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

    assert_exchange(
        settled,
        &counts,
        &Expectation {
            reply: HandlerWait::Replied(Some(Bytes::from_static(b"failed"))),
            recorded: StreamingBodyOutcome::Terminal,
            why: "the handler must observe the failure rather than a clean end of stream",
        },
    );
}

/// How a case leaves the body's channel before the failure is delivered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyEnd {
    /// The handler holds the body, with its one slot already taken.
    Queued,
    /// The handler dropped the body, so the failure has no receiver.
    Dropped,
}

/// A failure competing with its handler is counted by which one won.
///
/// The channel is full, so the failure needs a slot only the handler can free,
/// and the bounded wait is the only other way out. A handler that never reads
/// lets the wait expire, and a handler that answers first leaves the body
/// abandoned, counted once. A handler that dropped its body outright refuses
/// the failure before either race starts, which is still one abandonment: the
/// body stays open until the receiver is dropped afterwards, which proves the
/// wait was spent on channel capacity rather than on a closed channel.
#[rstest]
#[case::wait_expires(
    Box::pin(future::pending()),
    BodyEnd::Queued,
    Expectation {
        reply: HandlerWait::Deadline,
        recorded: StreamingBodyOutcome::DeliveryTimeout,
        why: "an unresponsive handler must be bounded by the wait",
    },
)]
#[case::reply_wins(
    Box::pin(future::ready(Some(Bytes::from_static(b"reply")))),
    BodyEnd::Queued,
    Expectation {
        reply: HandlerWait::Replied(Some(Bytes::from_static(b"reply"))),
        recorded: StreamingBodyOutcome::Abandoned,
        why: "the reply settles before the full channel can accept the failure",
    },
)]
#[case::receiver_gone(
    Box::pin(future::ready(Some(Bytes::from_static(b"reply")))),
    BodyEnd::Dropped,
    Expectation {
        reply: HandlerWait::Replied(Some(Bytes::from_static(b"reply"))),
        recorded: StreamingBodyOutcome::Abandoned,
        why: "a failure the channel refused was not delivered",
    },
)]
#[test]
fn a_failure_competing_with_its_handler_is_counted_by_which_won(
    #[case] mut handler: HandlerFuture,
    #[case] end: BodyEnd,
    #[case] expected: Expectation,
) {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    let held_body = (end == BodyEnd::Queued).then_some(body);

    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            if end == BodyEnd::Queued {
                queue_one_chunk(&sender).await;
            }
            deliver_then_await(wait(), truncated_failure(), sender, &mut handler).await
        })
    });
    drop(held_body);

    assert_exchange(settled, &counts, &expected);
}

/// A body completed before the close is delivered whole, or abandoned.
///
/// A whole message followed by a half-close is a common client shape, so a
/// handler still reading its body receives the final chunk and answers with
/// it; inventing a terminal error here would show up as `failed` instead of
/// the body's own bytes. A handler that had already dropped its body cannot be
/// given the chunk at all, and the bounded wait then ends the exchange with
/// the body abandoned.
#[rstest]
#[case::delivered(
    true,
    Expectation {
        reply: HandlerWait::Replied(Some(Bytes::from_static(b"last"))),
        recorded: StreamingBodyOutcome::Delivered,
        why: "the handler must read the completed body, not a truncation",
    },
)]
#[case::refused(
    false,
    Expectation {
        reply: HandlerWait::Deadline,
        recorded: StreamingBodyOutcome::Abandoned,
        why: "a dropped body cannot take the final chunk",
    },
)]
#[test]
fn a_completed_final_chunk_is_delivered_whole_or_abandoned(
    #[case] reads_body: bool,
    #[case] expected: Expectation,
) {
    let (sender, body) = accounted_body_channel(2, QueuedBytes::new());
    let mut handler: HandlerFuture = if reads_body {
        reading_handler(body)
    } else {
        drop(body);
        Box::pin(future::pending())
    };
    let pending = PendingSend {
        sender,
        chunk: Bytes::from_static(b"last"),
        is_last: true,
    };

    let delivery = FinalDelivery {
        identity: identity(),
        pending,
        budget: unbounded_budget(),
    };
    let (settled, counts) = recorded(|| {
        paused_block_on(deliver_final_chunk_then_await(
            wait(),
            delivery,
            &mut handler,
        ))
    });

    assert_exchange(settled, &counts, &expected);
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
            let delivery = FinalDelivery {
                identity: identity(),
                pending: PendingSend {
                    sender,
                    chunk: Bytes::from_static(b"last"),
                    is_last: true,
                },
                budget: unbounded_budget(),
            };
            deliver_final_chunk_then_await(wait(), delivery, &mut handler).await
        })
    });
    drop(body);

    assert_exchange(
        settled,
        &counts,
        &Expectation {
            reply: HandlerWait::Deadline,
            recorded: StreamingBodyOutcome::Abandoned,
            why: "an unresponsive handler must be bounded by the wait",
        },
    );
}

/// A final chunk that does not fit the budget is abandoned, not forced in.
///
/// The account already holds a queued chunk, and the final chunk would carry
/// it past the aggregate limit. Handing it over anyway would leave the
/// connection over budget for as long as the handler held it, so the chunk
/// waits for room instead; the bounded wait is the only other exit, and the
/// body is abandoned when it expires.
#[rstest]
#[test]
fn a_final_chunk_that_does_not_fit_the_budget_is_abandoned() {
    let (sender, body) = accounted_body_channel(2, QueuedBytes::new());
    let mut handler: HandlerFuture = Box::pin(future::pending());
    let room_for_one_chunk = ChunkBudget {
        other_retained: 0,
        limit: 4,
    };

    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            queue_one_chunk(&sender).await;
            let delivery = FinalDelivery {
                identity: identity(),
                pending: PendingSend {
                    sender,
                    chunk: Bytes::from_static(b"last"),
                    is_last: true,
                },
                budget: room_for_one_chunk,
            };
            deliver_final_chunk_then_await(wait(), delivery, &mut handler).await
        })
    });
    drop(body);

    assert_exchange(
        settled,
        &counts,
        &Expectation {
            reply: HandlerWait::Deadline,
            recorded: StreamingBodyOutcome::Abandoned,
            why: "a final chunk over the budget was forced into the account",
        },
    );
}

/// A pending chunk decides what a closing exchange still owes its handler.
///
/// A chunk that ends the body leaves nothing to truncate, so it is delivered
/// whole; any other pending chunk leaves the body short, and the handler is
/// owed the failure that stopped it.
#[rstest]
#[case::final_chunk(Bytes::from_static(b"last"), true, true)]
#[case::continuation(Bytes::from_static(b"more"), false, false)]
#[test]
fn a_pending_chunk_decides_what_the_close_still_owes(
    #[case] chunk: Bytes,
    #[case] is_last: bool,
    #[case] ends_body: bool,
) {
    let (sender, _body) = accounted_body_channel(1, QueuedBytes::new());
    let pending = PendingSend {
        sender: sender.clone(),
        chunk,
        is_last,
    };

    let close = body_close(
        Some(pending),
        Some(std::io::Error::other("peer went away")),
        Some(sender),
        identity(),
    );

    if ends_body {
        assert!(
            matches!(close, BodyClose::FinalChunk(_)),
            "a chunk that ends the body leaves nothing to truncate"
        );
    } else {
        assert!(
            matches!(close, BodyClose::Fail { .. }),
            "the handler is owed the reason its body stopped"
        );
    }
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
