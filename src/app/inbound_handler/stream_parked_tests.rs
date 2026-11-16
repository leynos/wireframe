//! Unit tests for the parked-message queue bounds.

use std::{collections::VecDeque, time::Duration};

use bytes::Bytes;
use futures::StreamExt;

use super::{MAX_PARKED_MESSAGES, ParkedMessage, deliver_prefilled, park};
use crate::{
    app::{Envelope, inbound_handler::stream_handles::ChunkBudget},
    request::{QueuedBytes, accounted_body_channel},
};

/// An empty payload is the case the byte-based memory budget cannot see.
fn empty_envelope() -> ParkedMessage { ParkedMessage::Envelope(Envelope::new(9, None, Vec::new())) }

#[test]
fn park_refuses_to_grow_past_the_cap() {
    let mut parked = VecDeque::new();
    for _ in 0..MAX_PARKED_MESSAGES {
        assert!(
            park(&mut parked, empty_envelope()).is_ok(),
            "every message below the cap should be parked"
        );
    }
    assert_eq!(parked.len(), MAX_PARKED_MESSAGES);

    let error = park(&mut parked, empty_envelope())
        .expect_err("a full queue should refuse another message");

    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidData,
        "the refusal should end the connection as invalid peer input"
    );
    assert_eq!(
        parked.len(),
        MAX_PARKED_MESSAGES,
        "a refused message should not be queued"
    );
}

#[test]
fn park_accepts_a_message_once_the_queue_drains() {
    let mut parked = VecDeque::new();
    for _ in 0..MAX_PARKED_MESSAGES {
        assert!(park(&mut parked, empty_envelope()).is_ok());
    }
    assert!(
        parked.pop_front().is_some(),
        "the queue should have an entry"
    );

    assert!(
        park(&mut parked, empty_envelope()).is_ok(),
        "draining one message should free a place for the next"
    );
}

/// Bytes an earlier body never released must not block a pre-filled body.
#[tokio::test(start_paused = true)]
async fn an_unaffordable_prefilled_body_becomes_a_terminal_error() {
    let queued = QueuedBytes::new();
    let (holder, _held_body) = accounted_body_channel(1, queued.clone());
    holder
        .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
        .await
        .expect("an empty queue admits one chunk");
    let (sender, mut body) = accounted_body_channel(1, queued);

    deliver_prefilled(&sender, Bytes::from_static(b"abcd"), tight_budget(), wait()).await;

    let item = body
        .next()
        .await
        .expect("the handler must observe the abandoned body");
    assert!(
        item.is_err(),
        "a body that cannot be admitted must fail, not queue silently"
    );
}

#[tokio::test(start_paused = true)]
async fn an_affordable_prefilled_body_arrives_whole() {
    let (sender, mut body) = accounted_body_channel(1, QueuedBytes::new());

    deliver_prefilled(&sender, Bytes::from_static(b"abcd"), tight_budget(), wait()).await;

    let item = body
        .next()
        .await
        .expect("the assembled body should arrive as one chunk");
    assert_eq!(
        item.expect("the chunk is not an error").as_ref(),
        b"abcd",
        "the whole body should be delivered in one chunk"
    );
}

/// Allow less room than four body bytes plus the held ones need.
fn tight_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 6,
    }
}

/// Bound used by the pre-filled delivery tests.
fn wait() -> Duration { Duration::from_millis(50) }
