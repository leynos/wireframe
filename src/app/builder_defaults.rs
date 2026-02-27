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
const DEFAULT_MESSAGE_BUDGET_MULTIPLIER: usize = 16;
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
    let per_message =
        NonZeroUsize::new(frame_budget.saturating_mul(DEFAULT_MESSAGE_BUDGET_MULTIPLIER))
            .unwrap_or(NonZeroUsize::MIN);
    let per_connection =
        NonZeroUsize::new(frame_budget.saturating_mul(DEFAULT_CONNECTION_BUDGET_MULTIPLIER))
            .unwrap_or(NonZeroUsize::MIN);
    let in_flight =
        NonZeroUsize::new(frame_budget.saturating_mul(DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER))
            .unwrap_or(NonZeroUsize::MIN);
    MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    )
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use rstest::rstest;

    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    #[rstest]
    fn default_budgets_use_expected_multipliers() -> TestResult {
        let budgets = default_memory_budgets(1024);
        if budgets.bytes_per_message().as_usize() != 16_384 {
            return Err(io::Error::other(format!(
                "expected bytes_per_message=16384, got {}",
                budgets.bytes_per_message().as_usize()
            ))
            .into());
        }
        if budgets.bytes_per_connection().as_usize() != 65_536 {
            return Err(io::Error::other(format!(
                "expected bytes_per_connection=65536, got {}",
                budgets.bytes_per_connection().as_usize()
            ))
            .into());
        }
        if budgets.bytes_in_flight().as_usize() != 65_536 {
            return Err(io::Error::other(format!(
                "expected bytes_in_flight=65536, got {}",
                budgets.bytes_in_flight().as_usize()
            ))
            .into());
        }
        Ok(())
    }

    #[rstest]
    fn default_budgets_scale_with_frame_budget() -> TestResult {
        let budgets = default_memory_budgets(4096);
        if budgets.bytes_per_message().as_usize() != 4096 * 16 {
            return Err(io::Error::other(format!(
                "expected bytes_per_message={}, got {}",
                4096 * 16,
                budgets.bytes_per_message().as_usize()
            ))
            .into());
        }
        if budgets.bytes_per_connection().as_usize() != 4096 * 64 {
            return Err(io::Error::other(format!(
                "expected bytes_per_connection={}, got {}",
                4096 * 64,
                budgets.bytes_per_connection().as_usize()
            ))
            .into());
        }
        if budgets.bytes_in_flight().as_usize() != 4096 * 64 {
            return Err(io::Error::other(format!(
                "expected bytes_in_flight={}, got {}",
                4096 * 64,
                budgets.bytes_in_flight().as_usize()
            ))
            .into());
        }
        Ok(())
    }

    #[rstest]
    fn default_budgets_clamp_minimum_frame_budget() -> TestResult {
        let budgets = default_memory_budgets(10);
        if budgets.bytes_per_message().as_usize() != 64 * 16 {
            return Err(io::Error::other(format!(
                "expected clamped bytes_per_message={}, got {}",
                64 * 16,
                budgets.bytes_per_message().as_usize()
            ))
            .into());
        }
        Ok(())
    }

    #[rstest]
    fn default_budgets_clamp_maximum_frame_budget() -> TestResult {
        let max_frame: usize = 16 * 1024 * 1024;
        let budgets = default_memory_budgets(max_frame + 1);
        if budgets.bytes_per_message().as_usize() != max_frame * 16 {
            return Err(io::Error::other(format!(
                "expected clamped bytes_per_message={}, got {}",
                max_frame * 16,
                budgets.bytes_per_message().as_usize()
            ))
            .into());
        }
        Ok(())
    }

    #[rstest]
    fn default_budgets_message_budget_aligns_with_fragmentation() -> TestResult {
        let frame_budget = 2048_usize;
        let budgets = default_memory_budgets(frame_budget);
        let expected = frame_budget * DEFAULT_MESSAGE_SIZE_MULTIPLIER;
        if budgets.bytes_per_message().as_usize() != expected {
            return Err(io::Error::other(format!(
                "per-message budget {} does not match fragmentation multiplier result {}",
                budgets.bytes_per_message().as_usize(),
                expected
            ))
            .into());
        }
        Ok(())
    }
}
