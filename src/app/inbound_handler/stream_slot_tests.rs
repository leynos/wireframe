//! Unit tests for the connection-owned streaming slot.

use std::{future, io, time::Duration};

use bytes::Bytes;
use futures::StreamExt;

use super::{
    super::stream_terminal::deliver_then_await,
    ActiveStream,
    HandlerFuture,
    PendingEvent,
    PendingSend,
};
use crate::request::{AccountedBodySender, QueuedBytes, RequestBodyStream, accounted_body_channel};

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

/// Build a slot whose handler and pending admission never make progress.
fn stalled_stream() -> ActiveStream {
    ActiveStream {
        key: None,
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
