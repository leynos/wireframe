//! Unit tests for `MessageAssemblyState` (8.2.3/8.2.4).

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rstest::{fixture, rstest};

use crate::message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    MessageSeriesError,
};

/// Creates a `MessageAssemblyState` with sensible defaults for testing.
#[fixture]
fn state_with_defaults() -> MessageAssemblyState {
    MessageAssemblyState::new(
        NonZeroUsize::new(1024).expect("non-zero"),
        Duration::from_secs(30),
    )
}

// =============================================================================
// Header factory helpers to reduce duplication
// =============================================================================

/// Creates a `FirstFrameHeader` with sensible defaults.
fn first_header(key: u64, body_len: usize, is_last: bool) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len,
        total_body_len: None,
        is_last,
    }
}

/// Creates a `FirstFrameHeader` with a declared total body length for early validation.
fn first_header_with_total(key: u64, body_len: usize, total: usize) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len,
        total_body_len: Some(total),
        is_last: false,
    }
}

/// Creates a `ContinuationFrameHeader` with a sequence number.
fn continuation_header(
    key: u64,
    seq: u32,
    body_len: usize,
    is_last: bool,
) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: Some(FrameSequence(seq)),
        body_len,
        is_last,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[rstest]
fn state_tracks_single_message_assembly(
    #[from(state_with_defaults)] mut state: MessageAssemblyState,
) {
    // Use a header with metadata to test metadata handling
    let first = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 2,
        body_len: 5,
        total_body_len: Some(10),
        is_last: false,
    };
    let input = FirstFrameInput::new(&first, vec![0x01, 0x02], b"hello").expect("valid input");
    let result = state.accept_first_frame(input).expect("accept first frame");
    assert!(result.is_none());
    assert_eq!(state.buffered_count(), 1);

    let cont = continuation_header(1, 1, 5, true);
    let msg = state
        .accept_continuation_frame(&cont, b"world")
        .expect("accept continuation")
        .expect("should complete");
    assert_eq!(msg.message_key(), MessageKey(1));
    assert_eq!(msg.metadata(), &[0x01, 0x02]);
    assert_eq!(msg.body(), b"helloworld");
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
fn state_tracks_multiple_interleaved_messages(
    #[from(state_with_defaults)] mut state: MessageAssemblyState,
) {
    // Start message 1
    let first1 = first_header(1, 2, false);
    state
        .accept_first_frame(FirstFrameInput::new(&first1, vec![], b"A1").expect("valid input"))
        .expect("first frame 1");

    // Start message 2
    let first2 = first_header(2, 2, false);
    state
        .accept_first_frame(FirstFrameInput::new(&first2, vec![], b"B1").expect("valid input"))
        .expect("first frame 2");

    assert_eq!(state.buffered_count(), 2);

    // Continue message 1
    let cont1 = continuation_header(1, 1, 2, true);
    let msg1 = state
        .accept_continuation_frame(&cont1, b"A2")
        .expect("continuation 1")
        .expect("message 1 should complete");
    assert_eq!(msg1.body(), b"A1A2");
    assert_eq!(state.buffered_count(), 1);

    // Continue message 2
    let cont2 = continuation_header(2, 1, 2, true);
    let msg2 = state
        .accept_continuation_frame(&cont2, b"B2")
        .expect("continuation 2")
        .expect("message 2 should complete");
    assert_eq!(msg2.body(), b"B1B2");
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
fn state_rejects_continuation_without_first_frame(
    #[from(state_with_defaults)] mut state: MessageAssemblyState,
) {
    let cont = continuation_header(99, 1, 4, false);
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

#[rstest]
fn state_rejects_duplicate_first_frame(
    #[from(state_with_defaults)] mut state: MessageAssemblyState,
) {
    let first = first_header(1, 5, false);
    state
        .accept_first_frame(FirstFrameInput::new(&first, vec![], b"hello").expect("valid input"))
        .expect("first frame");

    // Try duplicate first frame
    let err = state
        .accept_first_frame(FirstFrameInput::new(&first, vec![], b"again").expect("valid input"))
        .expect_err("should reject duplicate");
    assert!(matches!(
        err,
        MessageAssemblyError::DuplicateFirstFrame { key: MessageKey(1) }
    ));
}

/// Parameters for size limit test cases.
struct SizeLimitCase {
    first_body_len: usize,
    continuation_body_len: Option<usize>,
    expected_attempted: usize,
}

#[rstest]
#[case::first_frame_exceeds_limit(SizeLimitCase {
    first_body_len: 20,
    continuation_body_len: None,
    expected_attempted: 20,
})]
#[case::continuation_exceeds_limit(SizeLimitCase {
    first_body_len: 5,
    continuation_body_len: Some(10),
    expected_attempted: 15,
})]
fn state_enforces_size_limit(#[case] params: SizeLimitCase) {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(10).expect("non-zero"),
        Duration::from_secs(30),
    );

    let first_body = vec![0u8; params.first_body_len];
    let first = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: params.first_body_len,
        total_body_len: None,
        is_last: params.continuation_body_len.is_none(),
    };
    let input = FirstFrameInput::new(&first, vec![], &first_body).expect("valid input");

    match params.continuation_body_len {
        None => {
            // Rejection on first frame
            let err = state
                .accept_first_frame(input)
                .expect_err("should reject oversized first frame");
            assert!(matches!(
                err,
                MessageAssemblyError::MessageTooLarge {
                    key: MessageKey(1),
                    attempted,
                    ..
                } if attempted == params.expected_attempted
            ));
        }
        Some(cont_len) => {
            // First frame succeeds, continuation rejected
            state.accept_first_frame(input).expect("first frame");

            let cont = ContinuationFrameHeader {
                message_key: MessageKey(1),
                sequence: Some(FrameSequence(1)),
                body_len: cont_len,
                is_last: true,
            };
            let cont_body = vec![0u8; cont_len];
            let err = state
                .accept_continuation_frame(&cont, &cont_body)
                .expect_err("should reject oversized continuation");
            assert!(matches!(
                err,
                MessageAssemblyError::MessageTooLarge {
                    key: MessageKey(1),
                    attempted,
                    ..
                } if attempted == params.expected_attempted
            ));
        }
    }

    assert_eq!(state.buffered_count(), 0);
}

#[test]
fn state_enforces_size_limit_on_first_frame() {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(10).expect("non-zero"),
        Duration::from_secs(30),
    );

    // First frame body is within limit but declared total exceeds it
    let first = first_header_with_total(1, 5, 20);
    let input = FirstFrameInput::new(&first, vec![], b"hello").expect("valid input");

    let err = state
        .accept_first_frame(input)
        .expect_err("should reject based on declared total");
    assert!(matches!(
        err,
        MessageAssemblyError::MessageTooLarge {
            key: MessageKey(1),
            attempted: 20,
            ..
        }
    ));
    assert_eq!(state.buffered_count(), 0);
}

#[test]
fn state_purges_expired_assemblies() {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(1024).expect("non-zero"),
        Duration::from_secs(30),
    );

    let now = Instant::now();

    let first = first_header(1, 5, false);
    state
        .accept_first_frame_at(
            FirstFrameInput::new(&first, vec![], b"hello").expect("valid input"),
            now,
        )
        .expect("accept first frame");
    assert_eq!(state.buffered_count(), 1);

    // Advance time past timeout
    let future = now + Duration::from_secs(31);
    let evicted = state.purge_expired_at(future);
    assert_eq!(evicted, vec![MessageKey(1)]);
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
fn state_returns_single_frame_message_immediately(
    #[from(state_with_defaults)] mut state: MessageAssemblyState,
) {
    // Single frame message with metadata requires explicit header construction
    let first = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 1,
        body_len: 5,
        total_body_len: None,
        is_last: true, // Single frame message
    };
    let msg = state
        .accept_first_frame(
            FirstFrameInput::new(&first, vec![0xaa], b"hello").expect("valid input"),
        )
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
    let msg = AssembledMessage::new(MessageKey(1), vec![0x01], vec![0x02, 0x03]);
    let body = msg.into_body();
    assert_eq!(body, vec![0x02, 0x03]);
}
