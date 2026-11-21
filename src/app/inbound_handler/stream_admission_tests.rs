//! Admission of the slot's chunks against bytes the connection holds elsewhere.
//!
//! A chunk is admitted against the connection-wide account, so bytes an earlier
//! body, an assembly, or a parked message still holds can keep it waiting.
//! Waiting inside the slot would deadlock, because nothing can release those
//! bytes until the loop regains control, so every case here proves that a
//! refused chunk waits, and that the same chunk is admitted once the bytes that
//! refused it are released.

use std::{
    future,
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use rstest::{fixture, rstest};

use super::{super::stream_active::PendingEvent, ConnectionStreams};
use crate::{
    app::{
        StreamingHandler,
        inbound_handler::stream_handles::{ChunkBudget, ChunkOffer, FirstChunk},
    },
    message_assembler::{
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
    },
    request::{QueuedBytes, RequestBodyStream, RequestParts, accounted_body_channel},
};

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

/// A first chunk carrying a message key a continuation can name.
#[fixture]
fn keyed_first_chunk() -> FirstChunk {
    FirstChunk {
        key: Some(MessageKey(3)),
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
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    let _held_body = retain_bytes_outside_the_slot(&streams).await;

    streams.start(unaffordable_first_chunk, &stalled_handler);

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
    streams.start(unaffordable_first_chunk, &stalled_handler);

    // The earlier body releases the bytes it was holding.
    drop(held_body);
    let event = streams
        .drive_pending(Duration::from_secs(1), tight_budget)
        .await;
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

/// A chunk refused against a snapshot of outside bytes is admitted later.
///
/// The budget is measured again for every drive, so bytes released between
/// drives are seen. A measurement kept from the moment the chunk was queued
/// would refuse it forever, because it can only overstate what is retained
/// elsewhere.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_chunk_refused_by_outside_bytes_is_admitted_once_they_are_released(
    unaffordable_first_chunk: FirstChunk,
    stalled_handler: StreamingHandler,
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    // Four bytes are already charged to the connection account, so the
    // empty-account exception cannot admit the chunk on its own.
    let _held_body = retain_bytes_outside_the_slot(&streams).await;
    streams.start(unaffordable_first_chunk, &stalled_handler);

    // An assembly still holds four more, which leave no room for the chunk.
    let crowded = ChunkBudget {
        other_retained: 4,
        limit: 11,
    };
    let refused = streams.drive_pending(Duration::from_secs(1), crowded).await;
    assert!(
        matches!(refused, PendingEvent::Idle),
        "the chunk must wait while the assembly holds bytes"
    );

    // The assembly expires, so the same chunk now fits.
    let freed = ChunkBudget {
        other_retained: 0,
        limit: 11,
    };
    let admitted = streams.drive_pending(Duration::from_secs(1), freed).await;
    assert!(
        matches!(admitted, PendingEvent::Send(Ok(()))),
        "the released bytes must let the queued chunk in"
    );
}

/// An assembly purge releases the bytes that were refusing a pending chunk.
///
/// This is the release path the loop relies on: an assembly that expires stops
/// counting towards `other_retained` before the next admission drive, so the
/// chunk that had been refused is admitted without the chunk itself changing.
#[rstest]
#[tokio::test(start_paused = true)]
async fn an_assembly_purge_releases_the_bytes_refusing_a_pending_chunk(
    unaffordable_first_chunk: FirstChunk,
    stalled_handler: StreamingHandler,
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    let _held_body = retain_bytes_outside_the_slot(&streams).await;
    streams.start(unaffordable_first_chunk, &stalled_handler);

    // Four bytes are charged to the queue and four more are retained by an
    // assembly that outlives its reassembly timeout without completing.
    let start = Instant::now();
    let mut assembly = retaining_assembly(start);
    let crowded = ChunkBudget {
        other_retained: assembly.total_buffered_bytes(),
        limit: 11,
    };
    let refused = streams.drive_pending(Duration::from_secs(1), crowded).await;
    assert!(
        matches!(refused, PendingEvent::Idle),
        "the chunk must wait while the assembly retains its bytes"
    );

    let purged = assembly.purge_expired_detailed_at(start + Duration::from_secs(31));
    assert!(
        !purged.buffered().is_empty(),
        "the stalled assembly must actually expire"
    );
    let freed = ChunkBudget {
        other_retained: assembly.total_buffered_bytes(),
        limit: 11,
    };
    let admitted = streams.drive_pending(Duration::from_secs(1), freed).await;
    assert!(
        matches!(admitted, PendingEvent::Send(Ok(()))),
        "the purged assembly must release the bytes the chunk was waiting for"
    );
}

/// A continuation chunk follows its first chunk through the same admission.
///
/// Only one chunk may wait at a time. A continuation offered while the first
/// chunk is still unadmitted is refused rather than queued, because the
/// refusal is indistinguishable from an abandoned body and the caller then
/// discards the streaming assembly.
#[rstest]
#[tokio::test(start_paused = true)]
async fn a_continuation_chunk_is_admitted_only_after_the_first_chunk(
    keyed_first_chunk: FirstChunk,
    stalled_handler: StreamingHandler,
) {
    let mut streams = ConnectionStreams::new(QueuedBytes::new());
    let _held_body = retain_bytes_outside_the_slot(&streams).await;
    let key = keyed_first_chunk
        .key
        .expect("the fixture carries a message key");
    streams.start(keyed_first_chunk, &stalled_handler);

    let continuation = || ChunkOffer {
        key,
        chunk: Bytes::from_static(b"next"),
        is_last: true,
    };
    assert!(
        !streams.push_chunk(continuation()),
        "a continuation must not be queued behind an unadmitted chunk"
    );

    let roomy = ChunkBudget {
        other_retained: 0,
        limit: usize::MAX,
    };
    let first = streams.drive_pending(Duration::from_secs(1), roomy).await;
    assert!(
        matches!(first, PendingEvent::Send(Ok(()))),
        "the first chunk must be admitted once the account has room"
    );
    streams.complete_send(Ok(()));

    assert!(
        streams.push_chunk(continuation()),
        "the continuation is accepted once no admission is in flight"
    );
    let second = streams.drive_pending(Duration::from_secs(1), roomy).await;
    assert!(
        matches!(second, PendingEvent::Send(Ok(()))),
        "the continuation must reach the handler after the first chunk"
    );
    streams.complete_send(Ok(()));

    assert!(
        !streams.has_pending_send(),
        "the final chunk closes the body"
    );
    assert_eq!(
        streams.retained_bytes(),
        b"held".len() + b"abcd".len() + b"next".len(),
        "every delivered chunk is charged to the connection account"
    );
}

/// Build assembly state holding four buffered body bytes.
fn retaining_assembly(start: Instant) -> MessageAssemblyState {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(1024).expect("the per-message cap is non-zero"),
        Duration::from_secs(30),
    );
    let header = FirstFrameHeader {
        message_key: MessageKey(7),
        metadata_len: 0,
        body_len: 4,
        total_body_len: None,
        is_last: false,
    };
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], b"abcd")
        .expect("the first frame fits the per-message cap");
    state
        .accept_first_frame_at(input, start)
        .expect("the first frame is admissible");
    state
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
