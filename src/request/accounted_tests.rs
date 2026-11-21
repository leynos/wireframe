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

/// Two producers sharing an account must not both pass admission.
///
/// Admission used to read the account and charge it only once channel
/// capacity arrived. A second producer could charge during that wait, so the
/// waiting chunk pushed the account past the limit when it finally landed.
#[tokio::test]
async fn admission_reserves_bytes_while_it_waits_for_channel_capacity() {
    let queued = QueuedBytes::new();
    let (first, mut first_body) = accounted_body_channel(1, queued.clone());
    let (second, second_body) = accounted_body_channel(4, queued.clone());

    // A zero-byte chunk fills the first channel without touching the account,
    // so the next send there must wait for capacity rather than for room.
    first
        .clone()
        .send_chunk(Bytes::from_static(b""), 0, LIMIT)
        .await
        .expect("an empty queue admits one chunk");
    assert_eq!(queued.queued(), 0);

    let waiting = first.send_chunk(Bytes::from_static(b"aaaa"), 0, LIMIT);
    tokio::pin!(waiting);
    assert!(
        timeout(BLOCKED, &mut waiting).await.is_err(),
        "the send should wait for the channel slot"
    );
    assert_eq!(
        queued.queued(),
        4,
        "a chunk waiting for capacity still holds its reservation"
    );

    second
        .clone()
        .send_chunk(Bytes::from_static(b"bbbb"), 0, LIMIT)
        .await
        .expect("the remaining room admits the second chunk");
    assert_eq!(queued.queued(), LIMIT, "the account is now exactly full");

    let overflowing = second.send_chunk(Bytes::from_static(b"cccc"), 0, LIMIT);
    tokio::pin!(overflowing);
    assert!(
        timeout(BLOCKED, &mut overflowing).await.is_err(),
        "reserved bytes must count against the limit for every producer"
    );

    let filling = first_body.next().await.expect("the channel should yield");
    assert!(
        filling.expect("the chunk is not an error").is_empty(),
        "the zero-byte chunk frees the channel slot"
    );
    timeout(RESUMES, waiting)
        .await
        .expect("the waiting chunk should land once capacity frees")
        .expect("the waiting chunk was admitted");

    assert_eq!(
        queued.queued(),
        LIMIT,
        "the account never exceeds its limit"
    );
    drop(second_body);
}

/// Abandoning a send releases the room it reserved.
#[tokio::test]
async fn dropping_a_waiting_send_releases_its_reservation() {
    let queued = QueuedBytes::new();
    let (sender, _body) = accounted_body_channel(1, queued.clone());
    sender
        .clone()
        .send_chunk(Bytes::from_static(b""), 0, LIMIT)
        .await
        .expect("an empty queue admits one chunk");

    // Boxed so that dropping the binding drops the future itself: a `pin!`
    // binding only pins a hidden local that outlives the reference.
    let mut waiting = Box::pin(sender.send_chunk(Bytes::from_static(b"aaaa"), 0, LIMIT));
    assert!(
        timeout(BLOCKED, &mut waiting).await.is_err(),
        "the send should wait for the channel slot"
    );
    assert_eq!(queued.queued(), 4, "the waiting send reserved its bytes");

    drop(waiting);
    assert_eq!(
        queued.queued(),
        0,
        "an abandoned send must not keep its reservation"
    );
}

/// A waiting admission must observe a closed receiver directly.
///
/// A dropped receiver only releases the chunks it still holds, so closing it
/// does not necessarily notify the byte account. Here the queued bytes belong
/// to an earlier body that shares the connection account, so nothing wakes the
/// parked admission except the channel closing.
#[tokio::test]
async fn a_waiting_admission_observes_a_closed_receiver() {
    let queued = QueuedBytes::new();
    let (earlier, _earlier_body) = accounted_body_channel(1, queued.clone());
    earlier
        .send_chunk(Bytes::from_static(b"0123456789"), 0, 3)
        .await
        .expect("an empty queue admits the oversized chunk");

    let (sender, body) = accounted_body_channel(4, queued.clone());
    let pending = sender.send_chunk(Bytes::from_static(b"bbbb"), 0, 3);
    tokio::pin!(pending);
    assert!(
        timeout(BLOCKED, &mut pending).await.is_err(),
        "admission must wait while the shared account is full"
    );

    drop(body);

    assert_eq!(
        timeout(RESUMES, pending)
            .await
            .expect("a closed receiver should wake the waiting admission"),
        Err(SendChunkError::Closed),
    );
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
