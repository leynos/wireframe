//! Helpers for fragment reassembly.

use std::io;

use super::core::DeserFailureTracker;
use crate::app::{
    Envelope,
    codec_driver::FramePipeline,
    fragmentation_state::FragmentProcessError,
};

/// Attempt to reassemble a potentially fragmented envelope.
pub(crate) fn reassemble_if_needed(
    pipeline: &mut FramePipeline,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>> {
    let mut failures = DeserFailureTracker::new(deser_failures, max_deser_failures);
    let correlation_id = env.correlation_id;

    let Some(state) = pipeline.fragmentation_mut() else {
        return Ok(Some(env));
    };

    handle_reassembly_result(state.reassemble(env), &mut failures, correlation_id)
}

fn handle_reassembly_result(
    result: Result<Option<Envelope>, FragmentProcessError>,
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
) -> io::Result<Option<Envelope>> {
    match result {
        Ok(envelope) => Ok(envelope),
        Err(FragmentProcessError::Decode(err)) => {
            failures.record(correlation_id, "failed to decode fragment header", err)?;
            Ok(None)
        }
        Err(FragmentProcessError::Reassembly(err)) => {
            failures.record(correlation_id, "fragment reassembly failed", err)?;
            Ok(None)
        }
    }
}
