//! Unit tests for inbound message assembly integration.

use std::{
    io,
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};

use rstest::{fixture, rstest};

use super::{AssemblyRuntime, assemble_if_needed};
use crate::{
    app::Envelope,
    message_assembler::MessageAssemblyState,
    test_helpers::{self, TestAssembler},
};

#[fixture]
fn message_assembler() -> Arc<dyn crate::message_assembler::MessageAssembler> {
    Arc::new(TestAssembler)
}

#[fixture]
fn message_assembly_state() -> Result<MessageAssemblyState, &'static str> {
    let size = NonZeroUsize::new(1024).ok_or("failed to create NonZeroUsize")?;
    Ok(MessageAssemblyState::new(size, Duration::from_millis(5)))
}

fn inbound_envelope(id: u32, payload: Vec<u8>) -> Envelope { Envelope::new(id, Some(7), payload) }

/// Test helper to process a frame through the assembly pipeline.
fn process_assembly_frame(
    assembler: &Arc<dyn crate::message_assembler::MessageAssembler>,
    state: &mut Option<MessageAssemblyState>,
    deser_failures: &mut u32,
    envelope: Envelope,
) -> io::Result<Option<Envelope>> {
    assemble_if_needed(
        AssemblyRuntime::new(Some(assembler), state),
        deser_failures,
        envelope,
        10,
    )
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "test assertions are the correct failure mechanism in unit tests"
)]
fn inbound_assembly_handles_interleaved_sequences(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> Result<(), &'static str> {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state?);

    let key1_first = inbound_envelope(
        9,
        test_helpers::first_frame_payload(1, b"A1", false, Some(4)).expect("valid test payload"),
    );
    let key2_first = inbound_envelope(
        9,
        test_helpers::first_frame_payload(2, b"B1", false, Some(4)).expect("valid test payload"),
    );
    let key1_last = inbound_envelope(
        9,
        test_helpers::continuation_frame_payload(1, 1, b"A2", true).expect("valid test payload"),
    );
    let key2_last = inbound_envelope(
        9,
        test_helpers::continuation_frame_payload(2, 1, b"B2", true).expect("valid test payload"),
    );

    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            key1_first,
        )
        .expect("first key1 should process")
        .is_none()
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            key2_first,
        )
        .expect("first key2 should process")
        .is_none()
    );

    let completed_a = process_assembly_frame(
        &message_assembler,
        &mut message_assembly_state,
        &mut deser_failures,
        key1_last,
    )
    .expect("key1 completion should process")
    .expect("key1 should complete");
    assert_eq!(completed_a.payload_bytes(), b"A1A2");

    let completed_b = process_assembly_frame(
        &message_assembler,
        &mut message_assembly_state,
        &mut deser_failures,
        key2_last,
    )
    .expect("key2 completion should process")
    .expect("key2 should complete");
    assert_eq!(completed_b.payload_bytes(), b"B1B2");

    assert_eq!(deser_failures, 0, "no failures expected");
    Ok(())
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "test assertions are the correct failure mechanism in unit tests"
)]
fn inbound_assembly_rejects_ordering_violations(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> Result<(), &'static str> {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state?);

    let first = inbound_envelope(
        3,
        test_helpers::first_frame_payload(99, b"ab", false, Some(6)).expect("valid test payload"),
    );
    let cont_seq1 = inbound_envelope(
        3,
        test_helpers::continuation_frame_payload(99, 1, b"cd", false).expect("valid test payload"),
    );
    let cont_seq3 = inbound_envelope(
        3,
        test_helpers::continuation_frame_payload(99, 3, b"ef", false).expect("valid test payload"),
    );
    let cont_seq2 = inbound_envelope(
        3,
        test_helpers::continuation_frame_payload(99, 2, b"gh", true).expect("valid test payload"),
    );

    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            first,
        )
        .expect("first frame should process")
        .is_none()
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            cont_seq1,
        )
        .expect("first continuation should process")
        .is_none()
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            cont_seq3,
        )
        .expect("out-of-order continuation should be recoverable")
        .is_none()
    );
    assert_eq!(
        deser_failures, 1,
        "ordering violation should count as one failure"
    );

    let completed = process_assembly_frame(
        &message_assembler,
        &mut message_assembly_state,
        &mut deser_failures,
        cont_seq2,
    )
    .expect("recovery continuation should process")
    .expect("message should complete after recovery");
    assert_eq!(completed.payload_bytes(), b"abcdgh");
    Ok(())
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "test assertions are the correct failure mechanism in unit tests"
)]
fn inbound_assembly_timeout_purges_partial_state(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> Result<(), &'static str> {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state?);

    let first = inbound_envelope(
        5,
        test_helpers::first_frame_payload(7, b"ab", false, Some(4)).expect("valid test payload"),
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            first,
        )
        .expect("first frame should process")
        .is_none()
    );

    // Advance a synthetic clock well past the 5ms assembly timeout so the
    // purge is deterministic and independent of real wall-clock scheduling.
    let well_past_timeout = Instant::now() + Duration::from_secs(1);
    message_assembly_state
        .as_mut()
        .expect("assembly state should exist")
        .purge_expired_at(well_past_timeout);

    let continuation = inbound_envelope(
        5,
        test_helpers::continuation_frame_payload(7, 1, b"cd", true).expect("valid test payload"),
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            continuation,
        )
        .expect("continuation after purge should be recoverable")
        .is_none()
    );
    assert_eq!(
        deser_failures, 1,
        "continuation after timeout purge should count as missing first frame",
    );
    Ok(())
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "test assertions are the correct failure mechanism in unit tests"
)]
fn assemble_if_needed_passes_through_when_assembler_is_none(
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> Result<(), &'static str> {
    let mut deser_failures = 0_u32;
    let mut state = Some(message_assembly_state?);

    let envelope = inbound_envelope(
        9,
        test_helpers::first_frame_payload(1, b"A", true, Some(1)).expect("valid test payload"),
    );
    let result = assemble_if_needed(
        AssemblyRuntime::new(None, &mut state),
        &mut deser_failures,
        envelope.clone(),
        10,
    )
    .expect("assemble_if_needed should succeed");

    assert_eq!(result.as_ref(), Some(&envelope));
    assert_eq!(deser_failures, 0, "no failures when assembler is absent");
    Ok(())
}

#[rstest]
fn assemble_if_needed_passes_through_when_state_is_none(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
) {
    let mut deser_failures = 0_u32;
    let mut state: Option<MessageAssemblyState> = None;

    let envelope = inbound_envelope(
        9,
        test_helpers::first_frame_payload(1, b"A", true, Some(1)).expect("valid test payload"),
    );
    let result = assemble_if_needed(
        AssemblyRuntime::new(Some(&message_assembler), &mut state),
        &mut deser_failures,
        envelope.clone(),
        10,
    )
    .expect("assemble_if_needed should succeed");

    assert_eq!(result.as_ref(), Some(&envelope));
    assert_eq!(deser_failures, 0, "no failures when state is absent");
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "test assertions are the correct failure mechanism in unit tests"
)]
fn interleaved_assemblies_preserve_first_frame_routing_metadata(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> Result<(), &'static str> {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state?);

    // Key 1: envelope_id=10, correlation_id=100
    let key1_first = Envelope::new(
        10,
        Some(100),
        test_helpers::first_frame_payload(1, b"A1", false, Some(4)).expect("valid test payload"),
    );
    // Key 2: envelope_id=20, correlation_id=200
    let key2_first = Envelope::new(
        20,
        Some(200),
        test_helpers::first_frame_payload(2, b"B1", false, Some(4)).expect("valid test payload"),
    );

    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            key1_first,
        )
        .expect("first key1 should process")
        .is_none()
    );
    assert!(
        process_assembly_frame(
            &message_assembler,
            &mut message_assembly_state,
            &mut deser_failures,
            key2_first,
        )
        .expect("first key2 should process")
        .is_none()
    );

    // Complete key 2 first (with a different envelope id on the continuation)
    let key2_last = Envelope::new(
        88,
        Some(888),
        test_helpers::continuation_frame_payload(2, 1, b"B2", true).expect("valid test payload"),
    );
    let completed_b = process_assembly_frame(
        &message_assembler,
        &mut message_assembly_state,
        &mut deser_failures,
        key2_last,
    )
    .expect("key2 completion should process")
    .expect("key2 should complete");

    assert_eq!(completed_b.id, 20, "key2 envelope id from first frame");
    assert_eq!(completed_b.correlation_id, Some(200));
    assert_eq!(completed_b.payload_bytes(), b"B1B2");

    // Complete key 1
    let key1_last = Envelope::new(
        77,
        Some(777),
        test_helpers::continuation_frame_payload(1, 1, b"A2", true).expect("valid test payload"),
    );
    let completed_a = process_assembly_frame(
        &message_assembler,
        &mut message_assembly_state,
        &mut deser_failures,
        key1_last,
    )
    .expect("key1 completion should process")
    .expect("key1 should complete");

    assert_eq!(completed_a.id, 10, "key1 envelope id from first frame");
    assert_eq!(completed_a.correlation_id, Some(100));
    assert_eq!(completed_a.payload_bytes(), b"A1A2");
    Ok(())
}
