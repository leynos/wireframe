//! Unit tests for the connection-owned streaming slot.

use std::{future, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use rstest::{fixture, rstest};

use super::{
    super::stream_terminal::{HandlerWait, await_handler, deliver_then_await},
    ActiveStream,
    ConnectionStreams,
    HandlerFuture,
    PendingEvent,
    PendingSend,
};
use crate::{
    app::{
        StreamingHandler,
        inbound_handler::stream_handles::{ChunkBudget, FirstChunk},
    },
    request::{
        AccountedBodySender,
        QueuedBytes,
        RequestBodyStream,
        RequestParts,
        accounted_body_channel,
    },
};

/// A wait short enough that a stalled exchange hands control back quickly.
#[fixture]
fn short_wait() -> Duration {
    // Kept as a binding so the fixture body is a statement list; a bare
    // expression body trips `unused_braces` through the macro expansion.
    let millis = 50;
    Duration::from_millis(millis)
}

/// Handler that holds its body open without ever consuming a chunk.
#[fixture]
fn stalled_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(async move {
            let _body = body;
            future::pending::<Option<Bytes>>().await
        })
    })
}

/// A slot whose handler and pending admission never make progress.
#[fixture]
fn stalled_stream() -> ActiveStream {
    ActiveStream {
        key: None,
        assembly: None,
        route_id: 1,
        correlation_id: None,
        sender: None,
        handler: Box::pin(future::pending()),
        pending: Some(PendingSend {
            future: Box::pin(future::pending()),
            is_last: false,
        }),
    }
}

/// A first chunk whose four bytes do not fit beside the held ones.
#[fixture]
fn unaffordable_first_chunk() -> FirstChunk {
    FirstChunk {
        key: None,
        assembly: None,
        parts: RequestParts::new(1, None, Vec::new()),
        chunk: Bytes::from_static(b"abcd"),
        is_last: false,
    }
}

/// Allow less room than the held bytes and the first chunk need together.
#[fixture]
fn tight_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 6,
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
    mut stalled_stream: ActiveStream,
    short_wait: Duration,
) {
    let event = stalled_stream.drive_pending(short_wait).await;

    assert!(
        matches!(event, PendingEvent::Idle),
        "a stalled admission should hand control back to the loop"
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_resolved_handler_wins_over_a_stalled_admission(mut stalled_stream: ActiveStream) {
    stalled_stream.handler = Box::pin(async { Some(Bytes::from_static(b"reply")) });

    match stalled_stream.drive_pending(Duration::from_secs(1)).await {
        PendingEvent::Handler(outcome) => {
            assert_eq!(outcome.as_deref(), Some(b"reply".as_slice()));
        }
        _ => panic!("the handler outcome should win over a stalled admission"),
    }
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_completed_admission_wins_over_a_stalled_handler(mut stalled_stream: ActiveStream) {
    stalled_stream.pending = Some(PendingSend {
        future: Box::pin(async { Ok(()) }),
        is_last: true,
    });

    match stalled_stream.drive_pending(Duration::from_secs(1)).await {
        PendingEvent::Send(result) => assert!(result.is_ok()),
        _ => panic!("the admission result should win over a stalled handler"),
    }
}

/// A first chunk that cannot be admitted must not block the loop.
///
/// Admission is decided against the connection-wide account, so bytes an
/// earlier body still holds can keep it waiting. Waiting inside `start` would
/// deadlock: the slot is only registered after the wait, and nothing else can
/// release those bytes until the loop regains control.
#[rstest]
#[tokio::test(start_paused = true)]
async fn an_unaffordable_first_chunk_becomes_a_pending_admission(
    unaffordable_first_chunk: FirstChunk,
    stalled_handler: StreamingHandler,
    tight_budget: ChunkBudget,
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    let _held_body = retain_bytes_outside_the_slot(&streams).await;

    streams.start(unaffordable_first_chunk, &stalled_handler, tight_budget);

    assert!(
        streams.is_live(),
        "the slot must be registered before its first chunk is admitted"
    );
    assert!(
        streams.has_pending_send(),
        "an unaffordable first chunk must wait as a pending admission"
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn a_released_account_admits_the_pending_first_chunk(
    unaffordable_first_chunk: FirstChunk,
    stalled_handler: StreamingHandler,
    tight_budget: ChunkBudget,
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    let held_body = retain_bytes_outside_the_slot(&streams).await;
    streams.start(unaffordable_first_chunk, &stalled_handler, tight_budget);

    // The earlier body releases the bytes it was holding.
    drop(held_body);
    let event = streams.drive_pending(Duration::from_secs(1)).await;
    assert!(
        matches!(event, PendingEvent::Send(Ok(()))),
        "the first chunk should be admitted once the account drains"
    );
    streams.complete_send(Ok(()));

    assert!(!streams.has_pending_send(), "the admission is finished");
    assert_eq!(
        streams.retained_bytes(),
        b"abcd".len(),
        "the delivered chunk is charged to the connection account"
    );
}

/// Queue a chunk on the connection account that only `drop` releases.
///
/// This stands in for a handler that moved its body stream into a task the
/// connection no longer drives.
async fn retain_bytes_outside_the_slot(streams: &ConnectionStreams) -> RequestBodyStream {
    let (sender, body) = accounted_body_channel(1, streams.queued());
    sender
        .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
        .await
        .expect("the consumer is still open");
    body
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

    let outcome = deliver_then_await(
        Duration::from_secs(1),
        io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
        sender,
        &mut handler,
    )
    .await;

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

    let outcome = deliver_then_await(
        short_wait,
        io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
        sender,
        &mut handler,
    )
    .await;

    // Keep the receiver alive until the wait expires, so the delivery really is
    // stalled on channel capacity rather than on a closed channel.
    drop(body);
    assert_eq!(
        outcome,
        HandlerWait::Deadline,
        "an unresponsive handler should still be bounded by the wait"
    );
}

/// A handler that finishes normally reports its own outcome, not a deadline.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_handler_reply_is_reported_as_replied() {
    let mut handler: HandlerFuture =
        Box::pin(async { Some(Bytes::from_static(b"reply")) });

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
