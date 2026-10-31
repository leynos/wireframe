//! Regression tests for declared total body length enforcement (issue #544).
//!
//! A first frame may declare the total body length of its logical message.
//! That declaration is a protocol commitment: it must agree with the bytes
//! that are actually assembled, and a message that cannot honour it must be
//! rejected rather than dispatched. These tests pin every enforcement point.

use std::{num::NonZeroUsize, time::Duration};

use rstest::{fixture, rstest};

use crate::message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::{first_header, first_header_with_total},
    validation,
};

/// State with a limit large enough that declared-total checks are decisive.
#[fixture]
fn subject() -> MessageAssemblyState {
    let size = NonZeroUsize::new(1024).unwrap_or(NonZeroUsize::MIN);
    MessageAssemblyState::new(size, Duration::from_secs(30))
}

/// Build a first-frame header carrying `total` and the requested final flag.
fn first_with_total(key: u64, body_len: usize, total: usize, is_last: bool) -> FirstFrameHeader {
    FirstFrameHeader {
        is_last,
        ..first_header_with_total(key, body_len, total)
    }
}

/// Feed a first frame and return the state-machine result.
fn accept_first(
    state: &mut MessageAssemblyState,
    header: &FirstFrameHeader,
    body: &[u8],
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let input = FirstFrameInput::new(header, EnvelopeRouting::default(), vec![], body)
        .expect("header lengths match body");
    state.accept_first_frame(input)
}

/// A sequenced continuation frame to feed to the state machine.
#[derive(Clone, Copy, Debug)]
struct Continuation<'a> {
    key: u64,
    sequence: u32,
    body: &'a [u8],
    is_last: bool,
}

/// Feed a continuation frame and return the state-machine result.
fn accept_continuation(
    state: &mut MessageAssemblyState,
    frame: Continuation<'_>,
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let header = ContinuationFrameHeader {
        message_key: MessageKey(frame.key),
        sequence: Some(FrameSequence(frame.sequence)),
        body_len: frame.body.len(),
        is_last: frame.is_last,
    };
    state.accept_continuation_frame(&header, frame.body)
}

// =============================================================================
// Single-frame declarations
// =============================================================================

/// Every way a first frame can contradict its own declaration.
///
/// The three cases differ only in the declaration, the final-frame flag, and
/// the error the contradiction produces, so they share one body and one set of
/// post-conditions: rejection with the expected error and no buffered state.
#[rstest]
#[case::under_declared_final(3, true, MessageAssemblyError::DeclaredTotalTooSmall {
    key: MessageKey(1),
    declared: 3,
    body_len: 5,
})]
#[case::over_declared_final(10, true, MessageAssemblyError::DeclaredTotalMismatch {
    key: MessageKey(1),
    declared: 10,
    actual: 5,
})]
#[case::under_declared_non_final(4, false, MessageAssemblyError::DeclaredTotalTooSmall {
    key: MessageKey(1),
    declared: 4,
    body_len: 5,
})]
fn first_frame_contradicting_declaration_is_rejected(
    #[from(subject)] mut subject: MessageAssemblyState,
    #[case] total: usize,
    #[case] is_last: bool,
    #[case] expected: MessageAssemblyError,
) {
    let header = first_with_total(1, 5, total, is_last);

    let err = accept_first(&mut subject, &header, b"hello").expect_err("declaration rejected");

    assert_eq!(err, expected);
    assert_eq!(subject.buffered_count(), 0);
}

#[rstest]
fn single_frame_exact_declaration_is_dispatched(
    #[from(subject)] mut subject: MessageAssemblyState,
) {
    let header = first_with_total(1, 5, 5, true);

    let msg = accept_first(&mut subject, &header, b"hello")
        .expect("exact declaration accepted")
        .expect("single frame completes immediately");

    assert_eq!(msg.body(), b"hello");
    assert_eq!(subject.buffered_count(), 0);
}

// =============================================================================
// Multi-frame declarations
// =============================================================================

#[rstest]
fn continuation_exceeding_declared_total_is_rejected(
    #[from(subject)] mut subject: MessageAssemblyState,
) {
    let header = first_with_total(1, 5, 8, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");
    assert_eq!(subject.buffered_count(), 1);

    // 5 + 4 = 9 exceeds the declared 8.
    let frame = Continuation {
        key: 1,
        sequence: 1,
        body: b"more",
        is_last: false,
    };
    let err = accept_continuation(&mut subject, frame).expect_err("declaration exceeded");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalExceeded {
            key: MessageKey(1),
            declared: 8,
            attempted: 9,
        }
    );
    assert_eq!(
        subject.buffered_count(),
        0,
        "the assembly must be freed after overshooting its declaration"
    );
}

#[rstest]
fn final_frame_short_of_declared_total_is_rejected(
    #[from(subject)] mut subject: MessageAssemblyState,
) {
    let header = first_with_total(1, 5, 10, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    // 5 + 4 = 9, within the declaration, but the series completes here.
    let err = accept_continuation(
        &mut subject,
        Continuation {
            key: 1,
            sequence: 1,
            body: b"more",
            is_last: true,
        },
    )
    .expect_err("final frame mismatch");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalMismatch {
            key: MessageKey(1),
            declared: 10,
            actual: 9,
        }
    );
    assert_eq!(
        subject.buffered_count(),
        0,
        "the assembly must be freed after a completion mismatch"
    );
}

/// A completing frame dispatches the assembled body whether the protocol
/// declared an exact total or declared nothing at all.
///
/// The two cases share one body: the only difference is whether the
/// declaration exists for the completion check to enforce.
#[rstest]
#[case::exact_declaration(Some(10), "world", "helloworld")]
#[case::undeclared_total(None, "worlds", "helloworlds")]
fn completing_frame_dispatches_the_assembled_body(
    #[from(subject)] mut subject: MessageAssemblyState,
    #[case] declared: Option<usize>,
    #[case] final_body: &str,
    #[case] expected: &str,
) {
    let header = FirstFrameHeader {
        total_body_len: declared,
        ..first_header(1, 5, false)
    };
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    let msg = accept_continuation(
        &mut subject,
        Continuation {
            key: 1,
            sequence: 1,
            body: final_body.as_bytes(),
            is_last: true,
        },
    )
    .expect("body honours the declaration")
    .expect("message completes");

    assert_eq!(msg.body(), expected.as_bytes());
    assert_eq!(subject.buffered_count(), 0);
}

#[rstest]
fn non_final_frame_may_exactly_reach_the_declaration(
    #[from(subject)] mut subject: MessageAssemblyState,
) {
    let header = first_with_total(1, 5, 10, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    // Exactly reaches the declaration without completing.
    let msg = accept_continuation(
        &mut subject,
        Continuation {
            key: 1,
            sequence: 1,
            body: b"world",
            is_last: false,
        },
    )
    .expect("equals the declaration, so accepted");
    assert!(msg.is_none());
    assert_eq!(subject.buffered_count(), 1);

    // No bytes remain, so the completing frame must be rejected.
    let err = accept_continuation(
        &mut subject,
        Continuation {
            key: 1,
            sequence: 2,
            body: b"!",
            is_last: true,
        },
    )
    .expect_err("no room remains within the declaration");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalExceeded {
            key: MessageKey(1),
            declared: 10,
            attempted: 11,
        }
    );
    assert_eq!(subject.buffered_count(), 0);
}

#[rstest]
fn declared_total_equal_to_first_frame_body_leaves_no_room(
    #[from(subject)] mut subject: MessageAssemblyState,
) {
    // The boundary case: declared == body_len with more frames expected.
    let header = first_with_total(1, 5, 5, false);
    accept_first(&mut subject, &header, b"hello").expect("boundary declaration accepted");

    let err = accept_continuation(
        &mut subject,
        Continuation {
            key: 1,
            sequence: 1,
            body: b"!",
            is_last: true,
        },
    )
    .expect_err("no bytes remain within the declaration");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalExceeded {
            key: MessageKey(1),
            declared: 5,
            attempted: 6,
        }
    );
    assert_eq!(subject.buffered_count(), 0);
}

// =============================================================================
// Pure helper boundaries
// =============================================================================

/// The accumulator helper must report overflow as an exceeded declaration
/// rather than wrapping to a small value that appears to fit.
#[test]
fn continuation_accumulation_overflow_is_reported_as_exceeded() {
    let err = validation::validate_continuation(MessageKey(1), usize::MAX, usize::MAX, 1)
        .expect_err("sum overflows usize");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalExceeded {
            key: MessageKey(1),
            declared: usize::MAX,
            attempted: usize::MAX,
        }
    );
}
