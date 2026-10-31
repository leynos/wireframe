//! Property tests for declared total body length enforcement (issue #544).
//!
//! The example-based cases in `declared_total_tests.rs` name the protocol
//! outcomes that matter. These properties cover the open-ended space between
//! them: every validator is a pure function of `usize` values, so the boundary
//! around `usize::MAX` — where a naive sum wraps — is reachable by values
//! rather than only by hand-picked examples.
//!
//! The model-based property drives the public state machine and compares every
//! outcome against an independent reimplementation of the contract, so a
//! regression in admission, buffering, or cleanup fails with the partition
//! that provoked it.

use std::{num::NonZeroUsize, time::Duration};

use proptest::prelude::*;

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
    validation,
};

/// Values clustered at the top of the `usize` range, where addition overflows.
///
/// A uniform generator almost never lands within a few bytes of `usize::MAX`,
/// which is exactly where `accumulated + body_len` stops being representable.
fn near_max() -> impl Strategy<Value = usize> {
    (0usize..=3).prop_map(|offset| usize::MAX - offset)
}

/// A first-frame header for key `1` declaring `total`.
fn header_with_total(total: usize, body_len: usize) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len,
        total_body_len: Some(total),
        is_last: false,
    }
}

/// State with a limit large enough that the declaration is decisive.
fn subject() -> MessageAssemblyState {
    MessageAssemblyState::new(
        NonZeroUsize::new(1_024).unwrap_or(NonZeroUsize::MIN),
        Duration::from_secs(30),
    )
}

/// The outcome the contract requires for one continuation.
#[derive(Debug)]
enum Expected {
    /// The frame fits and does not complete the message.
    Buffered,
    /// The frame fits, completes the message, and dispatches the body.
    Dispatched,
    /// The frame would overrun the declaration.
    Exceeded,
    /// The frame completes the message at the wrong length.
    Mismatch,
}

/// Reimplement the contract independently of the production helpers.
fn expected_outcome(
    declared: usize,
    accumulated: usize,
    body_len: usize,
    is_last: bool,
) -> Expected {
    let Some(attempted) = accumulated.checked_add(body_len) else {
        return Expected::Exceeded;
    };
    if attempted > declared {
        return Expected::Exceeded;
    }
    if is_last {
        return if attempted == declared {
            Expected::Dispatched
        } else {
            Expected::Mismatch
        };
    }
    Expected::Buffered
}

proptest! {
    /// A first frame is accepted exactly when its declaration is satisfiable.
    ///
    /// Satisfiable means the declaration covers the frame's own body, and —
    /// when the frame also completes the message — equals that body exactly.
    /// The error variant distinguishes the two ways of failing.
    #[test]
    fn first_frame_accepts_only_satisfiable_declarations(
        body_len in 0usize..=1_000,
        total in 0usize..=1_000,
        is_last in any::<bool>(),
    ) {
        let header = FirstFrameHeader {
            is_last,
            ..header_with_total(total, body_len)
        };

        let outcome = validation::validate_first_frame(&header, body_len);

        let is_satisfiable = total >= body_len && (!is_last || total == body_len);
        prop_assert_eq!(outcome.is_ok(), is_satisfiable);

        match outcome {
            Ok(()) => {}
            Err(MessageAssemblyError::DeclaredTotalTooSmall {
                declared,
                body_len: carried,
                ..
            }) => {
                prop_assert!(total < body_len);
                prop_assert_eq!(declared, total);
                prop_assert_eq!(carried, body_len);
            }
            Err(MessageAssemblyError::DeclaredTotalMismatch { declared, actual, .. }) => {
                prop_assert!(is_last);
                prop_assert!(total >= body_len);
                prop_assert_eq!(declared, total);
                prop_assert_eq!(actual, body_len);
            }
            Err(other) => prop_assert!(false, "unexpected first-frame error: {other:?}"),
        }
    }

    /// A continuation is admitted exactly when its bytes fit the remaining room.
    ///
    /// On rejection the attempted length is the true sum, except that an
    /// unrepresentable sum is reported as `usize::MAX` rather than wrapping to
    /// a small value that would appear to fit.
    #[test]
    fn continuation_admission_matches_available_room(
        declared in 0usize..=1_000,
        accumulated in 0usize..=1_000,
        body_len in 0usize..=1_000,
    ) {
        let key = MessageKey(1);
        let outcome = validation::validate_continuation(key, declared, accumulated, body_len);

        let fits = match accumulated.checked_add(body_len) {
            Some(attempted) => attempted <= declared,
            None => false,
        };
        prop_assert_eq!(outcome.is_ok(), fits);

        if let Err(MessageAssemblyError::DeclaredTotalExceeded { attempted, .. }) = outcome {
            prop_assert_eq!(attempted, accumulated + body_len);
            prop_assert!(attempted > declared);
        }
    }

    /// Addition that `usize` cannot represent is rejected, never wrapped.
    ///
    /// This is the region a uniform generator misses and the only place where
    /// a wrapping implementation would silently accept an overrun.
    #[test]
    fn unrepresentable_sums_are_rejected_as_exceeded(
        declared in near_max(),
        accumulated in near_max(),
        body_len in near_max(),
    ) {
        let outcome =
            validation::validate_continuation(MessageKey(1), declared, accumulated, body_len);

        match accumulated.checked_add(body_len) {
            Some(attempted) => prop_assert_eq!(outcome.is_ok(), attempted <= declared),
            None => {
                prop_assert_eq!(
                    outcome,
                    Err(MessageAssemblyError::DeclaredTotalExceeded {
                        key: MessageKey(1),
                        declared,
                        attempted: usize::MAX,
                    })
                );
            }
        }
    }

    /// Completion accepts exactly the declared length.
    ///
    /// Both directions matter: a short body is the under-declared message the
    /// contract exists to stop, and a long body is an overrun that reached
    /// completion despite continuation admission.
    #[test]
    fn completion_accepts_only_an_exact_match(
        declared in any::<usize>(),
        actual in any::<usize>(),
    ) {
        prop_assert_eq!(
            validation::validate_completion(MessageKey(1), declared, actual).is_ok(),
            declared == actual,
        );
    }
}

/// Feed one continuation and report what the state machine did.
fn feed_continuation(
    state: &mut MessageAssemblyState,
    sequence: u32,
    body: &[u8],
    is_last: bool,
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let header = ContinuationFrameHeader {
        message_key: MessageKey(1),
        sequence: Some(FrameSequence(sequence)),
        body_len: body.len(),
        is_last,
    };
    state.accept_continuation_frame(&header, body)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// The state machine matches an independent model of the contract for an
    /// arbitrary partition of a message across a first frame and up to three
    /// continuations, including partitions that fall short or overshoot.
    #[test]
    fn partitioned_frames_match_the_declared_total_model(
        declared in 0usize..=64,
        first_body in 0usize..=64,
        continuations in proptest::collection::vec(0usize..=64, 0..3),
    ) {
        prop_assume!(first_body <= declared);

        let first_bytes = vec![b'a'; first_body];
        let mut state = subject();

        let header = header_with_total(declared, first_body);
        let input = FirstFrameInput::new(
            &header,
            EnvelopeRouting::default(),
            vec![],
            &first_bytes,
        )
        .expect("header lengths match the body");
        state.accept_first_frame(input).expect("satisfiable first frame");

        let mut assembled = first_bytes;
        let mut terminated = false;
        for (index, body_len) in continuations.iter().copied().enumerate() {
            let is_last = index + 1 == continuations.len();
            let body = vec![b'b'; body_len];
            let expected = expected_outcome(declared, assembled.len(), body_len, is_last);

            let result = feed_continuation(
                &mut state,
                u32::try_from(index).expect("fewer than u32::MAX frames"),
                &body,
                is_last,
            );

            match expected {
                Expected::Buffered => {
                    prop_assert_eq!(result, Ok(None));
                    assembled.extend_from_slice(&body);
                    continue;
                }
                Expected::Dispatched => {
                    assembled.extend_from_slice(&body);
                    let message = result
                        .expect("exact completion is accepted")
                        .expect("a completing frame dispatches");
                    prop_assert_eq!(message.body(), assembled.as_slice());
                }
                Expected::Exceeded => {
                    let was_exceeded =
                        matches!(result, Err(MessageAssemblyError::DeclaredTotalExceeded { .. }));
                    prop_assert!(was_exceeded, "expected DeclaredTotalExceeded, got {result:?}");
                }
                Expected::Mismatch => {
                    let was_mismatch =
                        matches!(result, Err(MessageAssemblyError::DeclaredTotalMismatch { .. }));
                    prop_assert!(was_mismatch, "expected DeclaredTotalMismatch, got {result:?}");
                }
            }

            terminated = true;
            break;
        }

        // A rejected continuation or completion frees the assembly; otherwise
        // the message is still in flight under its key.
        let expected_buffered = usize::from(!terminated);
        prop_assert_eq!(
            state.buffered_count(),
            expected_buffered,
            "the partial assembly must be freed exactly when a frame terminated it"
        );
    }
}
