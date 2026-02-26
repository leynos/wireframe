//! Allocation-specific helpers for codec benchmark execution and test validation.
//!
//! This module defines the allocation baseline types and labelling helpers used
//! by criterion allocation benchmarks, rstest unit tests, and rstest-bdd
//! behavioural tests.
//!
//! # Layout coupling
//!
//! This module references `codec_benchmark_support` via `super::` and therefore
//! must be declared as a sibling `mod` in the same parent scope. Every current
//! consumer already satisfies this constraint because the `#[path]` inclusion
//! pattern compiles both modules into the same crate root. If the helpers are
//! ever reused outside the current test/bench tree, consider introducing a
//! `tests/common/mod.rs` hierarchy with normal `mod`/`pub` wiring instead.

use super::codec_benchmark_support::BenchmarkWorkload;

/// Allocation baseline values captured during allocation benchmark setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationBaseline {
    /// Baseline allocation count observed while wrapping payloads.
    pub wrap_allocations: usize,
    /// Baseline allocation count observed while decoding payloads.
    pub decode_allocations: usize,
}

/// Label fragment embedding allocation baseline counts.
#[must_use]
pub fn allocation_label(workload: BenchmarkWorkload, baseline: AllocationBaseline) -> String {
    format!(
        "{}_wrap_allocs_{}_decode_allocs_{}",
        workload.label(),
        baseline.wrap_allocations,
        baseline.decode_allocations
    )
}
