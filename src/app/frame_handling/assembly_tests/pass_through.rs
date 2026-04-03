//! Pass-through tests for `assemble_if_needed`.

use std::sync::Arc;

use rstest::rstest;

use super::{
    AssemblyRuntime,
    inbound_envelope,
    load_message_assembly_state,
    message_assembler,
    message_assembly_state,
    ok,
    test_helpers,
};
use crate::message_assembler::{MessageAssemblyState, MessageKey};

#[rstest]
fn assemble_if_needed_passes_through_when_assembler_is_none(
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut state = Some(load_message_assembly_state(message_assembly_state));

    let envelope = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(MessageKey::from(1), b"A", true, Some(1)),
            "valid test payload",
        ),
    );
    let result = ok(
        super::assemble_if_needed(
            AssemblyRuntime::new(None, &mut state),
            &mut deser_failures,
            envelope.clone(),
            10,
        ),
        "assemble_if_needed should succeed",
    );

    assert_eq!(result.as_ref(), Some(&envelope));
    assert_eq!(deser_failures, 0, "no failures when assembler is absent");
}

#[rstest]
fn assemble_if_needed_passes_through_when_state_is_none(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
) {
    let mut deser_failures = 0_u32;
    let mut state: Option<MessageAssemblyState> = None;

    let envelope = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(MessageKey::from(1), b"A", true, Some(1)),
            "valid test payload",
        ),
    );
    let result = ok(
        super::assemble_if_needed(
            AssemblyRuntime::new(Some(&message_assembler), &mut state),
            &mut deser_failures,
            envelope.clone(),
            10,
        ),
        "assemble_if_needed should succeed",
    );

    assert_eq!(result.as_ref(), Some(&envelope));
    assert_eq!(deser_failures, 0, "no failures when state is absent");
}
