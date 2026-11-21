//! Unit tests for the connection-owned streaming slot.
//!
//! The slot's admission behaviour against bytes retained elsewhere is covered
//! in [`admission_tests`](super::admission_tests); this module proves how an
//! in-flight admission races the handler, and how a body failure is delivered
//! to a handler before its reply is awaited.

use std::{
    future::{self, poll_fn},
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use rstest::{fixture, rstest};

use super::{
    super::{
        stream_active::{ActiveStream, BodyIdentity, HandlerFuture, PendingEvent, PendingSend},
        stream_terminal::{HandlerWait, TerminalFailure, await_handler, deliver_then_await},
    },
    ConnectionStreams,
};
use crate::{
    app::inbound_handler::stream_handles::ChunkBudget,
    request::{AccountedBodySender, QueuedBytes, RequestBodyStream, accounted_body_channel},
};

/// Identity of the message a test's body belongs to.
fn identity() -> BodyIdentity {
    BodyIdentity {
        message_id: 1,
        correlation_id: None,
    }
}

/// A truncated body failure for the test's message.
fn failure() -> TerminalFailure {
    TerminalFailure {
        identity: identity(),
        error: io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
    }
}

/// A wait short enough that a stalled exchange hands control back quickly.
#[fixture]
fn short_wait() -> Duration {
    // Kept as a binding so the fixture body is a statement list; a bare
    // expression body trips `unused_braces` through the macro expansion.
    let millis = 50;
    Duration::from_millis(millis)
}

/// A slot whose handler and pending admission never make progress.
///
/// The body stream is returned alongside the slot: dropping it would close the
/// channel, and a closed consumer turns the pending admission into an error
/// instead of leaving it waiting.
///
/// One chunk is left unread in the channel. It charges bytes to the slot's
/// account, so the four-byte pending chunk is refused outright rather than
/// admitted by the empty-account exception.
async fn stalled_stream() -> (ActiveStream, RequestBodyStream) {
    let (sender, body) = accounted_body_channel(2, QueuedBytes::new());
    sender
        .clone()
        .send_chunk(Bytes::from_static(b"unread"), 0, usize::MAX)
        .await
        .expect("an empty channel admits one chunk");
    let stream = ActiveStream {
        key: None,
        assembly: None,
        route_id: 1,
        correlation_id: None,
        sender: None,
        handler: Box::pin(future::pending()),
        pending: Some(PendingSend {
            sender,
            chunk: Bytes::from_static(b"abcd"),
            is_last: false,
        }),
    };
    (stream, body)
}

/// A budget that cannot admit the stalled slot's four-byte chunk.
#[fixture]
fn blocked_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 2,
    }
}

/// A body channel that holds one value, so the next send must wait.
#[fixture]
fn body_channel() -> (AccountedBodySender, RequestBodyStream) {
    accounted_body_channel(1, QueuedBytes::new())
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_stalled_admission_returns_control_after_the_read_timeout(
    short_wait: Duration,
    blocked_budget: ChunkBudget,
) {
    let (mut stream, _body) = stalled_stream().await;
    let event = stream.drive_pending(short_wait, blocked_budget).await;

    assert!(
        matches!(event, PendingEvent::Idle),
        "a stalled admission should hand control back to the loop"
    );
}

/// A stall is measured across drives, and a settled send ends it.
///
/// Each drive is bounded so the loop can re-check shutdown, expiry and the
/// memory budget, so one expired wait must not be mistaken for the whole
/// stall: the slot accumulates the interval instead. A send that settles,
/// however it settles, ends that interval, so a handler that catches up is
/// never charged for the time it was busy.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_stall_accumulates_across_drives_until_an_admission_settles(
    short_wait: Duration,
    blocked_budget: ChunkBudget,
) {
    let (active, body) = stalled_stream().await;
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    streams.active = Some(active);

    assert!(
        !streams.stall_expired(short_wait),
        "a body that has not waited yet cannot have stalled"
    );
    let stalled = streams.drive_pending(short_wait, blocked_budget).await;
    assert!(
        matches!(stalled, PendingEvent::Idle),
        "the blocked chunk should hand control back to the loop"
    );
    assert!(
        streams.stall_expired(short_wait),
        "the expired wait should count toward the body's stall"
    );

    drop(body);
    let settled = streams.drive_pending(short_wait, blocked_budget).await;
    assert!(
        matches!(settled, PendingEvent::Send(Err(_))),
        "a dropped body refuses the queued chunk"
    );
    assert!(
        !streams.stall_expired(short_wait),
        "a settled admission should clear the stall interval"
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_resolved_handler_wins_over_a_stalled_admission(blocked_budget: ChunkBudget) {
    let (mut stream, _body) = stalled_stream().await;
    stream.handler = Box::pin(async { Some(Bytes::from_static(b"reply")) });

    match stream
        .drive_pending(Duration::from_secs(1), blocked_budget)
        .await
    {
        PendingEvent::Handler(outcome) => {
            assert_eq!(outcome.as_deref(), Some(b"reply".as_slice()));
        }
        _ => panic!("the handler outcome should win over a stalled admission"),
    }
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_completed_admission_wins_over_a_stalled_handler() {
    let (mut stream, _body) = stalled_stream().await;
    let pending = stream
        .pending
        .as_mut()
        .expect("the fixture queues a pending admission");
    // An empty chunk needs no bytes, so its admission settles immediately.
    pending.chunk = Bytes::new();
    pending.is_last = true;

    match stream
        .drive_pending(
            Duration::from_secs(1),
            ChunkBudget {
                other_retained: 0,
                limit: usize::MAX,
            },
        )
        .await
    {
        PendingEvent::Send(result) => assert!(result.is_ok()),
        _ => panic!("the admission result should win over a stalled handler"),
    }
}

/// A terminal error must reach a handler whose body channel is already full.
///
/// Redelivering it before polling the handler would deadlock: the error needs a
/// channel slot, and only the handler can free one.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_terminal_error_reaches_a_handler_whose_channel_is_full(
    body_channel: (AccountedBodySender, RequestBodyStream),
) {
    let (sender, body) = body_channel;
    queue_one_chunk(&sender).await;
    let mut handler: HandlerFuture = Box::pin(read_until_terminal(body));

    let outcome = deliver_then_await(Duration::from_secs(1), failure(), sender, &mut handler).await;

    assert_eq!(
        outcome,
        HandlerWait::Replied(Some(Bytes::from_static(b"error"))),
        "the handler should observe the failure, not a clean end of stream"
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn terminal_delivery_is_bounded_when_the_handler_never_reads(
    body_channel: (AccountedBodySender, RequestBodyStream),
    short_wait: Duration,
) {
    let (sender, body) = body_channel;
    queue_one_chunk(&sender).await;
    let mut handler: HandlerFuture = Box::pin(future::pending());

    let outcome = deliver_then_await(short_wait, failure(), sender, &mut handler).await;

    // Keep the receiver alive until the wait expires, so the delivery really is
    // stalled on channel capacity rather than on a closed channel.
    drop(body);
    assert_eq!(
        outcome,
        HandlerWait::Deadline,
        "an unresponsive handler should still be bounded by the wait"
    );
}

/// A delivery that times out abandons the handler instead of polling it again.
///
/// Once the channel refuses the failure for a whole wait, the handler can only
/// read a clean end of stream, so a reply it produces afterwards would present a
/// truncated body as complete. The exchange therefore ends at the delivery
/// timeout, without another poll of the handler.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_delivery_timeout_stops_polling_the_handler(
    body_channel: (AccountedBodySender, RequestBodyStream),
    short_wait: Duration,
) {
    let (sender, body) = body_channel;
    queue_one_chunk(&sender).await;
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let mut handler: HandlerFuture = Box::pin(poll_fn(move |_context| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Poll::Pending
        } else {
            panic!("a timed-out delivery must abandon the handler");
        }
    }));

    let outcome = deliver_then_await(short_wait, failure(), sender, &mut handler).await;

    // The receiver outlives the exchange, so the delivery was blocked on
    // channel capacity rather than on a closed channel.
    drop(body);
    assert_eq!(
        outcome,
        HandlerWait::Deadline,
        "a delivery the channel refused must end the exchange"
    );
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "the abandoned handler must not be polled again"
    );
}

/// A handler that finishes normally reports its own outcome, not a deadline.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_handler_reply_is_reported_as_replied() {
    let mut handler: HandlerFuture = Box::pin(async { Some(Bytes::from_static(b"reply")) });

    let outcome = await_handler(Duration::from_secs(1), &mut handler).await;

    assert_eq!(
        outcome,
        HandlerWait::Replied(Some(Bytes::from_static(b"reply"))),
        "a completed handler must not be reported as a deadline"
    );
}

/// A deliberate silence is distinct from the handler outliving the wait.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_handler_without_a_reply_is_reported_as_replied() {
    let mut handler: HandlerFuture = Box::pin(async { None });

    let outcome = await_handler(Duration::from_secs(1), &mut handler).await;

    assert_eq!(
        outcome,
        HandlerWait::Replied(None),
        "choosing not to reply must not look like a deadline"
    );
}

/// A handler that never finishes must be reported as a deadline.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_handler_outliving_the_wait_is_reported_as_a_deadline(short_wait: Duration) {
    let mut handler: HandlerFuture = Box::pin(future::pending());

    let outcome = await_handler(short_wait, &mut handler).await;

    assert_eq!(
        outcome,
        HandlerWait::Deadline,
        "a hung handler must be observable as a deadline"
    );
}

/// Fill the body channel so the next send must wait for the consumer.
async fn queue_one_chunk(sender: &AccountedBodySender) {
    sender
        .clone()
        .send_chunk(Bytes::from_static(b"chunk"), 0, usize::MAX)
        .await
        .expect("an empty queue admits one chunk");
}

/// Read the body to its end, reporting whether the terminal error arrived.
async fn read_until_terminal(mut body: RequestBodyStream) -> Option<Bytes> {
    while let Some(item) = body.next().await {
        if item.is_err() {
            return Some(Bytes::from_static(b"error"));
        }
    }
    Some(Bytes::from_static(b"clean"))
}
