//! Declared-total violations for streaming message assembly.
//!
//! A streaming first frame may declare its total body length. Both ways a
//! continuation can break that declaration — overshooting it, or completing the
//! body short of it — are terminal, so the partial is removed and the handler
//! sees an error rather than a truncated body.

use std::time::Instant;

use proptest::prelude::*;
use rstest::rstest;

use super::{admit_first, continuation_header, first_header, state_with_limit};
use crate::message_assembler::{
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    StreamingFailure,
};

/// Which declared-total contract a parameterized case violates.
#[derive(Clone, Copy, Debug)]
enum DeclaredTotal {
    /// The frame would carry the body past its declared total.
    Exceeded,
    /// The completing frame leaves the body short of its declared total.
    Mismatch,
}

impl DeclaredTotal {
    /// Return whether `error` is this violation.
    fn matches(self, error: &MessageAssemblyError) -> bool {
        match self {
            Self::Exceeded => matches!(error, MessageAssemblyError::DeclaredTotalExceeded { .. }),
            Self::Mismatch => matches!(error, MessageAssemblyError::DeclaredTotalMismatch { .. }),
        }
    }
}

/// One declared-total violation: a first frame followed by a bad continuation.
struct DeclaredTotalCase {
    /// Message key shared by both frames.
    key: u64,
    /// Total body length the first frame declares.
    declared_total: usize,
    /// Sequence, body length, and finality of the continuation.
    continuation: (u32, usize, bool),
    /// Continuation body bytes.
    body: &'static [u8],
    /// Violation the continuation is expected to raise.
    expected: DeclaredTotal,
}

/// A continuation that overshoots the declared total.
const DECLARED_TOTAL_OVERSHOOT: DeclaredTotalCase = DeclaredTotalCase {
    key: 3,
    declared_total: 6,
    continuation: (1, 4, false),
    body: b"bbbb",
    expected: DeclaredTotal::Exceeded,
};

/// A completing continuation that leaves the body short of its total.
const DECLARED_TOTAL_SHORT_FINAL: DeclaredTotalCase = DeclaredTotalCase {
    key: 4,
    declared_total: 8,
    continuation: (1, 2, true),
    body: b"bb",
    expected: DeclaredTotal::Mismatch,
};

/// Admit a first frame declaring four bytes, then deliver a continuation that
/// cannot satisfy the declaration.
///
/// The state and the terminal failure are returned so a case can pin both the
/// error variant and the removal of the partial.
fn reject_declared_total_continuation(
    case: &DeclaredTotalCase,
) -> (MessageAssemblyState, StreamingFailure) {
    let mut state = state_with_limit(1024);
    let start = Instant::now();
    admit_first(
        &mut state,
        &first_header(case.key, 4, Some(case.declared_total), false),
        b"aaaa",
        start,
    );

    let (sequence, body_len, is_last) = case.continuation;
    let header = continuation_header(case.key, sequence, body_len, is_last);
    let failure = state
        .accept_streaming_continuation_at(&header, case.body, start)
        .expect_err("the continuation cannot satisfy the declared total");
    (state, failure)
}

/// A continuation that cannot satisfy the declared total is terminal.
#[rstest]
#[case::overshoot(&DECLARED_TOTAL_OVERSHOOT)]
#[case::short_final(&DECLARED_TOTAL_SHORT_FINAL)]
fn a_continuation_that_breaks_the_declared_total_is_terminal(#[case] case: &DeclaredTotalCase) {
    let (state, failure) = reject_declared_total_continuation(case);

    assert!(
        case.expected.matches(failure.source_error()),
        "expected {:?}, got {:?}",
        case.expected,
        failure.source_error()
    );
    assert!(
        failure.is_terminal(),
        "a rejected declaration closes the message"
    );
    assert_eq!(failure.key(), MessageKey(case.key));
    assert!(
        !state.is_streaming_key(MessageKey(case.key)),
        "the partial is removed"
    );
}

proptest! {
    /// A continuation's declared-total outcome follows the contract.
    ///
    /// The generated sizes cover both terminal violations and both successes.
    /// A terminal violation must remove the partial so the handler sees an
    /// error, and a completing continuation must remove it too, because the
    /// message finished rather than stalled.
    #[test]
    fn continuation_outcomes_follow_the_declared_total(
        declared in 4_usize..=64,
        continuation_len in 0_usize..=64,
        is_last in any::<bool>(),
    ) {
        /// Body bytes the first frame carries, and what it therefore declares.
        const FIRST: usize = 4;
        let key = MessageKey(1);
        let mut state = state_with_limit(1024);
        let start = Instant::now();
        admit_first(
            &mut state,
            &first_header(1, FIRST, Some(declared), false),
            b"aaaa",
            start,
        );

        let header = continuation_header(1, 1, continuation_len, is_last);
        let body = vec![b'b'; continuation_len];
        let outcome = state.accept_streaming_continuation_at(&header, &body, start);
        let attempted = FIRST + continuation_len;

        match outcome {
            Ok(_) => {
                prop_assert!(attempted <= declared, "an overrun must not be admitted");
                prop_assert!(
                    !is_last || attempted == declared,
                    "a completing frame must match the declaration exactly"
                );
                prop_assert_eq!(
                    state.is_streaming_key(key),
                    !is_last,
                    "only a completed message leaves no partial behind"
                );
            }
            Err(failure) => {
                prop_assert!(
                    attempted > declared || (is_last && attempted != declared),
                    "a continuation that honours the declaration is not terminal"
                );
                let source = failure.source_error();
                match source {
                    MessageAssemblyError::DeclaredTotalExceeded { .. } => {
                        prop_assert!(attempted > declared, "exceeded means the total was overrun");
                    }
                    MessageAssemblyError::DeclaredTotalMismatch { .. } => {
                        prop_assert!(
                            is_last && attempted != declared,
                            "a mismatch is only possible when the frame completes the body"
                        );
                    }
                    other => prop_assert!(false, "unexpected failure: {other:?}"),
                }
                prop_assert!(failure.is_terminal(), "a broken declaration closes the message");
                prop_assert!(
                    !state.is_streaming_key(key),
                    "a terminal violation removes the partial"
                );
            }
        }
    }
}
