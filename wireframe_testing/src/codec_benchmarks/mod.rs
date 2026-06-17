//! Codec benchmark helpers shared by benches and behavioural tests.

pub mod codec_alloc_benchmark_support;
pub mod codec_benchmark_support;
pub mod codec_fragmentation_benchmark_support;

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
