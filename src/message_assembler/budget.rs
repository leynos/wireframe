//! Budget enforcement helpers for message assembly.
//!
//! This module provides aggregate budget checks applied during frame
//! acceptance. Per-message size limits are handled by the existing
//! `check_size_limit` function in [`super::state`]; this module adds
//! per-connection and in-flight aggregate budget enforcement.

use std::num::NonZeroUsize;

use super::{MessageKey, error::MessageAssemblyError};

/// Paired connection and in-flight budget limits.
///
/// Bundled into a struct so call-sites pass a single value instead of two
/// `Option<NonZeroUsize>` parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct AggregateBudgets {
    pub(super) connection: Option<NonZeroUsize>,
    pub(super) in_flight: Option<NonZeroUsize>,
}

/// Check whether accepting `additional_bytes` for `key` would exceed
/// the connection budget or in-flight budget.
///
/// Both budgets are checked against the same `current_total` because,
/// at this layer, all buffered bytes are assembly bytes. The dimensions
/// are kept separate so future work (streaming body buffers, transport
/// buffering) can diverge them.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::ConnectionBudgetExceeded`] or
/// [`MessageAssemblyError::InFlightBudgetExceeded`] when the respective
/// limit would be exceeded.
pub(super) fn check_aggregate_budgets(
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
pub(super) fn check_size_limit(
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
