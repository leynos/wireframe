//! Unit tests for the bytes charged against a body chunk's admission.

use std::collections::VecDeque;

use bytes::Bytes;

use super::{ParkedMessage, retained_outside_queue};
use crate::{app::Envelope, request::RequestParts};

/// Build a queue holding one message of each parked shape.
fn mixed_queue() -> VecDeque<ParkedMessage> {
    VecDeque::from([
        ParkedMessage::Envelope(Envelope::new(9, None, vec![b'e'; 2])),
        ParkedMessage::Stream {
            parts: RequestParts::new(10, None, vec![b'm']),
            body: Bytes::from_static(b"body"),
        },
    ])
}

#[test]
fn every_retained_holder_counts_towards_the_chunk_budget() {
    // Assembly 3 + fragments 5 + parked (envelope 2, stream metadata 1 +
    // body 4) = 15. Dropping the parked queue would under-report by 7 and
    // admit a chunk the aggregate account cannot hold.
    assert_eq!(retained_outside_queue(3, 5, &mixed_queue()), 15);
}

#[test]
fn an_empty_parked_queue_retains_only_assembly_and_fragments() {
    assert_eq!(retained_outside_queue(3, 5, &VecDeque::new()), 8);
}
