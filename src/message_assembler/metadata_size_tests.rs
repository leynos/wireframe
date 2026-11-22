//! Per-message size-cap coverage for retained metadata (issue #825).
//!
//! `MessageAssemblyState` bounds everything one logical message retains: the
//! first frame's metadata plus the body bytes accumulated across frames. These
//! tests use the reported 10-byte limit to pin both directions — metadata
//! cannot be used to smuggle an over-cap message past the limit, and a message
//! that exactly fills the limit still dispatches.
//!
//! The accompanying property tests live in `metadata_size_property_tests.rs`.

use std::{num::NonZeroUsize, time::Duration};

use rstest::{fixture, rstest};

use crate::message_assembler::{
    AssembledMessage,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::continuation_header,
};

/// The per-message limit used by every example the ticket reports.
const LIMIT: usize = 10;

/// The message key used unless a test deliberately interleaves two.
const KEY: u64 = 1;

// =============================================================================
// Helpers
// =============================================================================

/// A first frame described independently of its encoded form.
#[derive(Clone, Copy, Debug)]
struct FirstFrame<'a> {
    metadata: &'a [u8],
    body: &'a [u8],
    declared_total: Option<usize>,
    is_last: bool,
}

impl<'a> FirstFrame<'a> {
    /// A non-final first frame with no declared total.
    fn new(metadata: &'a [u8], body: &'a [u8]) -> Self {
        Self {
            metadata,
            body,
            declared_total: None,
            is_last: false,
        }
    }

    /// Declare the total body length of the logical message.
    fn declared(self, total: usize) -> Self {
        Self {
            declared_total: Some(total),
            ..self
        }
    }

    /// Mark the frame as completing its message.
    fn final_frame(self) -> Self {
        Self {
            is_last: true,
            ..self
        }
    }

    /// Build the header matching this description.
    fn header(&self) -> FirstFrameHeader {
        FirstFrameHeader {
            message_key: MessageKey(KEY),
            metadata_len: self.metadata.len(),
            body_len: self.body.len(),
            total_body_len: self.declared_total,
            is_last: self.is_last,
        }
    }
}

/// Non-zero limit wrapper so the fixtures read like the configuration.
fn limit(bytes: usize) -> NonZeroUsize {
    NonZeroUsize::new(bytes).expect("test limits are non-zero")
}

/// State whose per-message limit is the ticket's 10 bytes.
#[fixture]
fn ten_byte_state() -> MessageAssemblyState {
    MessageAssemblyState::new(limit(LIMIT), Duration::from_secs(30))
}

/// Feed a first frame under `key` and report the state-machine outcome.
fn accept_first(
    state: &mut MessageAssemblyState,
    key: u64,
    frame: FirstFrame<'_>,
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let mut header = frame.header();
    header.message_key = MessageKey(key);
    let input = FirstFrameInput::new(
        &header,
        EnvelopeRouting::default(),
        frame.metadata.to_vec(),
        frame.body,
    )
    .expect("header lengths match the payloads");
    state.accept_first_frame(input)
}

/// A continuation frame to feed under a given key.
#[derive(Clone, Copy, Debug)]
struct Continuation<'a> {
    key: u64,
    body: &'a [u8],
    is_last: bool,
}

impl<'a> Continuation<'a> {
    /// A non-final continuation for `key`.
    fn new(key: u64, body: &'a [u8]) -> Self {
        Self {
            key,
            body,
            is_last: false,
        }
    }

    /// Mark the frame as completing its message.
    fn final_frame(self) -> Self {
        Self {
            is_last: true,
            ..self
        }
    }
}

/// Feed a continuation frame and report the outcome.
///
/// Every continuation in these tests uses sequence 1, so the sequence number
/// is implied rather than repeated at each call site.
fn accept_continuation(
    state: &mut MessageAssemblyState,
    frame: Continuation<'_>,
) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
    let header = continuation_header(frame.key, 1, frame.body.len(), frame.is_last);
    state.accept_continuation_frame(&header, frame.body)
}

/// The cap rejection the ticket's 11-into-10 example must produce.
fn too_large(key: u64, attempted: usize) -> MessageAssemblyError {
    MessageAssemblyError::MessageTooLarge {
        key: MessageKey(key),
        attempted,
        limit: limit(LIMIT),
    }
}

// =============================================================================
// Continuation accounting
// =============================================================================

/// A continuation is measured against metadata plus body, not body alone.
///
/// The first frame retains 4 metadata bytes and 3 body bytes, which fits the
/// 10-byte limit. The next 4 body bytes would retain 11 bytes in total.
#[rstest]
#[case::final_continuation(true)]
#[case::non_final_continuation(false)]
fn metadata_pushes_a_continuation_past_the_limit(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
    #[case] is_last: bool,
) {
    accept_first(&mut state, KEY, FirstFrame::new(b"META", b"abc")).expect("first frame admitted");

    let err = accept_continuation(
        &mut state,
        Continuation {
            is_last,
            ..Continuation::new(KEY, b"defg")
        },
    )
    .expect_err("4 metadata plus 7 body exceeds the 10-byte limit");

    assert_eq!(
        err,
        too_large(KEY, 11),
        "err should equal the cap rejection for 11 bytes"
    );
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
    assert_eq!(
        state.total_buffered_bytes(),
        0,
        "subject.total_buffered_bytes() should equal 0"
    );
}

/// Metadata and body may together fill the limit exactly.
#[rstest]
fn metadata_and_body_may_exactly_fill_the_limit(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
) {
    accept_first(&mut state, KEY, FirstFrame::new(b"META", b"abc")).expect("first frame admitted");

    let msg = accept_continuation(&mut state, Continuation::new(KEY, b"def").final_frame())
        .expect("the exact-fit frame is admitted")
        .expect("a completing frame dispatches");

    assert_eq!(
        msg.metadata(),
        b"META",
        "msg.metadata() should equal b\"META\""
    );
    assert_eq!(msg.body(), b"abcdef", "msg.body() should equal b\"abcdef\"");
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
    assert_eq!(
        state.total_buffered_bytes(),
        0,
        "subject.total_buffered_bytes() should equal 0"
    );
}

// =============================================================================
// First-frame accounting
// =============================================================================

/// A single frame's metadata counts against the limit too.
///
/// The declaration is satisfiable in both cases — it equals the body, so
/// `validate_first_frame` accepts it — which is exactly why the cap has to
/// measure the real retained bytes rather than trusting the declaration.
#[rstest]
#[case::undeclared(None)]
#[case::declared_matching_the_body(Some(7))]
fn metadata_pushes_a_single_frame_past_the_limit(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
    #[case] declared: Option<usize>,
) {
    let frame = FirstFrame {
        declared_total: declared,
        ..FirstFrame::new(b"META", b"abcdefg").final_frame()
    };

    let err = accept_first(&mut state, KEY, frame)
        .expect_err("4 metadata plus 7 body exceeds the 10-byte limit");

    assert_eq!(
        err,
        too_large(KEY, 11),
        "err should equal the cap rejection for 11 bytes"
    );
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
}

/// A declaration larger than the body tightens the first-frame measurement.
#[rstest]
fn a_declared_total_larger_than_the_body_is_counted_against_the_limit(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
) {
    // 4 metadata plus the 3-byte body fits; the declared 8 does not.
    let frame = FirstFrame::new(b"META", b"abc").declared(8);

    let err =
        accept_first(&mut state, KEY, frame).expect_err("declaration alone exceeds the limit");

    assert_eq!(
        err,
        too_large(KEY, 12),
        "err should equal the cap rejection for 12 bytes"
    );
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
}

/// The size limit is checked before the declared total, so it wins.
#[rstest]
fn the_size_limit_wins_when_the_declaration_is_also_violated(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
) {
    // 2 metadata plus the declared 8 fills the limit exactly.
    accept_first(&mut state, KEY, FirstFrame::new(b"MM", b"abc").declared(8))
        .expect("first frame admitted");

    // Body 3 + 6 = 9 overshoots the declaration, and 2 + 3 + 6 = 11 the limit.
    let err = accept_continuation(&mut state, Continuation::new(KEY, b"defghi"))
        .expect_err("the frame is refused");

    assert_eq!(
        err,
        too_large(KEY, 11),
        "the cap rejection must be reported rather than DeclaredTotalExceeded"
    );
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
}

/// Rejecting one assembly leaves the others exactly as they were.
#[rstest]
fn a_rejection_leaves_other_assemblies_intact(
    #[from(ten_byte_state)] mut state: MessageAssemblyState,
) {
    accept_first(&mut state, 1, FirstFrame::new(b"META", b"abc")).expect("key 1 admitted");
    accept_first(&mut state, 2, FirstFrame::new(b"m", b"b")).expect("key 2 admitted");

    let err = accept_continuation(&mut state, Continuation::new(1, b"defg").final_frame())
        .expect_err("key 1 overruns");
    assert_eq!(
        err,
        too_large(1, 11),
        "key 1 should equal the cap rejection for 11 bytes"
    );

    assert_eq!(
        state.buffered_count(),
        1,
        "only the rejected assembly is freed"
    );
    assert_eq!(
        state.total_buffered_bytes(),
        2,
        "key 2 still retains its metadata and body"
    );

    let msg = accept_continuation(&mut state, Continuation::new(2, b"!").final_frame())
        .expect("key 2 is unaffected")
        .expect("key 2 completes");
    assert_eq!(msg.metadata(), b"m", "msg.metadata() should equal b\"m\"");
    assert_eq!(msg.body(), b"b!", "msg.body() should equal b\"b!\"");
}

/// An unrepresentable sum is rejected even when the limit is `usize::MAX`.
///
/// Saturating arithmetic would have collapsed this to `usize::MAX`, which is
/// not greater than a `usize::MAX` limit, so the message would have been
/// admitted with no way to measure what it retains.
#[rstest]
fn an_unrepresentable_sum_is_rejected_at_the_largest_limit() {
    let mut state = MessageAssemblyState::new(NonZeroUsize::MAX, Duration::from_secs(30));
    let frame = FirstFrame::new(b"m", b"").declared(usize::MAX);

    let err = accept_first(&mut state, KEY, frame).expect_err("the sum overflows usize");

    assert_eq!(
        err,
        MessageAssemblyError::MessageTooLarge {
            key: MessageKey(KEY),
            attempted: usize::MAX,
            limit: NonZeroUsize::MAX,
        },
        "err should equal the overflow sentinel"
    );
    assert_eq!(
        state.buffered_count(),
        0,
        "subject.buffered_count() should equal 0"
    );
}
