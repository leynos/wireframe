//! Unit tests for `MessageSeries` (8.2.3/8.2.4).

use crate::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameSequence,
    MessageKey,
    MessageSeries,
    MessageSeriesError,
    MessageSeriesStatus,
};

// Helper functions to reduce test setup duplication
fn first_header(key: u64, is_last: bool) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len: 10,
        total_body_len: None,
        is_last,
    }
}

fn continuation_header(key: u64, sequence: u32, is_last: bool) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: Some(FrameSequence(sequence)),
        body_len: 10,
        is_last,
    }
}

fn continuation_header_untracked(key: u64, is_last: bool) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: None,
        body_len: 5,
        is_last,
    }
}

#[test]
fn series_accepts_sequential_continuation_frames() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);
    assert!(!series.is_complete());

    let cont1 = continuation_header(1, 1, false);
    assert_eq!(
        series.accept_continuation(&cont1),
        Ok(MessageSeriesStatus::Incomplete)
    );

    let cont2 = continuation_header(1, 2, true);
    assert_eq!(
        series.accept_continuation(&cont2),
        Ok(MessageSeriesStatus::Complete)
    );
    assert!(series.is_complete());
}

#[test]
fn series_accepts_continuation_without_sequence() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Continuations without sequences are accepted (no ordering check)
    let cont = continuation_header_untracked(1, true);
    assert_eq!(
        series.accept_continuation(&cont),
        Ok(MessageSeriesStatus::Complete)
    );
}

#[test]
fn series_rejects_wrong_key() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    let cont = continuation_header(2, 1, false); // Wrong key
    assert!(matches!(
        series.accept_continuation(&cont),
        Err(MessageSeriesError::KeyMismatch {
            expected: MessageKey(1),
            found: MessageKey(2),
        })
    ));
}

#[test]
fn series_rejects_out_of_order_sequence() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Accept first continuation
    let cont1 = continuation_header(1, 1, false);
    series
        .accept_continuation(&cont1)
        .expect("first continuation");

    // Skip sequence 2, send 3 (gap)
    let cont3 = continuation_header(1, 3, false);
    assert!(matches!(
        series.accept_continuation(&cont3),
        Err(MessageSeriesError::SequenceMismatch {
            expected: FrameSequence(2),
            found: FrameSequence(3),
        })
    ));
}

#[test]
fn series_rejects_duplicate_sequence() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Accept first continuation
    let cont1 = continuation_header(1, 1, false);
    series
        .accept_continuation(&cont1)
        .expect("first continuation");

    // Accept second continuation
    let cont2 = continuation_header(1, 2, false);
    series
        .accept_continuation(&cont2)
        .expect("second continuation");

    // Try to send sequence 1 again (duplicate)
    let duplicate = continuation_header(1, 1, false);
    assert!(matches!(
        series.accept_continuation(&duplicate),
        Err(MessageSeriesError::DuplicateFrame {
            key: MessageKey(1),
            sequence: FrameSequence(1),
        })
    ));
}

#[test]
fn series_rejects_after_completion() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Complete the series
    let final_cont = continuation_header(1, 1, true);
    series
        .accept_continuation(&final_cont)
        .expect("final continuation");
    assert!(series.is_complete());

    // Try to add more
    let extra = continuation_header(1, 2, false);
    assert!(matches!(
        series.accept_continuation(&extra),
        Err(MessageSeriesError::SeriesComplete)
    ));
}

#[test]
fn series_detects_sequence_overflow() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Force next_sequence to u32::MAX
    series.force_next_sequence_for_tests(FrameSequence(u32::MAX));

    // Send a non-final frame at MAX
    let cont = ContinuationFrameHeader {
        message_key: MessageKey(1),
        sequence: Some(FrameSequence(u32::MAX)),
        body_len: 5,
        is_last: false,
    };
    assert!(matches!(
        series.accept_continuation(&cont),
        Err(MessageSeriesError::SequenceOverflow { .. })
    ));
}

#[test]
fn series_accepts_final_continuation_at_max_sequence() {
    let first = first_header(1, false);
    let mut series = MessageSeries::from_first_frame(&first);

    // Force next_sequence to u32::MAX
    series.force_next_sequence_for_tests(FrameSequence(u32::MAX));

    // Send a final frame at MAX - should succeed (overflow only checked for non-final)
    let cont = ContinuationFrameHeader {
        message_key: MessageKey(1),
        sequence: Some(FrameSequence(u32::MAX)),
        body_len: 5,
        is_last: true,
    };
    assert_eq!(
        series.accept_continuation(&cont),
        Ok(MessageSeriesStatus::Complete)
    );
    assert!(series.is_complete());
}

#[test]
fn series_stores_expected_total_from_first_frame() {
    let first = FirstFrameHeader {
        message_key: MessageKey(42),
        metadata_len: 0,
        body_len: 100,
        total_body_len: Some(500),
        is_last: false,
    };
    let series = MessageSeries::from_first_frame(&first);
    assert_eq!(series.message_key(), MessageKey(42));
    assert_eq!(series.expected_total(), Some(500));
}

#[test]
fn series_completes_on_single_frame_message() {
    let first = first_header(1, true); // Single frame message
    let series = MessageSeries::from_first_frame(&first);
    assert!(series.is_complete());
}
