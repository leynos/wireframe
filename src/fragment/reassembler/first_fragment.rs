//! Admission for a message's first fragment.

use std::collections::hash_map::VacantEntry;

use super::{
    FirstFragment,
    MessageId,
    PartialMessage,
    ReassembledMessage,
    budget::ReassemblyBudget,
};
use crate::fragment::{FragmentSeries, ReassemblyError};

/// Validate limits before allocating state for a previously unseen message.
pub(super) fn push_first_fragment(
    budget: &mut ReassemblyBudget,
    buffered_partials: usize,
    vacant: VacantEntry<'_, MessageId, PartialMessage>,
    fragment: &FirstFragment<'_>,
) -> Result<Option<ReassembledMessage>, ReassemblyError> {
    let message_id = fragment.header.message_id();
    let (series, is_complete) = FragmentSeries::start_with_first(fragment.header)?;
    budget.assert_within_message_limit(message_id, fragment.payload.len())?;

    if is_complete {
        return Ok(Some(ReassembledMessage::new(
            message_id,
            fragment.payload.to_vec(),
        )));
    }

    budget.assert_within_partial_limit(buffered_partials)?;
    budget.add_bytes(fragment.payload.len())?;
    vacant.insert(PartialMessage::new(series, fragment.payload, fragment.now));
    Ok(None)
}
