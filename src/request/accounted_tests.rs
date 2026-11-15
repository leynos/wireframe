//! Tests for the byte-accounted request-body channel.
//!
//! The channel must release queued bytes exactly when the consumer takes a
//! chunk or drops the body, so the connection's aggregate cap measures what it
//! really retains. The empty-queue exception is also pinned here: without it a
//! chunk larger than the remaining budget could never make progress.

use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tokio::time::timeout;

use super::{QueuedBytes, SendChunkError, accounted_body_channel};

/// Bound used for every admission in these tests.
const LIMIT: usize = 8;
/// Long enough for a test to observe a blocked admission.
const BLOCKED: Duration = Duration::from_millis(50);
/// Generous bound for an admission that must resume.
const RESUMES: Duration = Duration::from_secs(5);

#[tokio::test]
async fn yielding_a_chunk_releases_its_queued_bytes() {
    let queued = QueuedBytes::new();
    let (sender, mut stream) = accounted_body_channel(4, queued.clone());

    sender
        .send_chunk(Bytes::from_static(b"hello"), 0, LIMIT)
        .await
        .expect("chunk should be admitted");
    assert_eq!(queued.queued(), 5, "queued bytes are charged on send");

    let chunk = stream.next().await.expect("stream should yield a chunk");
    assert_eq!(
        chunk.expect("chunk should be ok"),
        Bytes::from_static(b"hello")
    );
    assert_eq!(
        queued.queued(),
        0,
        "taking the chunk must release its bytes"
    );
}

#[tokio::test]
async fn dropping_the_body_releases_queued_bytes() {
    let queued = QueuedBytes::new();
    let (sender, stream) = accounted_body_channel(4, queued.clone());

    sender
        .send_chunk(Bytes::from_static(b"hello"), 0, LIMIT)
        .await
        .expect("chunk should be admitted");
    assert_eq!(queued.queued(), 5, "queued bytes are charged on send");

    drop(stream);
    assert_eq!(
        queued.queued(),
        0,
        "dropping the body must release its bytes"
    );
}

#[tokio::test]
async fn admission_waits_while_the_combined_limit_is_full() {
    let queued = QueuedBytes::new();
    let (sender, mut stream) = accounted_body_channel(4, queued.clone());

    sender
        .clone()
        .send_chunk(Bytes::from_static(b"aaaa"), 0, LIMIT)
        .await
        .expect("first chunk should be admitted");
    assert_eq!(queued.queued(), 4);

    // 4 queued + 4 retained elsewhere + 4 new bytes exceeds the limit, so the
    // second admission must wait for the consumer to release room.
    let pending = sender.send_chunk(Bytes::from_static(b"bbbb"), 4, LIMIT);
    tokio::pin!(pending);
    assert!(
        timeout(BLOCKED, &mut pending).await.is_err(),
        "admission must wait while the combined limit is full"
    );

    let released = stream.next().await.expect("stream should yield a chunk");
    assert_eq!(
        released.expect("chunk should be ok"),
        Bytes::from_static(b"aaaa")
    );
    assert_eq!(queued.queued(), 0, "the consumer released the queued bytes");

    timeout(RESUMES, pending)
        .await
        .expect("admission should resume once room is released")
        .expect("second chunk should be admitted");
    assert_eq!(queued.queued(), 4, "the resumed chunk is now queued");
}

#[tokio::test]
async fn an_oversized_chunk_is_admitted_to_an_empty_queue() {
    let queued = QueuedBytes::new();
    let (sender, mut stream) = accounted_body_channel(1, queued.clone());
    let chunk = Bytes::from_static(b"0123456789");

    timeout(BLOCKED, sender.send_chunk(chunk.clone(), 0, 4))
        .await
        .expect("an oversized chunk must not wait for room that cannot appear")
        .expect("the chunk should be admitted");
    assert_eq!(queued.queued(), 10, "the oversized chunk is fully charged");

    let delivered = stream.next().await.expect("stream should yield a chunk");
    assert_eq!(delivered.expect("chunk should be ok"), chunk);
    assert_eq!(
        queued.queued(),
        0,
        "taking the chunk releases all ten bytes"
    );
}

#[tokio::test]
async fn sending_after_the_handler_drops_its_body_reports_closed() {
    let queued = QueuedBytes::new();
    let (sender, stream) = accounted_body_channel(2, queued.clone());
    drop(stream);

    let error = sender
        .send_chunk(Bytes::from_static(b"orphan"), 0, LIMIT)
        .await
        .expect_err("send should fail when the handler dropped its body");
    assert_eq!(error, SendChunkError::Closed);
    assert_eq!(queued.queued(), 0, "a refused chunk is never charged");
}

#[tokio::test]
async fn a_terminal_error_carries_no_queued_bytes() {
    let queued = QueuedBytes::new();
    let (sender, mut stream) = accounted_body_channel(2, queued.clone());

    sender
        .clone()
        .send_chunk(Bytes::from_static(b"keep"), 0, LIMIT)
        .await
        .expect("chunk should be admitted");
    sender
        .send_terminal(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "cut short",
        ))
        .await
        .expect("terminal error should be queued");

    assert_eq!(queued.queued(), 4, "only the body chunk is charged");

    let first = stream.next().await.expect("stream should yield the chunk");
    assert_eq!(
        first.expect("chunk should be ok"),
        Bytes::from_static(b"keep")
    );
    let second = stream.next().await.expect("stream should yield the error");
    assert_eq!(
        second.expect_err("terminal error should surface").kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        queued.queued(),
        0,
        "both queued values released their bytes"
    );
}
