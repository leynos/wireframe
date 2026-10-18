//! Regression tests for declared total body length enforcement (issue #544).
//!
//! A first frame may declare the total body length of its logical message.
//! That declaration is a protocol commitment: it must agree with the bytes
//! that are actually assembled, and a message that cannot honour it must be
//! rejected rather than dispatched. These tests pin every enforcement point.

use std::{num::NonZeroUsize, time::Duration};

use rstest::fixture;

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

/// Feed a sequenced continuation frame and return the state-machine result.
fn accept_continuation(
    state: &mut MessageAssemblyState,
    key: u64,
    sequence: u32,
    body: &[u8],
    is_last: bool,
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let header = ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: Some(FrameSequence(sequence)),
        body_len: body.len(),
        is_last,
    };
    state.accept_continuation_frame(&header, body)
}

// =============================================================================
// Single-frame declarations
// =============================================================================

#[test]
fn single_frame_under_declared_total_is_rejected() {
    let mut subject = subject();
    // Declares 3 bytes but carries 5: the declaration can never be satisfied.
    let header = first_with_total(1, 5, 3, true);

    let err = accept_first(&mut subject, &header, b"hello").expect_err("under-declared total");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalTooSmall {
            key: MessageKey(1),
            declared: 3,
            body_len: 5,
        }
    );
    assert_eq!(subject.buffered_count(), 0);
}

#[test]
fn single_frame_over_declared_total_is_rejected() {
    let mut subject = subject();
    // Declares 10 bytes but is the final frame with only 5.
    let header = first_with_total(1, 5, 10, true);

    let err = accept_first(&mut subject, &header, b"hello").expect_err("over-declared total");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalMismatch {
            key: MessageKey(1),
            declared: 10,
            actual: 5,
        }
    );
    assert_eq!(subject.buffered_count(), 0);
}

#[test]
fn single_frame_exact_declaration_is_dispatched() {
    let mut subject = subject();
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

#[test]
fn first_frame_under_declared_multi_frame_total_is_rejected() {
    let mut subject = subject();
    // Even a non-final first frame may not carry more than its own total.
    let header = first_with_total(1, 5, 4, false);

    let err = accept_first(&mut subject, &header, b"hello").expect_err("under-declared total");

    assert_eq!(
        err,
        MessageAssemblyError::DeclaredTotalTooSmall {
            key: MessageKey(1),
            declared: 4,
            body_len: 5,
        }
    );
    assert_eq!(subject.buffered_count(), 0);
}

#[test]
fn continuation_exceeding_declared_total_is_rejected() {
    let mut subject = subject();
    let header = first_with_total(1, 5, 8, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");
    assert_eq!(subject.buffered_count(), 1);

    // 5 + 4 = 9 exceeds the declared 8.
    let err =
        accept_continuation(&mut subject, 1, 1, b"more", false).expect_err("declaration exceeded");

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

#[test]
fn final_frame_short_of_declared_total_is_rejected() {
    let mut subject = subject();
    let header = first_with_total(1, 5, 10, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    // 5 + 4 = 9, within the declaration, but the series completes here.
    let err =
        accept_continuation(&mut subject, 1, 1, b"more", true).expect_err("final frame mismatch");

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

#[test]
fn multi_frame_exact_declaration_is_dispatched() {
    let mut subject = subject();
    let header = first_with_total(1, 5, 10, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    let msg = accept_continuation(&mut subject, 1, 1, b"world", true)
        .expect("declaration honoured")
        .expect("message completes");

    assert_eq!(msg.body(), b"helloworld");
    assert_eq!(subject.buffered_count(), 0);
}

#[test]
fn non_final_frame_may_exactly_reach_the_declaration() {
    let mut subject = subject();
    let header = first_with_total(1, 5, 10, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    // Exactly reaches the declaration without completing.
    let msg = accept_continuation(&mut subject, 1, 1, b"world", false)
        .expect("equals the declaration, so accepted");
    assert!(msg.is_none());
    assert_eq!(subject.buffered_count(), 1);

    // No bytes remain, so the completing frame must be rejected.
    let err = accept_continuation(&mut subject, 1, 2, b"!", true)
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

// =============================================================================
// Protocols without a declaration
// =============================================================================

#[test]
fn undeclared_total_is_unaffected() {
    let mut subject = subject();
    let header = first_header(1, 5, false);
    accept_first(&mut subject, &header, b"hello").expect("first frame accepted");

    let msg = accept_continuation(&mut subject, 1, 1, b"worlds", true)
        .expect("undeclared total imposes no equality check")
        .expect("message completes");

    assert_eq!(msg.body(), b"helloworlds");
}

#[test]
fn declared_total_equal_to_first_frame_body_leaves_no_room() {
    let mut subject = subject();
    // The boundary case: declared == body_len with more frames expected.
    let header = first_with_total(1, 5, 5, false);
    accept_first(&mut subject, &header, b"hello").expect("boundary declaration accepted");

    let err = accept_continuation(&mut subject, 1, 1, b"!", true)
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
