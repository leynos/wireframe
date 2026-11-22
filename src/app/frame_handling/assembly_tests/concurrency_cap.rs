//! Inbound coverage for the concurrent-assembly cap (issue #830).

use std::{num::NonZeroUsize, sync::Arc};

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
    message_assembler::{MessageAssemblyState, MessageKey},
    test_helpers,
};

/// A first frame beyond the concurrent-assembly cap is recorded against the
/// deserialization-failure budget without disturbing admitted assemblies.
#[rstest]
fn inbound_assembly_rejects_frames_beyond_the_concurrency_cap(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let cap = NonZeroUsize::new(2).expect("the cap is non-zero");
    let state = load_message_assembly_state(message_assembly_state)
        .with_max_concurrent_assemblies(Some(cap));
    let mut assembly_state = Some(state);
    let mut deser_failures = 0_u32;

    for key in 1_u64..=2 {
        let envelope = inbound_envelope(
            9,
            ok(
                test_helpers::first_frame_payload(MessageKey::from(key), b"", false, None),
                "valid test payload",
            ),
        );
        assert!(
            ok(
                process_assembly_frame(
                    &message_assembler,
                    &mut assembly_state,
                    &mut deser_failures,
                    envelope,
                ),
                "admitted first frame should process",
            )
            .is_none(),
            "admitted first frames do not complete"
        );
    }
    assert_eq!(deser_failures, 0, "admitted frames report no failure");

    let over_cap = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(MessageKey::from(3), b"", false, None),
            "valid test payload",
        ),
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut assembly_state,
                &mut deser_failures,
                over_cap,
            ),
            "cap rejection is handled as a deserialization failure",
        )
        .is_none(),
        "a rejected first frame does not dispatch"
    );
    assert_eq!(
        deser_failures, 1,
        "the cap rejection counts as one deserialization failure"
    );
    assert_eq!(
        assembly_state
            .as_ref()
            .expect("assembly state should exist")
            .buffered_count(),
        2,
        "both admitted assemblies survive the rejection"
    );
}
