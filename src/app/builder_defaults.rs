//! Defaults and helpers for Wireframe application configuration.

use std::{num::NonZeroUsize, time::Duration};

use crate::{
    app::memory_budgets::{BudgetBytes, MemoryBudgets},
    codec::clamp_frame_length,
    fragment::FragmentationConfig,
};

pub(super) const MIN_READ_TIMEOUT_MS: u64 = 1;
pub(super) const MAX_READ_TIMEOUT_MS: u64 = 86_400_000;
/// Default preamble read timeout in milliseconds.
pub(super) const DEFAULT_READ_TIMEOUT_MS: u64 = 100;
const DEFAULT_FRAGMENT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MESSAGE_SIZE_MULTIPLIER: usize = 16;
const DEFAULT_MESSAGE_BUDGET_MULTIPLIER: usize = DEFAULT_MESSAGE_SIZE_MULTIPLIER;
const DEFAULT_CONNECTION_BUDGET_MULTIPLIER: usize = 64;
const DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER: usize = 64;

pub(super) fn default_fragmentation(frame_budget: usize) -> Option<FragmentationConfig> {
    let frame_budget = clamp_frame_length(frame_budget);
    let max_message =
        NonZeroUsize::new(frame_budget.saturating_mul(DEFAULT_MESSAGE_SIZE_MULTIPLIER))
            .or_else(|| NonZeroUsize::new(frame_budget));
    max_message.and_then(|limit| {
        FragmentationConfig::for_frame_budget(frame_budget, limit, DEFAULT_FRAGMENT_TIMEOUT)
    })
}

/// Derive a single [`BudgetBytes`] value from a frame budget and multiplier.
///
/// The `unwrap_or(NonZeroUsize::MIN)` fallback is effectively unreachable
/// because [`clamp_frame_length`] guarantees `frame_budget >= 64`, but it
/// satisfies the `NonZeroUsize` invariant without using the forbidden
/// `.unwrap()` lint.
fn derive_budget(frame_budget: usize, multiplier: usize) -> BudgetBytes {
    let bytes =
        NonZeroUsize::new(frame_budget.saturating_mul(multiplier)).unwrap_or(NonZeroUsize::MIN);
    BudgetBytes::new(bytes)
}

/// Derive sensible memory budgets from the codec frame budget.
///
/// Mirrors the pattern of [`default_fragmentation`], which derives
/// fragmentation settings from the same frame budget. The frame budget is
/// clamped to [`crate::codec::MIN_FRAME_LENGTH`]..=
/// [`crate::codec::MAX_FRAME_LENGTH`] before applying multipliers.
///
/// The per-message multiplier (16) is deliberately aligned with
/// `DEFAULT_MESSAGE_SIZE_MULTIPLIER` used by fragmentation defaults so
/// that the two guards agree on the maximum logical message size.
#[must_use]
pub(super) fn default_memory_budgets(frame_budget: usize) -> MemoryBudgets {
    let frame_budget = clamp_frame_length(frame_budget);
    MemoryBudgets::new(
        derive_budget(frame_budget, DEFAULT_MESSAGE_BUDGET_MULTIPLIER),
        derive_budget(frame_budget, DEFAULT_CONNECTION_BUDGET_MULTIPLIER),
        derive_budget(frame_budget, DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER),
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::codec::{MAX_FRAME_LENGTH, MIN_FRAME_LENGTH};

    #[rstest]
    #[case(1024)]
    #[case(4096)]
    fn default_budgets_scale_and_use_expected_multipliers(#[case] frame_budget: usize) {
        let budgets = default_memory_budgets(frame_budget);
        assert_eq!(
            frame_budget * DEFAULT_MESSAGE_BUDGET_MULTIPLIER,
            budgets.bytes_per_message().as_usize()
        );
        assert_eq!(
            frame_budget * DEFAULT_CONNECTION_BUDGET_MULTIPLIER,
            budgets.bytes_per_connection().as_usize()
        );
        assert_eq!(
            frame_budget * DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER,
            budgets.bytes_in_flight().as_usize()
        );
    }

    #[test]
    fn default_budgets_clamp_minimum_frame_budget() {
        let budgets = default_memory_budgets(10);
        assert_eq!(
            MIN_FRAME_LENGTH * DEFAULT_MESSAGE_BUDGET_MULTIPLIER,
            budgets.bytes_per_message().as_usize()
        );
        assert_eq!(
            MIN_FRAME_LENGTH * DEFAULT_CONNECTION_BUDGET_MULTIPLIER,
            budgets.bytes_per_connection().as_usize()
        );
        assert_eq!(
            MIN_FRAME_LENGTH * DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER,
            budgets.bytes_in_flight().as_usize()
        );
    }

    #[test]
    fn default_budgets_clamp_maximum_frame_budget() {
        let budgets = default_memory_budgets(MAX_FRAME_LENGTH + 1);
        assert_eq!(
            MAX_FRAME_LENGTH * DEFAULT_MESSAGE_BUDGET_MULTIPLIER,
            budgets.bytes_per_message().as_usize()
        );
        assert_eq!(
            MAX_FRAME_LENGTH * DEFAULT_CONNECTION_BUDGET_MULTIPLIER,
            budgets.bytes_per_connection().as_usize()
        );
        assert_eq!(
            MAX_FRAME_LENGTH * DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER,
            budgets.bytes_in_flight().as_usize()
        );
    }

    #[test]
    fn default_budgets_message_budget_aligns_with_fragmentation() {
        let frame_budget = 2048_usize;
        let budgets = default_memory_budgets(frame_budget);
        let expected = frame_budget * DEFAULT_MESSAGE_SIZE_MULTIPLIER;
        assert_eq!(
            expected,
            budgets.bytes_per_message().as_usize(),
            "per-message budget does not match fragmentation multiplier"
        );
    }
}
