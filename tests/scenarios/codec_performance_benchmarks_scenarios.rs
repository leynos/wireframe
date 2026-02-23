//! Scenario tests for codec performance benchmark behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_performance_benchmarks::*;

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Encode and decode matrix covers default and custom codecs"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn codec_matrix_samples(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {}

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Fragmentation overhead sample is recorded"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn fragmentation_overhead(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {}

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Allocation baseline labels include wrap and decode counters"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn allocation_labels(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {}
