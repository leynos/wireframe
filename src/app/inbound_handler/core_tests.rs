//! Tests for the frame-processing core of the inbound handler.
//!
//! The counter exercised here decides when a connection is closed for
//! malformed input, so each case is a named example rather than a table: the
//! name states why that outcome must, or must not, reset the count.

use std::io;

use super::clear_deser_failures_after;
use crate::{
    app::{Envelope, frame_handling::FrameOutcome},
    message_assembler::MessageKey,
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

/// A frame the pipeline accepted wholesale ends the current failure run.
#[test]
fn an_accepted_frame_clears_the_deserialization_failure_count() {
    let mut failures = 3;
    let outcome = FrameOutcome::Buffered(Envelope::new(9, None, Vec::new()));

    clear_deser_failures_after(&outcome, &mut failures);

    assert_eq!(failures, 0, "a well-formed frame ends the failure run");
}
