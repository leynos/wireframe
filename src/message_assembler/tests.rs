//! Unit tests for message assembler header parsing.

use std::io;

use bytes::{BufMut, BytesMut};
use rstest::rstest;

use super::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};
use crate::test_helpers::TestAssembler;

#[rstest]
#[case::first_frame_without_total(
    "first frame without total",
    build_first_header_payload(FirstHeaderSpec {
        flags: 0b0,
        message_key: 9,
        metadata_len: 2,
        body_len: 12,
        total_body_len: None,
    }),
    FrameHeader::First(FirstFrameHeader {
        message_key: MessageKey(9),
        metadata_len: 2,
        body_len: 12,
        total_body_len: None,
        is_last: false,
    }),
)]
#[case::first_frame_with_total_and_last(
    "first frame with total and last",
    build_first_header_payload(FirstHeaderSpec {
        flags: 0b11,
        message_key: 42,
        metadata_len: 0,
        body_len: 8,
        total_body_len: Some(64),
    }),
    FrameHeader::First(FirstFrameHeader {
        message_key: MessageKey(42),
        metadata_len: 0,
        body_len: 8,
        total_body_len: Some(64),
        is_last: true,
    }),
)]
#[case::continuation_frame_with_sequence(
    "continuation frame with sequence",
    build_continuation_header_payload(ContinuationHeaderSpec {
        flags: 0b10,
        message_key: 7,
        body_len: 16,
        sequence: Some(3),
    }),
    FrameHeader::Continuation(ContinuationFrameHeader {
        message_key: MessageKey(7),
        sequence: Some(FrameSequence(3)),
        body_len: 16,
        is_last: false,
    }),
)]
#[case::continuation_frame_without_sequence(
    "continuation frame without sequence",
    build_continuation_header_payload(ContinuationHeaderSpec {
        flags: 0b1,
        message_key: 11,
        body_len: 5,
        sequence: None,
    }),
    FrameHeader::Continuation(ContinuationFrameHeader {
        message_key: MessageKey(11),
        sequence: None,
        body_len: 5,
        is_last: true,
    }),
)]
fn parse_frame_headers(
    #[case] case_name: &'static str,
    #[case] payload: Vec<u8>,
    #[case] expected_header: FrameHeader,
) {
    let parsed = parse_header(&payload);
    assert_eq!(parsed.header(), &expected_header, "case: {case_name}");
    assert_eq!(
        parsed.header_len(),
        payload.len(),
        "case (header-only payload): {case_name}"
    );

    let mut payload_with_body = payload.clone();
    payload_with_body.extend_from_slice(&[0xaa, 0xbb, 0xcc]);

    let parsed_with_body = parse_header(&payload_with_body);
    assert_eq!(
        parsed_with_body.header(),
        &expected_header,
        "case (with body bytes): {case_name}"
    );
    assert_eq!(
        parsed_with_body.header_len(),
        payload.len(),
        "case (with body bytes, header_len mismatch): {case_name}"
    );
    assert!(
        parsed_with_body.header_len() < payload_with_body.len(),
        "case (with body bytes, header_len not less than payload.len()): {case_name}"
    );
}

#[test]
fn short_header_errors() {
    let payload = vec![0x01];
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "header too short");
}

#[test]
fn unknown_header_kind_errors() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0xff);
    bytes.put_u8(0x00);
    bytes.put_u64(0);
    let payload = bytes.to_vec();
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unknown header kind");
}

#[derive(Clone, Copy)]
struct FirstHeaderSpec {
    flags: u8,
    message_key: u64,
    metadata_len: u16,
    body_len: u32,
    total_body_len: Option<u32>,
}

#[derive(Clone, Copy)]
struct ContinuationHeaderSpec {
    flags: u8,
    message_key: u64,
    body_len: u32,
    sequence: Option<u32>,
}

fn build_first_header_payload(spec: FirstHeaderSpec) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x01);
    bytes.put_u8(spec.flags);
    bytes.put_u64(spec.message_key);
    bytes.put_u16(spec.metadata_len);
    bytes.put_u32(spec.body_len);
    if let Some(total_body_len) = spec.total_body_len {
        bytes.put_u32(total_body_len);
    }
    bytes.to_vec()
}

fn build_continuation_header_payload(spec: ContinuationHeaderSpec) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x02);
    bytes.put_u8(spec.flags);
    bytes.put_u64(spec.message_key);
    bytes.put_u32(spec.body_len);
    if let Some(sequence) = spec.sequence {
        bytes.put_u32(sequence);
    }
    bytes.to_vec()
}

fn parse_header(payload: &[u8]) -> ParsedFrameHeader {
    TestAssembler
        .parse_frame_header(payload)
        .expect("header parse")
}

// =============================================================================
// MessageSeries tests (8.2.3/8.2.4)
// =============================================================================

mod series_tests {
    use super::*;
    use crate::message_assembler::{MessageSeries, MessageSeriesError, MessageSeriesStatus};

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
}

// =============================================================================
// MessageAssemblyState tests (8.2.3/8.2.4)
// =============================================================================

mod state_tests {
    use std::{
        num::NonZeroUsize,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::message_assembler::{
        MessageAssemblyError,
        MessageAssemblyState,
        MessageSeriesError,
    };

    fn state_with_defaults() -> MessageAssemblyState {
        MessageAssemblyState::new(
            NonZeroUsize::new(1024).expect("non-zero"),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn state_tracks_single_message_assembly() {
        let mut state = state_with_defaults();

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 2,
            body_len: 5,
            total_body_len: Some(10),
            is_last: false,
        };
        let result = state
            .accept_first_frame(&first, vec![0x01, 0x02], b"hello")
            .expect("accept first frame");
        assert!(result.is_none());
        assert_eq!(state.buffered_count(), 1);

        let cont = ContinuationFrameHeader {
            message_key: MessageKey(1),
            sequence: Some(FrameSequence(1)),
            body_len: 5,
            is_last: true,
        };
        let msg = state
            .accept_continuation_frame(&cont, b"world")
            .expect("accept continuation")
            .expect("should complete");
        assert_eq!(msg.message_key(), MessageKey(1));
        assert_eq!(msg.metadata(), &[0x01, 0x02]);
        assert_eq!(msg.body(), b"helloworld");
        assert_eq!(state.buffered_count(), 0);
    }

    #[test]
    fn state_tracks_multiple_interleaved_messages() {
        let mut state = state_with_defaults();

        // Start message 1
        let first1 = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 2,
            total_body_len: None,
            is_last: false,
        };
        state
            .accept_first_frame(&first1, vec![], b"A1")
            .expect("first frame 1");

        // Start message 2
        let first2 = FirstFrameHeader {
            message_key: MessageKey(2),
            metadata_len: 0,
            body_len: 2,
            total_body_len: None,
            is_last: false,
        };
        state
            .accept_first_frame(&first2, vec![], b"B1")
            .expect("first frame 2");

        assert_eq!(state.buffered_count(), 2);

        // Continue message 1
        let cont1 = ContinuationFrameHeader {
            message_key: MessageKey(1),
            sequence: Some(FrameSequence(1)),
            body_len: 2,
            is_last: true,
        };
        let msg1 = state
            .accept_continuation_frame(&cont1, b"A2")
            .expect("continuation 1")
            .expect("message 1 should complete");
        assert_eq!(msg1.body(), b"A1A2");
        assert_eq!(state.buffered_count(), 1);

        // Continue message 2
        let cont2 = ContinuationFrameHeader {
            message_key: MessageKey(2),
            sequence: Some(FrameSequence(1)),
            body_len: 2,
            is_last: true,
        };
        let msg2 = state
            .accept_continuation_frame(&cont2, b"B2")
            .expect("continuation 2")
            .expect("message 2 should complete");
        assert_eq!(msg2.body(), b"B1B2");
        assert_eq!(state.buffered_count(), 0);
    }

    #[test]
    fn state_rejects_continuation_without_first_frame() {
        let mut state = state_with_defaults();

        let cont = ContinuationFrameHeader {
            message_key: MessageKey(99),
            sequence: Some(FrameSequence(1)),
            body_len: 5,
            is_last: false,
        };
        let err = state
            .accept_continuation_frame(&cont, b"data")
            .expect_err("should reject");
        assert!(matches!(
            err,
            MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
                key: MessageKey(99)
            })
        ));
    }

    #[test]
    fn state_rejects_duplicate_first_frame() {
        let mut state = state_with_defaults();

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 5,
            total_body_len: None,
            is_last: false,
        };
        state
            .accept_first_frame(&first, vec![], b"hello")
            .expect("first frame");

        // Try duplicate first frame
        let err = state
            .accept_first_frame(&first, vec![], b"again")
            .expect_err("should reject duplicate");
        assert!(matches!(
            err,
            MessageAssemblyError::DuplicateFirstFrame { key: MessageKey(1) }
        ));
    }

    #[test]
    fn state_enforces_size_limit_on_first_frame() {
        let mut state = MessageAssemblyState::new(
            NonZeroUsize::new(10).expect("non-zero"), // Very small limit
            Duration::from_secs(30),
        );

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 20,
            total_body_len: None,
            is_last: false,
        };
        let err = state
            .accept_first_frame(&first, vec![], &[0u8; 20])
            .expect_err("should reject oversized");
        assert!(matches!(
            err,
            MessageAssemblyError::MessageTooLarge {
                key: MessageKey(1),
                attempted: 20,
                ..
            }
        ));
    }

    #[test]
    fn state_enforces_size_limit_on_continuation() {
        let mut state = MessageAssemblyState::new(
            NonZeroUsize::new(10).expect("non-zero"),
            Duration::from_secs(30),
        );

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 5,
            total_body_len: None,
            is_last: false,
        };
        state
            .accept_first_frame(&first, vec![], b"hello")
            .expect("first frame");

        // Continuation would push us over the limit
        let cont = ContinuationFrameHeader {
            message_key: MessageKey(1),
            sequence: Some(FrameSequence(1)),
            body_len: 10,
            is_last: true,
        };
        let err = state
            .accept_continuation_frame(&cont, &[0u8; 10])
            .expect_err("should reject oversized");
        assert!(matches!(
            err,
            MessageAssemblyError::MessageTooLarge {
                key: MessageKey(1),
                attempted: 15,
                ..
            }
        ));
        // Partial should be removed on error
        assert_eq!(state.buffered_count(), 0);
    }

    #[test]
    fn state_purges_expired_assemblies() {
        let mut state = MessageAssemblyState::new(
            NonZeroUsize::new(1024).expect("non-zero"),
            Duration::from_secs(30),
        );

        let now = Instant::now();

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 5,
            total_body_len: None,
            is_last: false,
        };
        state
            .accept_first_frame_at(&first, vec![], b"hello", now)
            .expect("accept first frame");
        assert_eq!(state.buffered_count(), 1);

        // Advance time past timeout
        let future = now + Duration::from_secs(31);
        let evicted = state.purge_expired_at(future);
        assert_eq!(evicted, vec![MessageKey(1)]);
        assert_eq!(state.buffered_count(), 0);
    }

    #[test]
    fn state_returns_single_frame_message_immediately() {
        let mut state = state_with_defaults();

        let first = FirstFrameHeader {
            message_key: MessageKey(1),
            metadata_len: 0,
            body_len: 5,
            total_body_len: None,
            is_last: true, // Single frame message
        };
        let msg = state
            .accept_first_frame(&first, vec![0xaa], b"hello")
            .expect("accept first frame")
            .expect("single frame should complete");
        assert_eq!(msg.message_key(), MessageKey(1));
        assert_eq!(msg.metadata(), &[0xaa]);
        assert_eq!(msg.body(), b"hello");
        // Should not buffer single-frame messages
        assert_eq!(state.buffered_count(), 0);
    }

    #[test]
    fn assembled_message_into_body() {
        use crate::message_assembler::AssembledMessage;

        let msg = AssembledMessage::new(MessageKey(1), vec![0x01], vec![0x02, 0x03]);
        let body = msg.into_body();
        assert_eq!(body, vec![0x02, 0x03]);
    }
}
