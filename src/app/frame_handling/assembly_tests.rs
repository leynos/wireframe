//! Unit tests for inbound message assembly integration.

use std::{io, num::NonZeroUsize, sync::Arc, thread, time::Duration};

use bytes::{BufMut, BytesMut};
use rstest::{fixture, rstest};

use super::{AssemblyRuntime, assemble_if_needed, purge_expired_assemblies};
use crate::{app::Envelope, message_assembler::MessageAssemblyState, test_helpers::TestAssembler};

#[fixture]
fn message_assembler() -> Arc<dyn crate::message_assembler::MessageAssembler> {
    Arc::new(TestAssembler)
}

#[fixture]
fn message_assembly_state() -> MessageAssemblyState {
    MessageAssemblyState::new(
        NonZeroUsize::new(1024).unwrap_or(NonZeroUsize::MIN),
        Duration::from_millis(5),
    )
}

fn first_frame_payload(key: u64, total: Option<u32>, body: &[u8], is_last: bool) -> Vec<u8> {
    let mut payload = BytesMut::new();
    payload.put_u8(0x01);
    let mut flags = 0u8;
    if is_last {
        flags |= 0b1;
    }
    if total.is_some() {
        flags |= 0b10;
    }
    payload.put_u8(flags);
    payload.put_u64(key);
    payload.put_u16(0);
    payload.put_u32(u32::try_from(body.len()).unwrap_or(u32::MAX));
    if let Some(total) = total {
        payload.put_u32(total);
    }
    payload.extend_from_slice(body);
    payload.to_vec()
}

fn continuation_frame_payload(key: u64, sequence: u32, body: &[u8], is_last: bool) -> Vec<u8> {
    let mut payload = BytesMut::new();
    payload.put_u8(0x02);
    let mut flags = 0b10;
    if is_last {
        flags |= 0b1;
    }
    payload.put_u8(flags);
    payload.put_u64(key);
    payload.put_u32(u32::try_from(body.len()).unwrap_or(u32::MAX));
    payload.put_u32(sequence);
    payload.extend_from_slice(body);
    payload.to_vec()
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
fn inbound_assembly_handles_interleaved_sequences(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: MessageAssemblyState,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state);

    let key1_first = inbound_envelope(9, first_frame_payload(1, Some(4), b"A1", false));
    let key2_first = inbound_envelope(9, first_frame_payload(2, Some(4), b"B1", false));
    let key1_last = inbound_envelope(9, continuation_frame_payload(1, 1, b"A2", true));
    let key2_last = inbound_envelope(9, continuation_frame_payload(2, 1, b"B2", true));

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
}

#[rstest]
fn inbound_assembly_rejects_ordering_violations(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: MessageAssemblyState,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state);

    let first = inbound_envelope(3, first_frame_payload(99, Some(6), b"ab", false));
    let cont_seq1 = inbound_envelope(3, continuation_frame_payload(99, 1, b"cd", false));
    let cont_seq3 = inbound_envelope(3, continuation_frame_payload(99, 3, b"ef", false));
    let cont_seq2 = inbound_envelope(3, continuation_frame_payload(99, 2, b"gh", true));

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
}

#[rstest]
fn inbound_assembly_timeout_purges_partial_state(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: MessageAssemblyState,
) {
    let mut deser_failures = 0_u32;
    let mut message_assembly_state = Some(message_assembly_state);

    let first = inbound_envelope(5, first_frame_payload(7, Some(4), b"ab", false));
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

    thread::sleep(Duration::from_millis(20));
    purge_expired_assemblies(&mut message_assembly_state);

    let continuation = inbound_envelope(5, continuation_frame_payload(7, 1, b"cd", true));
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
}
