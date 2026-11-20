//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

mod assembly;
mod assembly_dispatch;
mod backpressure;
mod buffered_assembly;
mod core;
mod decode;
mod reassembly;
mod response;
mod response_pushes;
mod stream_assembly;
pub(crate) use core::{DeserFailureTracker, ResponseContext};

pub(crate) use assembly::{
    AssemblyRuntime,
    FrameOutcome,
    assemble_if_needed,
    new_message_assembly_state,
    purge_expired_assemblies,
};
pub(crate) use backpressure::{
    MemoryPressureAction,
    active_aggregate_limit_bytes,
    apply_memory_pressure,
    evaluate_memory_pressure_with_retained,
    resolve_effective_budgets,
};
pub(crate) use decode::decode_envelope;
pub(crate) use reassembly::reassemble_if_needed;
pub(crate) use response::{Dispatch, forward_response, forward_service_response};

#[cfg(all(test, not(loom)))]
mod assembly_tests;
#[cfg(all(test, not(loom)))]
mod backpressure_tests;
#[cfg(all(test, not(loom)))]
mod completion_tests;
#[cfg(all(test, not(loom)))]
mod delivery_span_tests;
#[cfg(all(test, not(loom)))]
mod response_pushes_tests;
#[cfg(all(test, not(loom)))]
mod terminator_tests;
#[cfg(all(test, not(loom)))]
mod tests;
