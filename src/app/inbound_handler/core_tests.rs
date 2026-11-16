//! Tests for the frame-processing core of the inbound handler.
//!
//! The counter exercised here decides when a connection is closed for
//! malformed input, so each case is a named example rather than a table: the
//! name states why that outcome must, or must not, reset the count.

use std::io;

use bytes::Bytes;

use super::clear_deser_failures_after;
use crate::{
    app::{Envelope, frame_handling::FrameOutcome},
    message_assembler::MessageKey,
    request::RequestParts,
};

/// A terminal streaming rejection was counted before its outcome was built.
///
/// The reset used to cover this outcome, so a peer that alternated a rejected
/// continuation with a well-formed frame could keep the counter at zero and
/// avoid the malformed-input limit indefinitely.
#[test]
fn a_terminal_streaming_failure_keeps_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::StreamFailed {
        key: MessageKey(7),
        error: io::Error::new(io::ErrorKind::InvalidData, "declared total mismatch"),
    };

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(failures, 3, "the recorded rejection must survive the frame");
}

/// A pending frame may be an accepted partial or a rejected frame.
#[test]
fn a_pending_outcome_keeps_the_deserialization_failure_count() {
    let mut failures = 3;

    clear_deser_failures_after(&FrameOutcome::Pending, &mut failures);

    assert_eq!(failures, 3, "a partial must not mask earlier failures");
}

/// A streaming first frame that is not the last frame completes nothing.
///
/// A peer could otherwise send a declared total, deliver too few bytes, and
/// start a fresh message after every rejection: each rejection is counted, but
/// the next first frame cleared the count before the limit was reached.
#[test]
fn an_incomplete_streaming_start_keeps_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::StreamStarted {
        key: None,
        assembly: None,
        parts: RequestParts::new(1, None, Vec::new()),
        chunk: Bytes::from_static(b"ab"),
        is_last: false,
    };

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(
        failures, 3,
        "an unfinished message must not end the failure run"
    );
}

/// A continuation that does not finish the message completes nothing either.
#[test]
fn an_incomplete_streaming_chunk_keeps_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::StreamChunk {
        key: MessageKey(7),
        chunk: Bytes::from_static(b"cd"),
        is_last: false,
    };

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(
        failures, 3,
        "a validated continuation is not a completed message"
    );
}

/// The chunk that completes a streaming message ends the failure run.
#[test]
fn a_final_streaming_chunk_clears_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::StreamChunk {
        key: MessageKey(7),
        chunk: Bytes::from_static(b"cd"),
        is_last: true,
    };

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(failures, 0, "the message arrived whole");
}

/// A complete message for a streaming route arrives whole while the slot is busy.
#[test]
fn a_streaming_fallback_clears_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::StreamFallback {
        parts: RequestParts::new(1, None, Vec::new()),
        body: Bytes::from_static(b"whole"),
    };

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(failures, 0, "the assembled message arrived whole");
}

/// A discarded continuation validated, but the message never completed.
#[test]
fn a_discarded_continuation_keeps_the_deserialization_failure_count() {
    let mut failures = 3;

    clear_deser_failures_after(&FrameOutcome::Discarded, &mut failures);

    assert_eq!(
        failures, 3,
        "discarding a continuation establishes no completion"
    );
}

/// A frame the pipeline accepted wholesale ends the current failure run.
#[test]
fn an_accepted_frame_clears_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::Buffered(Envelope::new(9, None, Vec::new()));

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(failures, 0, "a well-formed frame ends the failure run");
}
