//! Unit tests for the connection-owned streaming slot.

use std::{future, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::StreamExt;

use super::{
    super::stream_terminal::deliver_then_await,
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

#[tokio::test(start_paused = true)]
async fn a_stalled_admission_returns_control_after_the_read_timeout() {
    let mut stream = stalled_stream();

    let event = stream.drive_pending(Duration::from_millis(50)).await;

    assert!(
        matches!(event, PendingEvent::Idle),
        "a stalled admission should hand control back to the loop"
    );
}

#[tokio::test(start_paused = true)]
async fn a_resolved_handler_wins_over_a_stalled_admission() {
    let mut stream = stalled_stream();
    stream.handler = Box::pin(async { Some(Bytes::from_static(b"reply")) });

    match stream.drive_pending(Duration::from_secs(1)).await {
        PendingEvent::Handler(outcome) => {
            assert_eq!(outcome.as_deref(), Some(b"reply".as_slice()));
        }
        _ => panic!("the handler outcome should win over a stalled admission"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_completed_admission_wins_over_a_stalled_handler() {
    let mut stream = stalled_stream();
    stream.pending = Some(PendingSend {
        future: Box::pin(async { Ok(()) }),
        is_last: true,
    });

    match stream.drive_pending(Duration::from_secs(1)).await {
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
#[tokio::test(start_paused = true)]
async fn an_unaffordable_first_chunk_becomes_a_pending_admission() {
    let mut streams = ConnectionStreams::new();
    let _held_body = retain_bytes_outside_the_slot(&streams).await;

    streams.start(first_chunk(b"abcd"), &stalled_handler(), tight_budget());

    assert!(
        streams.is_live(),
        "the slot must be registered before its first chunk is admitted"
    );
    assert!(
        streams.has_pending_send(),
        "an unaffordable first chunk must wait as a pending admission"
    );
}

#[tokio::test(start_paused = true)]
async fn a_released_account_admits_the_pending_first_chunk() {
    let mut streams = ConnectionStreams::new();
    let held_body = retain_bytes_outside_the_slot(&streams).await;
    streams.start(first_chunk(b"abcd"), &stalled_handler(), tight_budget());

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
    let (sender, body) = accounted_body_channel(1, streams.queued.clone());
    sender
        .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
        .await
        .expect("the consumer is still open");
    body
}

/// Build a first chunk whose four bytes do not fit beside the held ones.
fn first_chunk(chunk: &'static [u8]) -> FirstChunk {
    FirstChunk {
        key: None,
        assembly: None,
        parts: RequestParts::new(1, None, Vec::new()),
        chunk: Bytes::from_static(chunk),
        is_last: false,
    }
}

/// Allow less room than the held bytes and the first chunk need together.
fn tight_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 6,
    }
}

/// Handler that holds its body open without ever consuming a chunk.
fn stalled_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(async move {
            let _body = body;
            future::pending::<Option<Bytes>>().await
        })
    })
}

/// Build a slot whose handler and pending admission never make progress.
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
/// A terminal error must reach a handler whose body channel is already full.
///
/// Redelivering it before polling the handler would deadlock: the error needs a
/// channel slot, and only the handler can free one.
#[tokio::test(start_paused = true)]
async fn a_terminal_error_reaches_a_handler_whose_channel_is_full() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
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
        outcome.as_deref(),
        Some(b"error".as_slice()),
        "the handler should observe the failure, not a clean end of stream"
    );
}

#[tokio::test(start_paused = true)]
async fn terminal_delivery_is_bounded_when_the_handler_never_reads() {
    let (sender, body) = accounted_body_channel(1, QueuedBytes::new());
    queue_one_chunk(&sender).await;
    let mut handler: HandlerFuture = Box::pin(future::pending());

    let outcome = deliver_then_await(
        Duration::from_millis(50),
        io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
        sender,
        &mut handler,
    )
    .await;

    // Keep the receiver alive until the wait expires, so the delivery really is
    // stalled on channel capacity rather than on a closed channel.
    drop(body);
    assert_eq!(
        outcome, None,
        "an unresponsive handler should still be bounded by the wait"
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
