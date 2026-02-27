//! `CodecPerformanceBenchmarksWorld` fixture for codec benchmark behaviour tests.

use rstest::fixture;

#[path = "../common/codec_benchmark_support.rs"]
mod codec_benchmark_support;

#[path = "../common/codec_fragmentation_benchmark_support.rs"]
mod codec_fragmentation_benchmark_support;

#[path = "../common/codec_alloc_benchmark_support.rs"]
mod codec_alloc_benchmark_support;

use codec_alloc_benchmark_support::{AllocationBaseline, allocation_label};
use codec_benchmark_support::{
    VALIDATION_ITERATIONS,
    benchmark_workloads,
    measure_decode,
    measure_encode,
};
use codec_fragmentation_benchmark_support::{
    FRAGMENT_PAYLOAD_CAP_BYTES,
    FragmentationOverhead,
    MeasurementExt as _,
    measure_fragmentation_overhead,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// Holds benchmark configuration and captured state for codec performance tests.
///
/// The world stores:
/// - `iterations`: the currently configured iteration count for benchmark sampling.
/// - `matrix_samples`: how many codec/payload matrix samples were collected.
/// - `fragmentation_overhead`: the most recent fragmentation overhead summary.
/// - `allocation_labels`: generated allocation baseline labels for assertions.
#[derive(Debug, Default)]
pub struct CodecPerformanceBenchmarksWorld {
    /// Iteration count used when running benchmark helper measurements.
    iterations: u64,
    /// Number of workload samples collected by the codec matrix run.
    matrix_samples: usize,
    /// Last recorded fragmentation overhead sample, if one was captured.
    fragmentation_overhead: Option<FragmentationOverhead>,
    /// Allocation baseline labels generated for each workload.
    allocation_labels: Vec<String>,
}

/// Build the codec performance fixture world with validation iterations.
///
/// Returns a default world configured with `iterations` set to
/// `VALIDATION_ITERATIONS`.
#[fixture]
pub fn codec_performance_benchmarks_world() -> CodecPerformanceBenchmarksWorld {
    CodecPerformanceBenchmarksWorld {
        iterations: VALIDATION_ITERATIONS,
        ..CodecPerformanceBenchmarksWorld::default()
    }
}

impl CodecPerformanceBenchmarksWorld {
    /// Configure the iteration count used by benchmark helper runs.
    ///
    /// # Arguments
    /// - `iterations`: Number of iterations to execute; must be greater than zero.
    ///
    /// # Returns
    /// - `Ok(())` when the iteration count is accepted and stored.
    /// - `Err` when `iterations == 0`.
    pub fn configure_iterations(&mut self, iterations: u64) -> TestResult {
        if iterations == 0 {
            return Err("iteration count must be greater than zero".into());
        }

        self.iterations = iterations;
        Ok(())
    }

    /// Run encode/decode benchmark samples for every workload in the matrix.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: `iterations` must be configured to a value greater than zero.
    ///
    /// # Returns
    /// - `Ok(())` when all workloads run and operation counts match the configured `iterations`.
    /// - `Err` when iterations were not configured, measurement fails, or an encode/decode
    ///   operation count mismatch is detected.
    pub fn run_codec_matrix_samples(&mut self) -> TestResult {
        if self.iterations == 0 {
            return Err("iteration count must be configured before running matrix samples".into());
        }

        self.matrix_samples = 0;
        for workload in benchmark_workloads() {
            let encode = measure_encode(workload, self.iterations)?;
            let decode = measure_decode(workload, self.iterations)?;

            if encode.operations != self.iterations {
                return Err(format!("encode operations mismatch for {}", workload.label()).into());
            }
            if decode.operations != self.iterations {
                return Err(format!("decode operations mismatch for {}", workload.label()).into());
            }
            self.matrix_samples += 1;
        }

        Ok(())
    }

    /// Run one fragmentation-overhead sample for the large payload class.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: `iterations` must be configured to a value greater than zero.
    ///
    /// # Returns
    /// - `Ok(())` when fragmentation overhead is measured and stored.
    /// - `Err` when iterations were not configured or measurement fails.
    pub fn run_fragmentation_sample(&mut self) -> TestResult {
        if self.iterations == 0 {
            return Err("iteration count must be configured before fragmentation samples".into());
        }

        self.fragmentation_overhead = Some(measure_fragmentation_overhead(
            codec_benchmark_support::PayloadClass::Large,
            self.iterations,
            FRAGMENT_PAYLOAD_CAP_BYTES,
        )?);

        Ok(())
    }

    /// Build allocation labels for each workload using deterministic baselines.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: `iterations` must be configured to a value greater than zero.
    ///
    /// # Returns
    /// - `Ok(())` when labels are generated for all workloads.
    /// - `Err` when iterations were not configured.
    pub fn build_allocation_labels(&mut self) -> TestResult {
        if self.iterations == 0 {
            return Err(
                "iteration count must be configured before allocation label generation".into(),
            );
        }

        self.allocation_labels.clear();
        for (index, workload) in benchmark_workloads().iter().copied().enumerate() {
            let baseline = AllocationBaseline {
                wrap_allocations: index + 1,
                decode_allocations: (index + 1) * 2,
            };
            self.allocation_labels
                .push(allocation_label(workload, baseline));
        }

        Ok(())
    }

    /// Assert that matrix sampling covered the full benchmark workload matrix.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: run `run_codec_matrix_samples` first to populate `matrix_samples`.
    ///
    /// # Returns
    /// - `Ok(())` when `matrix_samples` equals `benchmark_workloads().len()`.
    /// - `Err` when the recorded sample count does not match workload count.
    pub fn assert_matrix_samples_cover_workloads(&self) -> TestResult {
        if self.matrix_samples != benchmark_workloads().len() {
            return Err(format!(
                "expected {} workload samples, found {}",
                benchmark_workloads().len(),
                self.matrix_samples
            )
            .into());
        }

        Ok(())
    }

    /// Assert that fragmentation overhead was captured and contains valid
    /// metrics.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: run `run_fragmentation_sample` first to populate `fragmentation_overhead`.
    ///
    /// # Returns
    /// - `Ok(())` when an overhead sample is present with usable nanos-per-op values,
    ///   non-decreasing operation count, and an available ratio.
    /// - `Err` when no sample exists or any validation check fails.
    pub fn assert_fragmentation_overhead_available(&self) -> TestResult {
        let Some(overhead) = self.fragmentation_overhead else {
            return Err("fragmentation overhead sample was not recorded".into());
        };
        if overhead.unfragmented.nanos_per_op().is_none() {
            return Err("unfragmented nanos-per-op metric is unavailable".into());
        }
        if overhead.fragmented.nanos_per_op().is_none() {
            return Err("fragmented nanos-per-op metric is unavailable".into());
        }

        if overhead.fragmented.operations != overhead.unfragmented.operations {
            return Err(format!(
                "operation count mismatch: fragmented={}, unfragmented={}",
                overhead.fragmented.operations, overhead.unfragmented.operations
            )
            .into());
        }

        if overhead.nanos_ratio().is_none() {
            return Err("fragmentation overhead ratio is unavailable".into());
        }

        Ok(())
    }

    /// Assert that generated allocation labels include both counter fragments.
    ///
    /// # Arguments
    /// - None.
    /// - Preconditions: run `build_allocation_labels` first to populate `allocation_labels`.
    ///
    /// # Returns
    /// - `Ok(())` when labels are present and each contains `wrap_allocs_` and `decode_allocs_`.
    /// - `Err` when no labels were generated or any label omits required counters.
    pub fn assert_allocation_labels_include_counters(&self) -> TestResult {
        if self.allocation_labels.is_empty() {
            return Err("allocation labels were not generated".into());
        }

        for label in &self.allocation_labels {
            if !label.contains("wrap_allocs_") || !label.contains("decode_allocs_") {
                return Err(format!("allocation label missing counters: {label}").into());
            }
        }

        Ok(())
    }
}
