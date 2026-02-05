//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

mod core;
mod reassembly;
mod response;

pub(crate) use core::ResponseContext;

pub(crate) use reassembly::reassemble_if_needed;
pub(crate) use response::forward_response;

#[cfg(all(test, not(loom)))]
mod tests;
