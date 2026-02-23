//! Soft-limit back-pressure helpers for inbound read pacing.
//!
//! These helpers detect when buffered assembly bytes approach configured
//! aggregate memory budgets and provide a small pause duration that throttles
//! subsequent socket reads.

use std::time::Duration;

use crate::{app::MemoryBudgets, message_assembler::MessageAssemblyState};

/// Soft-pressure threshold numerator (4/5 == 80%).
const SOFT_LIMIT_NUMERATOR: u128 = 4;
/// Soft-pressure threshold denominator (4/5 == 80%).
const SOFT_LIMIT_DENOMINATOR: u128 = 5;
/// Read-pacing delay applied while under soft budget pressure.
const SOFT_LIMIT_PAUSE_DURATION: Duration = Duration::from_millis(5);

/// Return `true` when inbound reads should be paced due to soft budget pressure.
#[must_use]
pub(crate) fn should_pause_inbound_reads(
    state: Option<&MessageAssemblyState>,
    budgets: Option<MemoryBudgets>,
) -> bool {
    let (Some(state), Some(budgets)) = (state, budgets) else {
        return false;
    };

    let buffered_bytes = state.total_buffered_bytes();
    let aggregate_limit = active_aggregate_limit_bytes(budgets);
    is_at_or_above_soft_limit(buffered_bytes, aggregate_limit)
}

/// Duration to pause between inbound reads while soft pressure is active.
#[must_use]
pub(crate) const fn soft_limit_pause_duration() -> Duration { SOFT_LIMIT_PAUSE_DURATION }

fn active_aggregate_limit_bytes(budgets: MemoryBudgets) -> usize {
    budgets
        .bytes_per_connection()
        .as_usize()
        .min(budgets.bytes_in_flight().as_usize())
}

fn is_at_or_above_soft_limit(buffered_bytes: usize, aggregate_limit: usize) -> bool {
    let lhs = (buffered_bytes as u128).saturating_mul(SOFT_LIMIT_DENOMINATOR);
    let rhs = (aggregate_limit as u128).saturating_mul(SOFT_LIMIT_NUMERATOR);
    lhs >= rhs
}
