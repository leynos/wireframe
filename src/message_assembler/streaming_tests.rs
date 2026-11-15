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
};

/// Assembly timeout used by these tests.
const TIMEOUT: Duration = Duration::from_mins(1);

/// Build an assembly state with a per-message cap of `limit` bytes.
fn state_with_limit(limit: usize) -> MessageAssemblyState {
    let max = NonZeroUsize::new(limit).expect("test limit must be non-zero");
    MessageAssemblyState::new(max, TIMEOUT)
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

#[test]
fn declared_total_overshoot_is_rejected_before_delivery() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(3);
    let start = Instant::now();
    admit_first(
        &mut state,
        &first_header(3, 4, Some(6), false),
        b"aaaa",
        start,
    );

    let header = continuation_header(3, 1, 4, false);
    let failure = state
        .accept_streaming_continuation_at(&header, b"bbbb", start)
        .expect_err("4 more bytes would overshoot the declared six");

    assert!(
        matches!(
            failure.source_error(),
            MessageAssemblyError::DeclaredTotalExceeded { .. }
        ),
        "unexpected error: {:?}",
        failure.source_error()
    );
    assert!(failure.is_terminal(), "an overshoot closes the message");
    assert_eq!(failure.key(), key);
    assert!(!state.is_streaming_key(key), "the partial is removed");
}

#[test]
fn declared_total_mismatch_at_completion_is_rejected() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(4);
    let start = Instant::now();
    admit_first(
        &mut state,
        &first_header(4, 4, Some(8), false),
        b"aaaa",
        start,
    );

    let header = continuation_header(4, 1, 2, true);
    let failure = state
        .accept_streaming_continuation_at(&header, b"bb", start)
        .expect_err("a short final frame must not complete the message");

    assert!(
        matches!(
            failure.source_error(),
            MessageAssemblyError::DeclaredTotalMismatch { .. }
        ),
        "unexpected error: {:?}",
        failure.source_error()
    );
    assert!(
        failure.is_terminal(),
        "a short final frame closes the message"
    );
    assert!(!state.is_streaming_key(key), "the partial is removed");
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
