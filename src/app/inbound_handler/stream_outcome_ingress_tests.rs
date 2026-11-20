//! Accounting for the outcomes recorded while inbound frames are routed.
//!
//! A streaming route with no handler and a continuation for a body that has
//! already gone are both decided here, before any exchange exists to report
//! them. Neither case has a handler that could observe the body, so the count
//! is the only record: a missing handler would otherwise look like a body that
//! was delivered, and a dropped continuation like a complete one.

use bytes::Bytes;
use tracing_test::traced_test;

use super::{deliver_chunk, start_stream};
use crate::{
    app::inbound_handler::{
        stream_handles::{ChunkOffer, FirstChunk, LoopState},
        stream_outcome_support::{counted, final_chunk, reading_route, recorded},
    },
    message_assembler::{MessageAssemblyState, MessageKey},
    metrics::StreamingBodyOutcome,
    request::QueuedBytes,
};

/// An opening chunk that carries an assembly key, so continuations can match it.
///
/// The body stays open because the chunk does not end it: `start` keeps a
/// sender for the live exchange only while more chunks are expected.
fn keyed_first_chunk() -> FirstChunk {
    FirstChunk {
        key: Some(MessageKey(1)),
        is_last: false,
        ..final_chunk()
    }
}

/// A continuation for the keyed message.
fn continuation() -> ChunkOffer {
    ChunkOffer {
        key: MessageKey(1),
        chunk: Bytes::from_static(b"more"),
        is_last: true,
    }
}

/// A start with no registered handler is counted and starts nothing.
///
/// The frame is dropped rather than queued: a route that never registered a
/// streaming handler cannot answer, so holding its body open would only keep
/// the dispatch slot busy.
#[test]
fn a_start_without_a_handler_is_counted_as_no_handler() {
    let mut state = LoopState::new(QueuedBytes::new());

    let (_, no_handler) = counted(StreamingBodyOutcome::NoHandler, || {
        start_stream(&mut state, None, final_chunk());
    });

    assert_eq!(no_handler, 1, "the missing handler is counted once");
    assert!(
        !state.streams.is_live(),
        "a body with no handler must not occupy the slot"
    );
}

/// A start with a registered handler occupies the slot and counts nothing.
#[test]
fn a_start_with_a_handler_registers_the_exchange() {
    let handler = reading_route();
    let mut state = LoopState::new(QueuedBytes::new());

    let (_, counts) = recorded(|| {
        start_stream(&mut state, Some(&handler), final_chunk());
    });

    assert!(
        state.streams.is_live(),
        "the handler's exchange should own the slot"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::NoHandler),
        0,
        "a resolved handler is not a missing one"
    );
}

/// A continuation for an abandoned body is counted, not silently dropped.
///
/// The body it belonged to has already been reported as abandoned, but this
/// frame still arrived and still has to be discarded; counting it as a second
/// abandoned body would overstate the damage, so the outcome names the dropped
/// chunk's own body.
#[traced_test]
#[test]
fn a_continuation_for_an_abandoned_body_is_counted_as_abandoned() {
    let mut state = LoopState::new(QueuedBytes::new());
    let mut assembly: Option<MessageAssemblyState> = None;

    let (_, abandoned) = counted(StreamingBodyOutcome::Abandoned, || {
        deliver_chunk(&mut state, &mut assembly, continuation());
    });

    assert_eq!(abandoned, 1, "the dropped continuation is counted once");
    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .any(|line| {
                line.contains("dropping chunk for abandoned streaming message")
                    && line.contains("abandoned_body")
            })
            .then_some(())
            .ok_or_else(|| "the dropped continuation is not reported".to_owned())
    });
}

/// A continuation for the live body is queued and counts nothing.
#[test]
fn a_continuation_for_the_live_body_is_not_counted() {
    let handler = reading_route();
    let mut state = LoopState::new(QueuedBytes::new());
    start_stream(&mut state, Some(&handler), keyed_first_chunk());
    // The first chunk's own admission has to finish before a continuation can
    // be queued behind it; this test is about routing, not admission.
    if let Some(active) = state.streams.active.as_mut() {
        active.pending = None;
    }
    let mut assembly: Option<MessageAssemblyState> = None;

    let (_, counts) = recorded(|| {
        deliver_chunk(&mut state, &mut assembly, continuation());
    });

    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        0,
        "a continuation for the live body is not an abandoned chunk"
    );
    assert!(
        state
            .streams
            .active
            .as_ref()
            .is_some_and(|active| active.pending.is_some()),
        "the continuation should be waiting for admission"
    );
}