//! Memory budget pressure helpers for inbound read pacing.
//!
//! These helpers detect when buffered assembly bytes approach or exceed
//! configured aggregate memory budgets. The module provides two tiers of
//! protection:
//!
//! - **Soft limit** (80% of aggregate cap): paces reads with a short pause.
//! - **Hard cap** (100% of aggregate cap): signals immediate connection abort.

use std::time::Duration;

use log::{debug, warn};
use tokio::{io, time::sleep};

use crate::{
    app::{MemoryBudgets, builder_defaults::default_memory_budgets},
    message_assembler::MessageAssemblyState,
};

/// Soft-pressure threshold numerator (4/5 == 80%).
const SOFT_LIMIT_NUMERATOR: u128 = 4;
/// Soft-pressure threshold denominator (4/5 == 80%).
const SOFT_LIMIT_DENOMINATOR: u128 = 5;
/// Read-pacing delay applied while under soft budget pressure.
const SOFT_LIMIT_PAUSE_DURATION: Duration = Duration::from_millis(5);

/// Action to take based on current memory budget pressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryPressureAction {
    /// No pressure; proceed normally.
    Continue,
    /// Soft pressure; pause reads briefly before continuing.
    Pause(Duration),
    /// Hard cap breached; abort the connection immediately.
    Abort,
}

/// Evaluate memory budget pressure and return the appropriate action.
///
/// The hard cap is checked first (connection abort at 100% of the aggregate
/// limit), then the soft limit (read pacing at 80%). `retained_bytes` covers
/// request-body chunks queued for a handler and complete messages parked
/// while a streaming body owns the dispatch slot.
/// Including them keeps the aggregate cap meaningful for streaming routes,
/// whose frames never enter the assembly budgets.
#[must_use]
pub(crate) fn evaluate_memory_pressure_with_retained(
    state: Option<&MessageAssemblyState>,
    budgets: Option<MemoryBudgets>,
    fragment_bytes: usize,
    retained_bytes: usize,
) -> MemoryPressureAction {
    if has_hard_cap_been_breached(state, budgets, fragment_bytes, retained_bytes) {
        return MemoryPressureAction::Abort;
    }
    if should_pause_inbound_reads(state, budgets, fragment_bytes, retained_bytes) {
        return MemoryPressureAction::Pause(SOFT_LIMIT_PAUSE_DURATION);
    }
    MemoryPressureAction::Continue
}

/// Act on the result of [`evaluate_memory_pressure_with_retained`].
///
/// - `Abort`: logs a warning and returns `Err(InvalidData)`.
/// - `Pause(d)`: logs at debug level, sleeps for `d`, then purges expired assemblies via the
///   caller-supplied closure.
/// - `Continue`: no-op.
///
/// # Errors
///
/// Returns an [`io::Error`] with kind `InvalidData` when the hard cap is
/// breached.
pub(crate) async fn apply_memory_pressure(
    action: MemoryPressureAction,
    mut purge: impl FnMut(),
) -> io::Result<()> {
    match action {
        MemoryPressureAction::Abort => {
            warn!("memory budget hard cap exceeded; aborting connection");
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "per-connection memory budget hard cap exceeded",
            ))
        }
        MemoryPressureAction::Pause(duration) => {
            debug!("soft memory budget pressure; pausing inbound reads");
            sleep(duration).await;
            purge();
            Ok(())
        }
        MemoryPressureAction::Continue => Ok(()),
    }
}

/// Return `true` when buffered assembly bytes strictly exceed the aggregate
/// budget cap, indicating the connection must be aborted immediately.
///
/// This is a defence-in-depth safety net. Under normal operation, per-frame
/// budget enforcement (8.3.2) prevents the total from exceeding the limit.
#[must_use]
pub(super) fn has_hard_cap_been_breached(
    state: Option<&MessageAssemblyState>,
    budgets: Option<MemoryBudgets>,
    fragment_bytes: usize,
    retained_bytes: usize,
) -> bool {
    let Some(memory_budgets) = budgets else {
        return false;
    };
    let buffered_bytes = retained_total(state, fragment_bytes, retained_bytes);
    let aggregate_limit = active_aggregate_limit_bytes(memory_budgets);
    buffered_bytes > aggregate_limit
}

/// Return `true` when inbound reads should be paced due to soft budget pressure.
#[must_use]
pub(super) fn should_pause_inbound_reads(
    state: Option<&MessageAssemblyState>,
    budgets: Option<MemoryBudgets>,
    fragment_bytes: usize,
    retained_bytes: usize,
) -> bool {
    let Some(memory_budgets) = budgets else {
        return false;
    };

    let buffered_bytes = retained_total(state, fragment_bytes, retained_bytes);
    let aggregate_limit = active_aggregate_limit_bytes(memory_budgets);
    is_at_or_above_soft_limit(buffered_bytes, aggregate_limit)
}

/// Sum every byte the connection currently retains.
fn retained_total(
    state: Option<&MessageAssemblyState>,
    fragment_bytes: usize,
    retained_bytes: usize,
) -> usize {
    state
        .map_or(0, MessageAssemblyState::total_buffered_bytes)
        .saturating_add(fragment_bytes)
        .saturating_add(retained_bytes)
}

/// Select the stricter connection-wide or in-flight cap.
///
/// This is the limit the streaming dispatch slot uses when admitting a body
/// chunk, so queued body bytes and parked messages share one aggregate budget
/// with assembly partials and retained fragments.
#[must_use]
pub(crate) fn active_aggregate_limit_bytes(budgets: MemoryBudgets) -> usize {
    budgets
        .bytes_per_connection()
        .as_usize()
        .min(budgets.bytes_in_flight().as_usize())
}

/// Resolve the effective memory budgets for one connection.
///
/// Returns the explicit budgets if configured, or derives sensible
/// defaults from `frame_budget` using the same multiplier pattern as
/// fragmentation defaults.
#[must_use]
pub(crate) fn resolve_effective_budgets(
    explicit: Option<MemoryBudgets>,
    frame_budget: usize,
) -> MemoryBudgets {
    explicit.unwrap_or_else(|| default_memory_budgets(frame_budget))
}

/// Compare buffered bytes against the configured soft pause threshold safely.
const fn is_at_or_above_soft_limit(buffered_bytes: usize, aggregate_limit: usize) -> bool {
    let lhs = (buffered_bytes as u128).saturating_mul(SOFT_LIMIT_DENOMINATOR);
    let rhs = (aggregate_limit as u128).saturating_mul(SOFT_LIMIT_NUMERATOR);
    lhs >= rhs
}
