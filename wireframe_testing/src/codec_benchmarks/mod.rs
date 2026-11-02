//! Codec benchmark helpers shared by benches and behavioural tests.

pub mod codec_alloc_benchmark_support;
pub mod codec_benchmark_support;
pub mod codec_fragmentation_benchmark_support;
pub mod counting_allocator;
#[cfg(feature = "pipeline-benchmarks")]
pub mod pipeline_benchmark_support;
#[cfg(feature = "pipeline-benchmarks")]
pub mod pipeline_hooks;
#[cfg(feature = "pipeline-benchmarks")]
pub mod pipeline_middleware;
#[cfg(feature = "pipeline-benchmarks")]
pub mod pipeline_outbound;

pub use codec_alloc_benchmark_support::{AllocationBaseline, allocation_label};
pub use codec_benchmark_support::{
    BenchmarkWorkload,
    CodecUnderTest,
    LARGE_PAYLOAD_BYTES,
    Measurement,
    PayloadClass,
    SMALL_PAYLOAD_BYTES,
    VALIDATION_ITERATIONS,
    benchmark_workloads,
    measure_decode,
    measure_encode,
    payload_for_class,
};
pub use codec_fragmentation_benchmark_support::{
    FRAGMENT_PAYLOAD_CAP_BYTES,
    FragmentationOverhead,
    MeasurementExt,
    measure_fragmentation_overhead,
    measure_fragmented_wrap,
    measure_unfragmented_wrap,
};
pub use counting_allocator::{CountingAllocator, count_allocations};
#[cfg(feature = "pipeline-benchmarks")]
pub use pipeline_benchmark_support::{AccessMode, pipeline_label, workload_label};
#[cfg(feature = "pipeline-benchmarks")]
pub use pipeline_hooks::measure_hook_dispatch;
#[cfg(feature = "pipeline-benchmarks")]
pub use pipeline_middleware::measure_middleware_passthrough;
#[cfg(feature = "pipeline-benchmarks")]
pub use pipeline_outbound::{
    OutboundMeasurement,
    measure_framed_copies,
    measure_outbound_encode,
    measure_outbound_serialize,
    measure_outbound_wrap,
};
