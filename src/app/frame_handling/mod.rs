//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

mod assembly;
mod backpressure;
mod core;
mod decode;
mod reassembly;
mod response;

pub(crate) use core::ResponseContext;

pub(crate) use assembly::{
    AssemblyRuntime,
    assemble_if_needed,
    new_message_assembly_state,
    purge_expired_assemblies,
};
pub(crate) use backpressure::{
    apply_memory_pressure,
    evaluate_memory_pressure,
    resolve_effective_budgets,
};
pub(crate) use decode::decode_envelope;
pub(crate) use reassembly::reassemble_if_needed;
pub(crate) use response::forward_response;

#[cfg(all(test, not(loom)))]
mod assembly_tests;
#[cfg(all(test, not(loom)))]
mod backpressure_tests;
#[cfg(all(test, not(loom)))]
mod tests;
