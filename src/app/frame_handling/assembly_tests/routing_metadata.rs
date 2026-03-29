//! Routing metadata preservation tests for interleaved inbound assemblies.

use rstest::rstest;

use super::*;
use crate::message_assembler::{FrameSequence, MessageKey};

#[rstest]
fn interleaved_assemblies_preserve_first_frame_routing_metadata(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(load_message_assembly_state(message_assembly_state));

    // Key 1: envelope_id=10, correlation_id=100
    let key1_first = Envelope::new(
        10,
        Some(100),
        ok(
            test_helpers::first_frame_payload(MessageKey::from(1), b"A1", false, Some(4)),
            "valid test payload",
        ),
    );
    // Key 2: envelope_id=20, correlation_id=200
    let key2_first = Envelope::new(
        20,
        Some(200),
        ok(
            test_helpers::first_frame_payload(MessageKey::from(2), b"B1", false, Some(4)),
            "valid test payload",
        ),
    );

    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                key1_first,
            ),
            "first key1 should process",
        )
        .is_none()
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                key2_first,
            ),
            "first key2 should process",
        )
        .is_none()
    );

    // Complete key 2 first (with a different envelope id on the continuation)
    let key2_last = Envelope::new(
        88,
        Some(888),
        ok(
            test_helpers::continuation_frame_payload(
                MessageKey::from(2),
                FrameSequence::from(1),
                b"B2",
                true,
            ),
            "valid test payload",
        ),
    );
    let completed_b = require_completed(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                key2_last,
            ),
            "key2 completion should process",
        ),
        "key2 should complete",
    );

    assert_eq!(completed_b.id, 20, "key2 envelope id from first frame");
    assert_eq!(completed_b.correlation_id, Some(200));
    assert_eq!(completed_b.payload_bytes(), b"B1B2");

    // Complete key 1
    let key1_last = Envelope::new(
        77,
        Some(777),
        ok(
            test_helpers::continuation_frame_payload(
                MessageKey::from(1),
                FrameSequence::from(1),
                b"A2",
                true,
            ),
            "valid test payload",
        ),
    );
    let completed_a = require_completed(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                key1_last,
            ),
            "key1 completion should process",
        ),
        "key1 should complete",
    );

    assert_eq!(completed_a.id, 10, "key1 envelope id from first frame");
    assert_eq!(completed_a.correlation_id, Some(100));
    assert_eq!(completed_a.payload_bytes(), b"A1A2");
}
