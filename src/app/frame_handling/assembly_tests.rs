//! Unit tests for inbound message assembly integration.
#![cfg(test)]

use std::{
    fmt::Display,
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

fn load_message_assembly_state(
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) -> MessageAssemblyState {
    ok(
        message_assembly_state,
        "message assembly state should initialize",
    )
}

fn require_completed(envelope: Option<Envelope>, context: &str) -> Envelope {
    some(envelope, context)
}

fn ok<T, E>(result: Result<T, E>, context: &str) -> T
where
    E: Display,
{
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

fn some<T>(value: Option<T>, context: &str) -> T {
    match value {
        Some(value) => value,
        None => panic!("{context}"),
    }
}

#[rstest]
fn inbound_assembly_handles_interleaved_sequences(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(load_message_assembly_state(message_assembly_state));

    let key1_first = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(1, b"A1", false, Some(4)),
            "valid test payload",
        ),
    );
    let key2_first = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(2, b"B1", false, Some(4)),
            "valid test payload",
        ),
    );
    let key1_last = inbound_envelope(
        9,
        ok(
            test_helpers::continuation_frame_payload(1, 1, b"A2", true),
            "valid test payload",
        ),
    );
    let key2_last = inbound_envelope(
        9,
        ok(
            test_helpers::continuation_frame_payload(2, 1, b"B2", true),
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
    assert_eq!(completed_a.payload_bytes(), b"A1A2");

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
    assert_eq!(completed_b.payload_bytes(), b"B1B2");

    assert_eq!(deser_failures, 0, "no failures expected");
}

#[rstest]
fn inbound_assembly_rejects_ordering_violations(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(load_message_assembly_state(message_assembly_state));

    let first = inbound_envelope(
        3,
        ok(
            test_helpers::first_frame_payload(99, b"ab", false, Some(6)),
            "valid test payload",
        ),
    );
    let cont_seq1 = inbound_envelope(
        3,
        ok(
            test_helpers::continuation_frame_payload(99, 1, b"cd", false),
            "valid test payload",
        ),
    );
    let cont_seq3 = inbound_envelope(
        3,
        ok(
            test_helpers::continuation_frame_payload(99, 3, b"ef", false),
            "valid test payload",
        ),
    );
    let cont_seq2 = inbound_envelope(
        3,
        ok(
            test_helpers::continuation_frame_payload(99, 2, b"gh", true),
            "valid test payload",
        ),
    );

    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                first,
            ),
            "first frame should process",
        )
        .is_none()
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                cont_seq1,
            ),
            "first continuation should process",
        )
        .is_none()
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                cont_seq3,
            ),
            "out-of-order continuation should be recoverable",
        )
        .is_none()
    );
    assert_eq!(
        deser_failures, 1,
        "ordering violation should count as one failure"
    );

    let completed = require_completed(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                cont_seq2,
            ),
            "recovery continuation should process",
        ),
        "message should complete after recovery",
    );
    assert_eq!(completed.payload_bytes(), b"abcdgh");
}

#[rstest]
fn inbound_assembly_timeout_purges_partial_state(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(load_message_assembly_state(message_assembly_state));

    let first = inbound_envelope(
        5,
        ok(
            test_helpers::first_frame_payload(7, b"ab", false, Some(4)),
            "valid test payload",
        ),
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                first,
            ),
            "first frame should process",
        )
        .is_none()
    );

    // Advance a synthetic clock well past the 5ms assembly timeout so the
    // purge is deterministic and independent of real wall-clock scheduling.
    let well_past_timeout = Instant::now() + Duration::from_secs(1);
    some(
        message_assembly_state.as_mut(),
        "assembly state should exist",
    )
    .purge_expired_at(well_past_timeout);

    let continuation = inbound_envelope(
        5,
        ok(
            test_helpers::continuation_frame_payload(7, 1, b"cd", true),
            "valid test payload",
        ),
    );
    assert!(
        ok(
            process_assembly_frame(
                &message_assembler,
                &mut message_assembly_state,
                &mut deser_failures,
                continuation,
            ),
            "continuation after purge should be recoverable",
        )
        .is_none()
    );
    assert_eq!(
        deser_failures, 1,
        "continuation after timeout purge should count as missing first frame",
    );
}

#[rstest]
fn assemble_if_needed_passes_through_when_assembler_is_none(
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut state = Some(load_message_assembly_state(message_assembly_state));

    let envelope = inbound_envelope(
        9,
        ok(
            test_helpers::first_frame_payload(1, b"A", true, Some(1)),
            "valid test payload",
        ),
    );
    let result = ok(
        assemble_if_needed(
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
            test_helpers::first_frame_payload(1, b"A", true, Some(1)),
            "valid test payload",
        ),
    );
    let result = ok(
        assemble_if_needed(
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

mod routing_metadata;
