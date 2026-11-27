//! Inbound-adapter coverage for the metadata-inclusive limit (issue #825).
//!
//! The domain owns the size check, but the observable contract is that the
//! inbound adapter splits the first frame into metadata and body, converts a
//! rejection into `InvalidData`, counts one deserialization failure, and never
//! dispatches an over-cap message. These tests pin that end-to-end behaviour
//! against a 10-byte per-message limit.

use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use rstest::rstest;

use super::{inbound_envelope, message_assembler, ok, process_assembly_frame};
use crate::{
    app::Envelope,
    message_assembler::{FrameSequence, MessageAssemblyState, MessageKey},
    test_helpers,
};

/// Per-message limit used by the ticket's examples.
const LIMIT: usize = 10;

/// Envelope identifier every frame in these tests carries.
const ENVELOPE_ID: u32 = 9;

/// Build the connection-local state the adapter drives.
fn limited_state() -> MessageAssemblyState {
    let size = NonZeroUsize::new(LIMIT).unwrap_or(NonZeroUsize::MIN);
    MessageAssemblyState::new(size, Duration::from_mins(1))
}

/// Total bytes the assembly currently retains.
fn retained_bytes(state: Option<&MessageAssemblyState>) -> usize {
    state.map_or(usize::MAX, MessageAssemblyState::total_buffered_bytes)
}

/// Build an envelope carrying a first frame with `metadata` ahead of `body`.
fn metadata_envelope(key: u64, metadata: &[u8], body: &[u8], is_last: bool) -> Envelope {
    let payload = test_helpers::first_frame_payload_with_metadata(test_helpers::FirstFrameSpec {
        key: MessageKey::from(key),
        metadata,
        body,
        is_last,
        total: None,
    });
    inbound_envelope(
        ENVELOPE_ID,
        ok(payload, "valid metadata-bearing first frame"),
    )
}

/// Build an envelope carrying a continuation frame.
fn continuation_envelope(key: u64, sequence: u32, body: &[u8], is_last: bool) -> Envelope {
    let payload = test_helpers::continuation_frame_payload(
        MessageKey::from(key),
        FrameSequence::from(sequence),
        body,
        is_last,
    );
    inbound_envelope(ENVELOPE_ID, ok(payload, "valid continuation frame"))
}

/// Feed one envelope through the assembly adapter and report any dispatch.
fn feed(
    assembler: &Arc<dyn crate::message_assembler::MessageAssembler>,
    state: &mut Option<MessageAssemblyState>,
    failures: &mut u32,
    envelope: Envelope,
) -> Option<Envelope> {
    ok(
        process_assembly_frame(assembler, state, failures, envelope),
        "the inbound frame should be handled",
    )
}

#[rstest]
fn inbound_assembly_rejects_metadata_that_overruns_the_limit(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
) {
    let mut state = Some(limited_state());
    let mut deser_failures = 0_u32;

    // 4 metadata + 3 body fits the 10-byte limit.
    let first = metadata_envelope(1, b"META", b"abc", false);
    let first_dispatch = feed(&message_assembler, &mut state, &mut deser_failures, first);
    assert!(first_dispatch.is_none(), "first frame should not complete");

    // The next 4 body bytes would retain 11 bytes, one over the limit.
    let overrun = continuation_envelope(1, 1, b"defg", true);
    let overrun_dispatch = feed(&message_assembler, &mut state, &mut deser_failures, overrun);
    assert!(
        overrun_dispatch.is_none(),
        "an over-cap continuation must not dispatch"
    );

    assert_eq!(
        deser_failures, 1,
        "the rejection should count as one failure"
    );
    assert_eq!(
        state
            .as_ref()
            .map_or(usize::MAX, MessageAssemblyState::buffered_count),
        0,
        "the rejected assembly must be freed"
    );
    assert_eq!(
        retained_bytes(state.as_ref()),
        0,
        "no bytes remain retained"
    );
}

#[rstest]
fn inbound_assembly_dispatches_metadata_and_body_at_the_limit(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
) {
    let mut state = Some(limited_state());
    let mut deser_failures = 0_u32;

    // 4 metadata + 3 body + 3 body fills the limit exactly.
    let first = metadata_envelope(1, b"META", b"abc", false);
    let dispatched = feed(&message_assembler, &mut state, &mut deser_failures, first);
    assert!(dispatched.is_none(), "first frame should not complete");

    let completing = continuation_envelope(1, 1, b"def", true);
    let completed = feed(
        &message_assembler,
        &mut state,
        &mut deser_failures,
        completing,
    )
    .expect("the exact-fit message should complete");

    assert_eq!(
        completed.payload_bytes(),
        b"METAabcdef",
        "the dispatched payload should equal b\"METAabcdef\""
    );
    assert_eq!(
        completed.id, ENVELOPE_ID,
        "routing metadata from the first frame should be preserved"
    );
    assert_eq!(
        completed.correlation_id,
        Some(7),
        "the correlation id should come from the first frame"
    );
    assert_eq!(deser_failures, 0, "no failures expected");
    assert_eq!(
        retained_bytes(state.as_ref()),
        0,
        "no bytes remain retained"
    );
}

#[rstest]
fn inbound_assembly_recovers_after_an_over_cap_rejection(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
) {
    let mut state = Some(limited_state());
    let mut deser_failures = 0_u32;

    // 4 metadata + 3 body fits; the next 4 body bytes overrun the limit.
    let first = metadata_envelope(1, b"META", b"abc", false);
    let overrun = continuation_envelope(1, 1, b"defg", true);
    let _ = feed(&message_assembler, &mut state, &mut deser_failures, first);
    let rejected = feed(&message_assembler, &mut state, &mut deser_failures, overrun);
    assert!(rejected.is_none(), "the over-cap frame is not dispatched");

    // A fresh, well-formed message reusing the key must still dispatch.
    let replacement = metadata_envelope(1, b"TAG", b"1234", true);
    let completed = feed(
        &message_assembler,
        &mut state,
        &mut deser_failures,
        replacement,
    )
    .expect("the replacement message should complete");

    assert_eq!(
        completed.payload_bytes(),
        b"TAG1234",
        "the recovered payload should equal b\"TAG1234\""
    );
    assert_eq!(deser_failures, 1, "only the rejection is counted");
    assert_eq!(
        retained_bytes(state.as_ref()),
        0,
        "no bytes remain retained"
    );
}
