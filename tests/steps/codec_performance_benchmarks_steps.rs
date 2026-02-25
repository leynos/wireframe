//! Step definitions for codec benchmark behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_performance_benchmarks::{CodecPerformanceBenchmarksWorld, TestResult};

#[given("codec benchmark validation uses {iterations:u64} iterations")]
fn given_codec_benchmark_iterations(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
    iterations: u64,
) -> TestResult {
    codec_performance_benchmarks_world.configure_iterations(iterations)
}

#[when("encode and decode benchmark samples run across the codec matrix")]
fn when_codec_matrix_samples_run(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.run_codec_matrix_samples()
}

#[then("encode and decode benchmark samples report all workloads")]
fn then_codec_matrix_samples_cover_workloads(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.assert_matrix_samples_cover_workloads()
}

#[when("fragmented and unfragmented wrapping samples are executed")]
fn when_fragmentation_samples_run(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.run_fragmentation_sample()
}

#[then("fragmentation overhead metrics are available")]
fn then_fragmentation_metrics_are_available(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.assert_fragmentation_overhead_available()
}

#[when("allocation baseline labels are generated for codec workloads")]
fn when_allocation_labels_generated(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.build_allocation_labels()
}

#[then("allocation baseline labels include wrap and decode counters")]
fn then_allocation_labels_include_counters(
    codec_performance_benchmarks_world: &mut CodecPerformanceBenchmarksWorld,
) -> TestResult {
    codec_performance_benchmarks_world.assert_allocation_labels_include_counters()
}
