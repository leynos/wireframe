//! Helpers for fragment reassembly.

use std::io;

use super::core::DeserFailureTracker;
use crate::app::{
    Envelope,
    fragmentation_state::{FragmentProcessError, FragmentationState},
};

/// Attempt to reassemble a potentially fragmented envelope.
pub(crate) fn reassemble_if_needed(
    fragmentation: &mut Option<FragmentationState>,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>> {
    let mut failures = DeserFailureTracker::new(deser_failures, max_deser_failures);

    if let Some(state) = fragmentation.as_mut() {
        let correlation_id = env.correlation_id;
        match state.reassemble(env) {
            Ok(Some(env)) => Ok(Some(env)),
            Ok(None) => Ok(None),
            Err(FragmentProcessError::Decode(err)) => {
                failures.record(correlation_id, "failed to decode fragment header", err)?;
                Ok(None)
            }
            Err(FragmentProcessError::Reassembly(err)) => {
                failures.record(correlation_id, "fragment reassembly failed", err)?;
                Ok(None)
            }
        }
    } else {
        Ok(Some(env))
    }
}
