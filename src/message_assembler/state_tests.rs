//! Unit tests for `MessageAssemblyState` (8.2.3/8.2.4).

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

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
    let input = FirstFrameInput::new(&first, vec![0x01, 0x02], b"hello").expect("valid input");
    let result = state.accept_first_frame(input).expect("accept first frame");
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
        .accept_first_frame(FirstFrameInput::new(&first1, vec![], b"A1").expect("valid input"))
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
        .accept_first_frame(FirstFrameInput::new(&first2, vec![], b"B1").expect("valid input"))
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
        body_len: 4,
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
        .accept_first_frame(FirstFrameInput::new(&first, vec![], &[0u8; 20]).expect("valid input"))
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
        .accept_first_frame(FirstFrameInput::new(&first, vec![], b"hello").expect("valid input"))
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

#[test]
fn state_returns_single_frame_message_immediately() {
    let mut state = state_with_defaults();

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
