//! Budget enforcement helpers for message assembly.
//!
//! This module provides aggregate budget checks applied during frame
//! acceptance. Per-message size limits are handled by the existing
//! `check_size_limit` function in [`super::state`]; this module adds
//! per-connection and in-flight aggregate budget enforcement.

use std::num::NonZeroUsize;

use super::{MessageKey, error::MessageAssemblyError};

/// Fixed per-entry charge applied to aggregate budgets.
///
/// An assembly costs memory beyond the bytes it carries: the map entry, its
/// routing metadata, the ordering state, and the deadline record. Charging a
/// flat minimum keeps an empty assembly from counting as zero bytes, so a peer
/// cannot create unbounded in-flight entries while staying below the aggregate
/// limits.
pub const MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES: usize = 64;

/// Paired connection and in-flight budget limits.
///
/// Bundled into a struct so call-sites pass a single value instead of two
/// `Option<NonZeroUsize>` parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct AggregateBudgets {
    /// Maximum bytes retained across all assemblies on one connection.
    pub(super) connection: Option<NonZeroUsize>,
    /// Maximum bytes retained for frames currently awaiting completion.
    pub(super) in_flight: Option<NonZeroUsize>,
}

impl AggregateBudgets {
    /// Returns `true` when at least one aggregate budget limit is configured.
    // Equivalent mutant (`is_enabled` → `true`): a hot-path optimization only;
    // `check_aggregate_budgets` returns `Ok(())` when both budgets are `None`,
    // so forcing the gate on changes nothing observable.
    #[cfg_attr(test, mutants::skip)]
    pub(super) const fn is_enabled(&self) -> bool {
        self.connection.is_some() || self.in_flight.is_some()
    }
}

/// Every limit that governs whether a continuation body may be buffered.
///
/// Bundled so admission receives one value rather than a growing list of
/// parameters, and so a caller cannot pair the per-message cap with the wrong
/// aggregate budgets.
#[derive(Clone, Copy, Debug)]
pub(super) struct ContinuationLimits {
    /// Per-message body cap.
    pub(super) max_message_size: NonZeroUsize,
    /// Aggregate budgets across in-flight assemblies.
    pub(super) budgets: AggregateBudgets,
}

/// Check whether accepting `additional_bytes` for `key` would exceed
/// the connection budget or in-flight budget.
///
/// Both budgets are checked against the same `current_total` because,
/// at this layer, all charged bytes are assembly bytes. Callers pass charged
/// totals: retained payload plus [`MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES`] for
/// every live partial, and a new entry's charge includes one overhead. The
/// dimensions are kept separate so future work (streaming body buffers,
/// transport buffering) can diverge them.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::ConnectionBudgetExceeded`] or
/// [`MessageAssemblyError::InFlightBudgetExceeded`] when the respective
/// limit would be exceeded.
pub(super) const fn check_aggregate_budgets(
    key: MessageKey,
    current_total: usize,
    additional_bytes: usize,
    budgets: &AggregateBudgets,
) -> Result<(), MessageAssemblyError> {
    let new_total = current_total.saturating_add(additional_bytes);

    if let Some(limit) = budgets.connection
        && new_total > limit.get()
    {
        return Err(MessageAssemblyError::ConnectionBudgetExceeded {
            key,
            attempted: new_total,
            limit,
        });
    }

    if let Some(limit) = budgets.in_flight
        && new_total > limit.get()
    {
        return Err(MessageAssemblyError::InFlightBudgetExceeded {
            key,
            attempted: new_total,
            limit,
        });
    }

    Ok(())
}

/// Check if accumulated size plus new body would exceed the per-message
/// size limit.
///
/// Returns the new total size on success.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::MessageTooLarge`] when the new total
/// would exceed `max_message_size`.
pub(super) const fn check_size_limit(
    max_message_size: NonZeroUsize,
    key: MessageKey,
    accumulated: usize,
    body_len: usize,
) -> Result<usize, MessageAssemblyError> {
    let Some(new_len) = accumulated.checked_add(body_len) else {
        return Err(MessageAssemblyError::MessageTooLarge {
            key,
            attempted: usize::MAX,
            limit: max_message_size,
        });
    };

    if new_len > max_message_size.get() {
        return Err(MessageAssemblyError::MessageTooLarge {
            key,
            attempted: new_len,
            limit: max_message_size,
        });
    }

    Ok(new_len)
}
