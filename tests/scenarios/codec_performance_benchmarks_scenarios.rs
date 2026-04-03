//! Scenario tests for codec performance benchmark behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_performance_benchmarks::*;

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Encode and decode matrix covers default and custom codecs"
)]
fn codec_matrix_samples(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {
    let _ = codec_performance_benchmarks_world;
}

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Fragmentation overhead sample is recorded"
)]
fn fragmentation_overhead(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {
    let _ = codec_performance_benchmarks_world;
}

#[scenario(
    path = "tests/features/codec_performance_benchmarks.feature",
    name = "Allocation baseline labels include wrap and decode counters"
)]
fn allocation_labels(codec_performance_benchmarks_world: CodecPerformanceBenchmarksWorld) {
    let _ = codec_performance_benchmarks_world;
}
