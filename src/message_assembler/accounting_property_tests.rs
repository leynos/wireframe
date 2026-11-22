//! Property tests for running byte totals and the expiry index.
//!
//! The example tests name the outcomes of insert, append, completion, and
//! purge. These properties drive arbitrary interleavings of them and check the
//! invariants that keep admission O(1): the running payload total always equals
//! a full rescan, the charged total always equals that payload plus one fixed
//! charge per live entry, and the deadline index always holds exactly one
//! record per live entry.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use proptest::prelude::*;

use crate::message_assembler::{
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES,
    MessageAssemblyState,
    MessageKey,
};

/// One mutation applied to the state under test.
#[derive(Clone, Copy, Debug)]
enum Operation {
    /// Start an assembly, possibly completing a single-frame message.
    First {
        key: u8,
        body_len: usize,
        metadata_len: usize,
        is_last: bool,
        declared: Option<usize>,
        millis: u64,
    },
    /// Continue or complete an existing assembly.
    Continuation {
        key: u8,
        sequence: Option<u32>,
        body_len: usize,
        is_last: bool,
        millis: u64,
    },
    /// Sweep expired assemblies at an explicit clock reading.
    Purge { millis: u64 },
}

/// Generate one operation with small keys and payloads so interleavings are
/// frequent and the interesting admission failures are reachable.
fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        4 => (
            0_u8..4,
            0_usize..8,
            0_usize..4,
            any::<bool>(),
            proptest::option::of(0_usize..16),
            0_u64..120_000,
        )
            .prop_map(
                |(key, body_len, metadata_len, is_last, declared, millis)| Operation::First {
                    key,
                    body_len,
                    metadata_len,
                    is_last,
                    declared,
                    millis,
                },
            ),
        4 => (
            0_u8..4,
            proptest::option::of(0_u32..4),
            0_usize..8,
            any::<bool>(),
            0_u64..120_000,
        )
            .prop_map(
                |(key, sequence, body_len, is_last, millis)| Operation::Continuation {
                    key,
                    sequence,
                    body_len,
                    is_last,
                    millis,
                },
            ),
        1 => (0_u64..120_000).prop_map(|millis| Operation::Purge { millis }),
    ]
}

/// Build a non-zero limit from a generated value.
fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("generated limits are non-zero")
}

/// Build a state from the generated budget, cap, and timeout configuration.
fn subject(
    connection: Option<usize>,
    in_flight: Option<usize>,
    cap: Option<usize>,
    timeout_ms: u64,
) -> MessageAssemblyState {
    let state = MessageAssemblyState::with_budgets(
        nz(1024),
        Duration::from_millis(timeout_ms),
        connection.map(nz),
        in_flight.map(nz),
    );
    state.with_max_concurrent_assemblies(cap.map(nz))
}

/// Apply one operation at `base + millis`.
fn apply(state: &mut MessageAssemblyState, base: Instant, operation: Operation) {
    match operation {
        Operation::First {
            key,
            body_len,
            metadata_len,
            is_last,
            declared,
            millis,
        } => {
            let body = vec![0_u8; body_len];
            let metadata = vec![0_u8; metadata_len];
            let header = FirstFrameHeader {
                message_key: MessageKey::from(u64::from(key)),
                metadata_len,
                body_len,
                total_body_len: declared,
                is_last,
            };
            let Ok(input) =
                FirstFrameInput::new(&header, EnvelopeRouting::default(), metadata, &body)
            else {
                return;
            };
            let _outcome = state.accept_first_frame_at(input, base + Duration::from_millis(millis));
        }
        Operation::Continuation {
            key,
            sequence,
            body_len,
            is_last,
            millis,
        } => {
            let body = vec![0_u8; body_len];
            let header = ContinuationFrameHeader {
                message_key: MessageKey::from(u64::from(key)),
                sequence: sequence.map(FrameSequence),
                body_len,
                is_last,
            };
            let _outcome = state.accept_continuation_frame_at(
                &header,
                &body,
                base + Duration::from_millis(millis),
            );
        }
        Operation::Purge { millis } => {
            state.purge_expired_at(base + Duration::from_millis(millis));
        }
    }
}

/// Check every running-total and index invariant against a full recomputation.
fn assert_invariants(state: &MessageAssemblyState) -> Result<(), TestCaseError> {
    let payload = state.total_buffered_bytes();
    prop_assert_eq!(
        payload,
        state.recompute_buffered_bytes(),
        "running payload total diverged from a full rescan"
    );

    let expected_charged = payload.saturating_add(
        state
            .buffered_count()
            .saturating_mul(MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES),
    );
    prop_assert_eq!(
        state.total_charged_bytes(),
        expected_charged,
        "charged total must be payload plus one fixed charge per live entry"
    );
    prop_assert_eq!(
        state.indexed_deadlines(),
        state.buffered_count(),
        "the expiry index must hold one record per live assembly"
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    })]

    /// Arbitrary operation sequences preserve the running totals and index.
    #[test]
    fn running_totals_survive_arbitrary_operations(
        operations in proptest::collection::vec(operation(), 0..24),
        connection in proptest::option::of(1_usize..=512),
        in_flight in proptest::option::of(1_usize..=512),
        cap in proptest::option::of(1_usize..=4),
        timeout_ms in 0_u64..=60_000,
    ) {
        let mut state = subject(connection, in_flight, cap, timeout_ms);
        let base = Instant::now();

        for operation in operations {
            apply(&mut state, base, operation);
            assert_invariants(&state)?;
        }

        // A sweep beyond every generated timestamp must release all state and
        // leave all three derived values mutually consistent.
        state.purge_expired_at(base + Duration::from_secs(600));
        prop_assert_eq!(state.buffered_count(), 0);
        prop_assert_eq!(state.total_buffered_bytes(), 0);
        prop_assert_eq!(state.total_charged_bytes(), 0);
        prop_assert_eq!(state.indexed_deadlines(), 0);
    }
}
