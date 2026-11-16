//! Unit tests for the parked-message queue bounds.

use std::collections::VecDeque;

use super::{MAX_PARKED_MESSAGES, ParkedMessage, park};
use crate::app::Envelope;

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
    assert!(parked.pop_front().is_some(), "the queue should have an entry");

    assert!(
        park(&mut parked, empty_envelope()).is_ok(),
        "draining one message should free a place for the next"
    );
}