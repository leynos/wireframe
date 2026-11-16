//! Streaming message assembly tests.
//!
//! A streaming route consumes body bytes while frames are still arriving, so
//! these tests pin two properties: delivered bytes are never retained in the
//! assembly budgets, and the per-message cap and declared-total contract are
//! enforced before a chunk reaches the handler.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rstest::rstest;

use super::{
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    StreamingContinuation,
    StreamingFailure,
};

/// Assembly timeout used by these tests.
const TIMEOUT: Duration = Duration::from_mins(1);

/// Build an assembly state with a per-message cap of `limit` bytes.
fn state_with_limit(limit: usize) -> MessageAssemblyState {
    let max = NonZeroUsize::new(limit).expect("test limit must be non-zero");
    MessageAssemblyState::new(max, TIMEOUT)
}

/// Which declared-total contract a parameterised case violates.
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

/// Build a first-frame header carrying no metadata.
fn first_header(
    key: u64,
    body_len: usize,
    total_body_len: Option<usize>,
    is_last: bool,
) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len,
        total_body_len,
        is_last,
    }
}

/// Build a sequenced continuation header.
fn continuation_header(
    key: u64,
    sequence: u32,
    body_len: usize,
    is_last: bool,
) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: Some(FrameSequence(sequence)),
        body_len,
        is_last,
    }
}

/// Admit a first frame with an empty metadata block.
fn admit_first(
    state: &mut MessageAssemblyState,
    header: &FirstFrameHeader,
    body: &[u8],
    now: Instant,
) {
    let input = FirstFrameInput::new(header, EnvelopeRouting::default(), Vec::new(), body)
        .expect("header lengths must match the body");
    state
        .accept_streaming_first_frame_at(input, now)
        .expect("first frame should be admitted");
}

#[test]
fn streaming_first_frame_retains_no_buffered_bytes() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(1);
    let header = first_header(1, 4, Some(8), false);
    let body = b"abcd";

    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), Vec::new(), body)
        .expect("header lengths must match the body");
    let first = state
        .accept_streaming_first_frame_at(input, Instant::now())
        .expect("first frame should be admitted");

    assert_eq!(first.key(), key);
    assert_eq!(first.chunk(), b"abcd");
    assert!(!first.is_last(), "the series continues");
    assert_eq!(
        state.total_buffered_bytes(),
        0,
        "delivered bytes must not enter the assembly budgets"
    );
    assert_eq!(state.streaming_count(), 1);
    assert!(state.is_streaming_key(key));
}

#[test]
fn single_frame_streaming_message_leaves_no_partial() {
    let mut state = state_with_limit(1024);
    let header = first_header(2, 4, Some(4), true);
    let body = b"wxyz";

    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), Vec::new(), body)
        .expect("header lengths must match the body");
    let first = state
        .accept_streaming_first_frame_at(input, Instant::now())
        .expect("single-frame message should be admitted");

    assert!(first.is_last(), "the one frame completes the message");
    assert_eq!(first.chunk(), b"wxyz");
    assert_eq!(state.streaming_count(), 0, "no partial should be retained");
    assert_eq!(state.total_buffered_bytes(), 0);
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

#[test]
fn continuation_exceeding_the_per_message_cap_is_terminal() {
    let mut state = state_with_limit(8);
    let key = MessageKey(5);
    let start = Instant::now();
    admit_first(&mut state, &first_header(5, 4, None, false), b"aaaa", start);

    let second = continuation_header(5, 1, 4, false);
    assert!(
        state
            .accept_streaming_continuation_at(&second, b"bbbb", start)
            .is_ok(),
        "eight bytes exactly fit the cap"
    );

    let third = continuation_header(5, 2, 4, true);
    let failure = state
        .accept_streaming_continuation_at(&third, b"cccc", start)
        .expect_err("twelve accumulated bytes exceed the eight-byte cap");

    assert!(
        matches!(
            failure.source_error(),
            MessageAssemblyError::MessageTooLarge { .. }
        ),
        "unexpected error: {:?}",
        failure.source_error()
    );
    assert!(failure.is_terminal());
    assert!(!state.is_streaming_key(key), "the partial is removed");
}

#[test]
fn sequence_gap_keeps_the_assembly_for_recovery() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(6);
    let start = Instant::now();
    admit_first(&mut state, &first_header(6, 4, None, false), b"aaaa", start);

    // The first sequenced continuation starts sequence tracking; a later gap
    // must not advance it.
    let first = continuation_header(6, 1, 4, false);
    let chunk = state
        .accept_streaming_continuation_at(&first, b"bbbb", start)
        .expect("the first sequence should be accepted");
    assert!(chunk.is_chunk(), "the chunk is deliverable");
    assert_eq!(chunk.key(), key);

    let gap = continuation_header(6, 3, 4, false);
    let failure = state
        .accept_streaming_continuation_at(&gap, b"dddd", start)
        .expect_err("a gap must be reported");

    assert!(
        !failure.is_terminal(),
        "a retransmission can still recover the series"
    );
    assert!(state.is_streaming_key(key), "the partial is retained");

    let recovered = continuation_header(6, 2, 2, true);
    let completion = state
        .accept_streaming_continuation_at(&recovered, b"cc", start)
        .expect("the retransmitted sequence should complete the message");
    assert!(
        matches!(
            completion,
            StreamingContinuation::Chunk { is_last: true, .. }
        ),
        "the final frame must be delivered as the last chunk"
    );
    assert_eq!(state.streaming_count(), 0, "no partial remains");
}

#[test]
fn discard_mode_validates_without_delivering() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(7);
    let start = Instant::now();
    admit_first(&mut state, &first_header(7, 4, None, false), b"aaaa", start);

    assert!(
        state.discard_streaming(key),
        "the live key should switch to discard mode"
    );

    let last = continuation_header(7, 1, 4, true);
    let outcome = state
        .accept_streaming_continuation_at(&last, b"bbbb", start)
        .expect("a discarded chunk is still validated");
    assert!(
        matches!(outcome, StreamingContinuation::Discarded { key: k } if k == key),
        "the chunk must be discarded, not delivered"
    );
    assert_eq!(state.streaming_count(), 0, "completion removes the partial");
}

#[test]
fn expiry_reports_streaming_keys_separately() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(8);
    let start = Instant::now();
    admit_first(&mut state, &first_header(8, 4, None, false), b"aaaa", start);

    let early = state.purge_expired_detailed_at(start + Duration::from_secs(30));
    assert!(
        early.streaming().is_empty(),
        "the assembly has not expired yet"
    );
    assert!(state.is_streaming_key(key));

    let late = state.purge_expired_detailed_at(start + Duration::from_secs(61));
    assert_eq!(late.streaming(), [key], "the expiry is reported per key");
    assert!(
        late.buffered().is_empty(),
        "no buffered assembly was evicted"
    );
    assert_eq!(state.streaming_count(), 0);
}

#[test]
fn admission_defers_an_evicted_streaming_key_to_the_next_purge() {
    let mut state = state_with_limit(1024);
    let start = Instant::now();
    let expired = first_header(1, 4, Some(8), false);
    admit_first(&mut state, &expired, b"abcd", start);

    // Admitting an unrelated message after the timeout purges the stalled one.
    let later = start + TIMEOUT + Duration::from_secs(1);
    let unrelated = first_header(2, 2, Some(4), false);
    admit_first(&mut state, &unrelated, b"xy", later);
    assert!(
        !state.is_streaming_key(MessageKey(1)),
        "the stalled assembly should have been evicted during admission"
    );

    let reported = state.purge_expired_detailed_at(later);
    assert_eq!(
        reported.streaming(),
        [MessageKey(1)],
        "the body consumer must still be told its assembly expired"
    );

    let repeated = state.purge_expired_detailed_at(later);
    assert!(
        repeated.streaming().is_empty(),
        "a deferred eviction should be reported once"
    );
}

#[test]
fn continuation_size_checks_include_the_metadata_block() {
    // The first frame already fills the cap once its metadata is counted.
    let mut state = state_with_limit(8);
    let start = Instant::now();
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 4,
        body_len: 4,
        total_body_len: None,
        is_last: false,
    };
    let input = FirstFrameInput::new(
        &header,
        EnvelopeRouting::default(),
        vec![0x01, 0x02, 0x03, 0x04],
        b"abcd",
    )
    .expect("header lengths must match the payload");
    state
        .accept_streaming_first_frame_at(input, start)
        .expect("the first frame fits the cap exactly");

    // Four more body bytes would make the logical message twelve bytes long.
    let continuation = continuation_header(1, 1, 4, true);
    let failure = state
        .accept_streaming_continuation_at(&continuation, b"wxyz", start)
        .expect_err("metadata must count against the per-message cap");

    assert!(
        matches!(
            failure.source_error(),
            MessageAssemblyError::MessageTooLarge { .. }
        ),
        "the continuation should be rejected for size, got {:?}",
        failure.source_error()
    );
}
