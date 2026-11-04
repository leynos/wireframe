//! Unit and property tests for sequence tracking helper logic.

use super::*;
use crate::message_assembler::{FirstFrameHeader, FrameSequence, MessageKey};

fn make_series() -> MessageSeries {
    MessageSeries::from_first_frame(&FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: 10,
        total_body_len: None,
        is_last: false,
    })
}

#[test]
fn advance_sequence_or_overflow_advances_on_normal_increment() {
    let mut series = make_series();
    let seq = FrameSequence(1);
    assert!(series.advance_sequence_or_overflow(seq, false).is_ok());
    assert_eq!(series.next_sequence, Some(FrameSequence(2)));
}

#[test]
fn advance_sequence_or_overflow_allows_overflow_on_last_frame() {
    let mut series = make_series();
    let seq = FrameSequence(u32::MAX);
    assert!(series.advance_sequence_or_overflow(seq, true).is_ok());
    assert_eq!(series.next_sequence, None);
}

#[test]
fn advance_sequence_or_overflow_errors_on_overflow_mid_stream() {
    let mut series = make_series();
    let seq = FrameSequence(u32::MAX);
    let result = series.advance_sequence_or_overflow(seq, false);
    assert!(matches!(
        result,
        Err(MessageSeriesError::SequenceOverflow { last }) if last == seq
    ));
}

#[test]
fn start_sequence_tracking_switches_to_tracked_mode() {
    let mut series = make_series();
    assert_eq!(series.sequence_tracking, SequenceTracking::Untracked);
    let seq = FrameSequence(0);
    let result = series.start_sequence_tracking(seq, false);
    assert!(result.is_ok());
    assert_eq!(series.sequence_tracking, SequenceTracking::Tracked);
    assert_eq!(series.next_sequence, Some(FrameSequence(1)));
}

#[test]
fn start_sequence_tracking_delegates_overflow_on_last() {
    let mut series = make_series();
    let seq = FrameSequence(u32::MAX);
    assert!(series.start_sequence_tracking(seq, true).is_ok());
    assert_eq!(series.sequence_tracking, SequenceTracking::Tracked);
}

#[test]
fn advance_tracked_sequence_accepts_expected_sequence() {
    let mut series = make_series();
    series.force_next_sequence_for_tests(FrameSequence(3));
    assert!(
        series
            .advance_tracked_sequence(FrameSequence(3), false)
            .is_ok()
    );
    assert_eq!(series.next_sequence, Some(FrameSequence(4)));
}

#[test]
fn advance_tracked_sequence_rejects_duplicate() {
    let mut series = make_series();
    series.force_next_sequence_for_tests(FrameSequence(5));
    let result = series.advance_tracked_sequence(FrameSequence(2), false);
    assert!(matches!(
        result,
        Err(MessageSeriesError::DuplicateFrame { .. })
    ));
}

#[test]
fn advance_tracked_sequence_rejects_out_of_order() {
    let mut series = make_series();
    series.force_next_sequence_for_tests(FrameSequence(3));
    let result = series.advance_tracked_sequence(FrameSequence(7), false);
    assert!(matches!(
        result,
        Err(MessageSeriesError::SequenceMismatch { .. })
    ));
}

#[test]
fn start_sequence_tracking_overflow_mid_stream_preserves_state() {
    let mut series = make_series();
    assert_eq!(series.sequence_tracking, SequenceTracking::Untracked);
    assert_eq!(series.next_sequence, None);
    let result = series.start_sequence_tracking(FrameSequence(u32::MAX), false);
    assert!(matches!(
        result,
        Err(MessageSeriesError::SequenceOverflow { .. })
    ));
    assert_eq!(series.sequence_tracking, SequenceTracking::Untracked);
    assert_eq!(series.next_sequence, None);
}

mod property {
    //! Property tests for sequence helper boundary conditions.

    use proptest::prelude::*;

    use super::super::*;
    use crate::message_assembler::{FirstFrameHeader, FrameSequence, MessageKey};

    fn make_series() -> MessageSeries {
        MessageSeries::from_first_frame(&FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 10,
            total_body_len: None,
            is_last: false,
        })
    }

    proptest! {
        /// Any sequence strictly below the maximum always advances without an
        /// overflow error when `is_last` is `false`.
        #[test]
        fn advance_sequence_or_overflow_no_error_below_max(
            seq in 0u16..u16::MAX,
        ) {
            let mut s = make_series();
            let incoming = FrameSequence(u32::from(seq));
            let result = s.advance_sequence_or_overflow(incoming, false);
            prop_assert!(result.is_ok());
        }

        /// `start_sequence_tracking` always switches to Tracked mode for any
        /// non-overflowing sequence value.
        #[test]
        fn start_sequence_tracking_always_enters_tracked_mode(seq in 0u16..u16::MAX) {
            let mut s = make_series();
            prop_assert_eq!(s.sequence_tracking, SequenceTracking::Untracked);
            let incoming = FrameSequence(u32::from(seq));
            let result = s.start_sequence_tracking(incoming, false);
            prop_assert!(result.is_ok());
            prop_assert_eq!(s.sequence_tracking, SequenceTracking::Tracked);
        }

        /// `advance_tracked_sequence` accepts exactly the expected sequence for
        /// any non-overflowing value.
        #[test]
        fn advance_tracked_sequence_accepts_exact_match(expected in 1u16..u16::MAX) {
            let mut s = make_series();
            let exp = FrameSequence(u32::from(expected));
            s.force_next_sequence_for_tests(exp);
            let result = s.advance_tracked_sequence(exp, false);
            prop_assert!(result.is_ok());
        }

        /// Any sequence strictly below `expected` is treated as a duplicate.
        #[test]
        fn advance_tracked_sequence_duplicate_below_expected(
            expected in 2u16..u16::MAX,
            delta in 1u16..10u16,
        ) {
            let exp = expected;
            let below = exp.saturating_sub(delta);
            prop_assume!(below < exp);
            let mut s = make_series();
            s.force_next_sequence_for_tests(FrameSequence(u32::from(exp)));
            let result = s.advance_tracked_sequence(FrameSequence(u32::from(below)), false);
            let is_duplicate = matches!(result, Err(MessageSeriesError::DuplicateFrame { .. }));
            prop_assert!(is_duplicate);
        }
    }
}
