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

/// Failure count each case starts from.
///
/// A non-zero start is what makes "kept" and "cleared" distinguishable; a
/// count of zero would pass whether or not the outcome reset it.
const STARTING_FAILURES: u32 = 3;

/// Run one outcome through the counter and return the resulting count.
fn failures_after(outcome: &FrameOutcome) -> u32 {
    let mut failures = STARTING_FAILURES;
    clear_deser_failures_after(outcome, &mut failures);
    failures
}

/// Assert that an outcome leaves the malformed-input count untouched.
fn assert_count_kept(outcome: &FrameOutcome, reason: &str) {
    assert_eq!(failures_after(outcome), STARTING_FAILURES, "{reason}");
}

/// Assert that an outcome ends the current run of malformed input.
fn assert_count_cleared(outcome: &FrameOutcome, reason: &str) {
    assert_eq!(failures_after(outcome), 0, "{reason}");
}

/// A continuation frame for message 7, optionally the message's last frame.
fn stream_chunk(is_last: bool) -> FrameOutcome {
    FrameOutcome::StreamChunk {
        key: MessageKey(7),
        chunk: Bytes::from_static(b"cd"),
        is_last,
    }
}

/// A terminal streaming rejection was counted before its outcome was built.
///
/// The reset used to cover this outcome, so a peer that alternated a rejected
/// continuation with a well-formed frame could keep the counter at zero and
/// avoid the malformed-input limit indefinitely.
#[test]
fn a_terminal_streaming_failure_keeps_the_deserialization_failure_count() {
    assert_count_kept(
        &FrameOutcome::StreamFailed {
            key: MessageKey(7),
            error: io::Error::new(io::ErrorKind::InvalidData, "declared total mismatch"),
        },
        "the recorded rejection must survive the frame",
    );
}

/// A pending frame may be an accepted partial or a rejected frame.
#[test]
fn a_pending_outcome_keeps_the_deserialization_failure_count() {
    assert_count_kept(
        &FrameOutcome::Pending,
        "a partial must not mask earlier failures",
    );
}

/// A streaming first frame that is not the last frame completes nothing.
///
/// A peer could otherwise send a declared total, deliver too few bytes, and
/// start a fresh message after every rejection: each rejection is counted, but
/// the next first frame cleared the count before the limit was reached.
#[test]
fn an_incomplete_streaming_start_keeps_the_deserialization_failure_count() {
    assert_count_kept(
        &FrameOutcome::StreamStarted {
            key: None,
            assembly: None,
            parts: RequestParts::new(1, None, Vec::new()),
            chunk: Bytes::from_static(b"ab"),
            is_last: false,
        },
        "an unfinished message must not end the failure run",
    );
}

/// A continuation that does not finish the message completes nothing either.
#[test]
fn an_incomplete_streaming_chunk_keeps_the_deserialization_failure_count() {
    assert_count_kept(
        &stream_chunk(false),
        "a validated continuation is not a completed message",
    );
}

/// The chunk that completes a streaming message ends the failure run.
#[test]
fn a_final_streaming_chunk_clears_the_deserialization_failure_count() {
    assert_count_cleared(&stream_chunk(true), "the message arrived whole");
}

/// A complete message for a streaming route arrives whole while the slot is busy.
#[test]
fn a_streaming_fallback_clears_the_deserialization_failure_count() {
    assert_count_cleared(
        &FrameOutcome::StreamFallback {
            parts: RequestParts::new(1, None, Vec::new()),
            body: Bytes::from_static(b"whole"),
        },
        "the assembled message arrived whole",
    );
}

/// A discarded continuation validated, but the message never completed.
#[test]
fn a_discarded_continuation_keeps_the_deserialization_failure_count() {
    assert_count_kept(
        &FrameOutcome::Discarded,
        "discarding a continuation establishes no completion",
    );
}

/// A frame the pipeline accepted wholesale ends the current failure run.
#[test]
fn an_accepted_frame_clears_the_deserialization_failure_count() {
    assert_count_cleared(
        &FrameOutcome::Buffered(Envelope::new(9, None, Vec::new())),
        "a well-formed frame ends the failure run",
    );
}
