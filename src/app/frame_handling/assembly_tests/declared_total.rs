//! Inbound-adapter coverage for declared-total enforcement (issue #544).
//!
//! The domain owns the declared-total checks, but the observable contract is
//! that the inbound adapter converts a rejection into `InvalidData`, counts a
//! deserialization failure, and never dispatches the message. These tests pin
//! that end-to-end behaviour.

use std::sync::Arc;

use rstest::rstest;

use super::{
    inbound_envelope,
    load_message_assembly_state,
    message_assembler,
    message_assembly_state,
    ok,
    process_assembly_frame,
};
use crate::{
    message_assembler::{FrameSequence, MessageAssemblyState, MessageKey},
    test_helpers,
};

#[rstest]
fn inbound_assembly_rejects_under_declared_single_frame(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut state = Some(load_message_assembly_state(message_assembly_state));
    let mut deser_failures = 0_u32;
    // Declares 3 bytes but carries 5 on the only frame.
    let frame = inbound_envelope(
        1,
        ok(
            test_helpers::first_frame_payload(MessageKey::from(5), b"hello", true, Some(3)),
            "valid test payload",
        ),
    );

    let result = ok(
        process_assembly_frame(&message_assembler, &mut state, &mut deser_failures, frame),
        "malformed declaration should be handled",
    );

    assert!(
        result.is_none(),
        "a declaration the first frame contradicts must not dispatch"
    );
    assert_eq!(
        deser_failures, 1,
        "declared-total rejection should count as one failure"
    );
}

#[rstest]
fn inbound_assembly_rejects_continuation_overshooting_declared_total(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut state = Some(load_message_assembly_state(message_assembly_state));
    let mut deser_failures = 0_u32;
    let first = inbound_envelope(
        1,
        ok(
            test_helpers::first_frame_payload(MessageKey::from(5), b"hello", false, Some(8)),
            "valid test payload",
        ),
    );
    let result = ok(
        process_assembly_frame(&message_assembler, &mut state, &mut deser_failures, first),
        "first frame should process",
    );
    assert!(result.is_none(), "first frame should not complete");

    // 5 + 4 exceeds the declared 8.
    let overrun = inbound_envelope(
        1,
        ok(
            test_helpers::continuation_frame_payload(
                MessageKey::from(5),
                FrameSequence::from(1),
                b"more",
                false,
            ),
            "valid test payload",
        ),
    );
    let result = ok(
        process_assembly_frame(&message_assembler, &mut state, &mut deser_failures, overrun),
        "overshooting continuation should be handled",
    );

    assert!(
        result.is_none(),
        "an overshooting continuation must not dispatch"
    );
    assert_eq!(
        deser_failures, 1,
        "declared-total rejection should count as one failure"
    );
    assert_eq!(
        state
            .as_ref()
            .map_or(usize::MAX, MessageAssemblyState::buffered_count),
        0,
        "the rejected assembly must be freed"
    );
}
