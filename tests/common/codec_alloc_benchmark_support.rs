//! Allocation-specific helpers for codec benchmark execution and test validation.
//!
//! This module defines the allocation baseline types and labelling helpers used
//! by criterion allocation benchmarks, rstest unit tests, and rstest-bdd
//! behavioural tests. It depends on the core benchmark support module for
//! workload definitions.

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
