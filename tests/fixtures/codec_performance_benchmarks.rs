//! `CodecPerformanceBenchmarksWorld` fixture for codec benchmark behaviour tests.

use rstest::fixture;

#[path = "../common/codec_benchmark_support.rs"]
mod codec_benchmark_support;

use codec_benchmark_support::{
    AllocationBaseline,
    FRAGMENT_PAYLOAD_CAP_BYTES,
    FragmentationOverhead,
    VALIDATION_ITERATIONS,
    allocation_label,
    benchmark_workloads,
    measure_decode,
    measure_encode,
    measure_fragmentation_overhead,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

#[derive(Debug, Default)]
pub struct CodecPerformanceBenchmarksWorld {
    iterations: u64,
    matrix_samples: usize,
    fragmentation_overhead: Option<FragmentationOverhead>,
    allocation_labels: Vec<String>,
}

#[fixture]
pub fn codec_performance_benchmarks_world() -> CodecPerformanceBenchmarksWorld {
    CodecPerformanceBenchmarksWorld {
        iterations: VALIDATION_ITERATIONS,
        ..CodecPerformanceBenchmarksWorld::default()
    }
}

impl CodecPerformanceBenchmarksWorld {
    pub fn configure_iterations(&mut self, iterations: u64) -> TestResult {
        if iterations == 0 {
            return Err("iteration count must be greater than zero".into());
        }

        self.iterations = iterations;
        Ok(())
    }

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

    pub fn assert_fragmentation_overhead_available(&self) -> TestResult {
        let Some(overhead) = self.fragmentation_overhead else {
            return Err("fragmentation overhead sample was not recorded".into());
        };
        let _ = overhead.unfragmented.nanos_per_op();
        let _ = overhead.fragmented.nanos_per_op();

        if overhead.fragmented.operations < overhead.unfragmented.operations {
            return Err(
                "fragmented sample recorded fewer operations than unfragmented sample".into(),
            );
        }

        if overhead.nanos_ratio().is_none() {
            return Err("fragmentation overhead ratio is unavailable".into());
        }

        Ok(())
    }

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
