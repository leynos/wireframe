//! Tests that retained bytes count towards memory pressure.
//!
//! Request-body and parked bytes are held outside the assembly state, so they
//! reach the pressure calculation through the separate `retained_bytes` input.
//! These tests pin that input to the same aggregate thresholds as assembly and
//! fragmentation bytes.

use std::io;

use proptest::prelude::*;
use rstest::rstest;

use super::{
    super::evaluate_memory_pressure_with_retained,
    ExpectedAction,
    TestResult,
    action_matches,
    budgets,
    custom_budgets,
    state_with_buffered_bytes,
};
use crate::app::MemoryBudgets;

/// Request-body and parked bytes count towards pressure with no assembly state.
///
/// The aggregate cap covers every byte the connection retains, so a connection
/// that has delivered its body to a handler and parked a message must be paced
/// and aborted at the same thresholds as one holding an assembly partial.
#[rstest]
#[case::below_the_soft_limit((0, 0, 79), ExpectedAction::Continue)]
#[case::at_the_soft_limit((0, 0, 80), ExpectedAction::Pause)]
#[case::at_the_hard_limit((0, 0, 100), ExpectedAction::Pause)]
#[case::past_the_hard_limit((0, 0, 101), ExpectedAction::Abort)]
#[case::mixed_retained_total_at_the_soft_limit((40, 20, 20), ExpectedAction::Pause)]
#[case::mixed_retained_total_past_the_hard_limit((40, 20, 41), ExpectedAction::Abort)]
fn retained_bytes_contribute_to_pressure(
    #[case] bytes: (usize, usize, usize),
    #[case] expected: ExpectedAction,
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    let (assembly_bytes, fragment_bytes, retained_bytes) = bytes;
    let state = state_with_buffered_bytes(assembly_bytes)?;
    let action = evaluate_memory_pressure_with_retained(
        (assembly_bytes != 0).then_some(&state),
        Some(budgets?),
        fragment_bytes,
        retained_bytes,
    );
    if !action_matches(&action, &expected.to_representative()) {
        return Err(io::Error::other(format!(
            concat!(
                "assembly={}, fragment={}, retained={}: ",
                "unexpected pressure action {:?}"
            ),
            assembly_bytes, fragment_bytes, retained_bytes, action
        ))
        .into());
    }
    Ok(())
}

proptest! {
    /// The decision depends on the retained total, not on which category holds it.
    ///
    /// Assembly, fragmentation, and parked/queued bytes share one aggregate
    /// cap, so any split of the same total must produce the same action. A
    /// regression that dropped one category from the sum would break this for
    /// partitions that put the pressure entirely in that category.
    #[test]
    fn pressure_depends_on_the_retained_total_not_its_partition(
        assembly in 0usize..=60,
        fragment in 0usize..=60,
        parked in 0usize..=60,
    ) {
        let budgets = custom_budgets(1024, 100, 100).expect("budgets are non-zero");
        let total = assembly.saturating_add(fragment).saturating_add(parked);
        let state = state_with_buffered_bytes(assembly).expect("assembly state builds");
        let consolidated = state_with_buffered_bytes(total).expect("assembly state builds");

        let partitioned = evaluate_memory_pressure_with_retained(
            (assembly != 0).then_some(&state),
            Some(budgets),
            fragment,
            parked,
        );
        let whole = evaluate_memory_pressure_with_retained(
            (total != 0).then_some(&consolidated),
            Some(budgets),
            0,
            0,
        );

        prop_assert_eq!(partitioned, whole);
    }
}
