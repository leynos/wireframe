//! Criterion benchmarks recording allocation baselines for codec wrapping and decode paths.
//!
//! Allocation counts are captured with a counting global allocator in this bench
//! binary. Baseline counts are embedded in benchmark labels as
//! `wrap_allocs_<n>` and `decode_allocs_<n>`.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence},
};

use bytes::{Bytes, BytesMut};
use criterion::{BenchmarkId, Criterion, black_box};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{
    FrameCodec,
    LengthDelimitedDecoder,
    LengthDelimitedFrameCodec,
    examples::{HotlineAdapter, HotlineFrameCodec},
};

#[path = "../tests/common/codec_benchmark_support.rs"]
mod codec_benchmark_support;

#[path = "../tests/common/codec_alloc_benchmark_support.rs"]
mod codec_alloc_benchmark_support;

use codec_alloc_benchmark_support::{AllocationBaseline, allocation_label};
use codec_benchmark_support::{
    BenchmarkWorkload,
    CodecUnderTest,
    LARGE_PAYLOAD_BYTES,
    VALIDATION_ITERATIONS,
    benchmark_workloads,
    measure_decode,
    measure_encode,
    payload_for_class,
};

struct CountingAllocator;

static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_COUNTING_ENABLED: AtomicBool = AtomicBool::new(false);

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

type LengthDelimitedFrameDecoder = LengthDelimitedDecoder;
type HotlineFrameDecoder = HotlineAdapter;

fn record_allocation_if_enabled() {
    if ALLOCATION_COUNTING_ENABLED.load(Ordering::Acquire) {
        ALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

// SAFETY: This allocator forwards all allocation operations directly to
// `System` while incrementing an atomic counter. It does not change pointer
// ownership or layout semantics.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Measure allocations in a scoped window using the process-global counter.
///
/// The result is a global allocation delta during `operation`, not a precise
/// codec-only measurement. It includes allocations from benchmark harness code,
/// closures, iterator state, and any other activity occurring in the window.
/// Treat the value as a noisy relative baseline for comparison across runs.
fn count_allocations<F>(operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(true, Ordering::SeqCst);
    fence(Ordering::SeqCst);

    if let Err(err) = operation() {
        fence(Ordering::SeqCst);
        ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
        panic!("allocation baseline operation failed: {err}");
    }

    fence(Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
    ALLOCATION_COUNT.load(Ordering::SeqCst)
}

enum PreparedDecodeInput {
    LengthDelimited {
        encoded: Bytes,
        decoder: LengthDelimitedFrameDecoder,
    },
    Hotline {
        encoded: Bytes,
        decoder: HotlineFrameDecoder,
    },
}

macro_rules! codec_prepare_decode_input_fn {
    ($fn_name:ident, $codec_type:ty, $decoder_type:ty, $label_str:literal) => {
        fn $fn_name(payload: Bytes) -> Result<(Bytes, $decoder_type), String> {
            let codec = <$codec_type>::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut seed_encoder = codec.encoder();
            let mut encoded = BytesMut::new();
            seed_encoder
                .encode(codec.wrap_payload(payload), &mut encoded)
                .map_err(|err| format!("{} seed encode failed: {err}", $label_str))?;
            Ok((encoded.freeze(), codec.decoder()))
        }
    };
}

codec_prepare_decode_input_fn!(
    prepare_length_delimited_decode_input,
    LengthDelimitedFrameCodec,
    LengthDelimitedFrameDecoder,
    "length-delimited"
);
codec_prepare_decode_input_fn!(
    prepare_hotline_decode_input,
    HotlineFrameCodec,
    HotlineFrameDecoder,
    "hotline"
);

fn prepare_decode_input(workload: BenchmarkWorkload) -> Result<PreparedDecodeInput, String> {
    let payload = payload_for_class(workload.payload_class);
    match workload.codec {
        CodecUnderTest::LengthDelimited => {
            let (encoded, decoder) = prepare_length_delimited_decode_input(payload)?;
            Ok(PreparedDecodeInput::LengthDelimited { encoded, decoder })
        }
        CodecUnderTest::Hotline => {
            let (encoded, decoder) = prepare_hotline_decode_input(payload)?;
            Ok(PreparedDecodeInput::Hotline { encoded, decoder })
        }
    }
}

struct DecodeOps<D: Decoder> {
    codec_name: &'static str,
    frame_payload: fn(&D::Item) -> &[u8],
}

fn run_decode_iterations<D>(
    decoder: &mut D,
    encoded: &Bytes,
    iterations: u64,
    decode_ops: &DecodeOps<D>,
) -> Result<(), String>
where
    D: Decoder,
    D::Error: std::fmt::Display,
{
    for _ in 0..iterations {
        let mut wire = BytesMut::from(encoded.as_ref());
        let frame = decoder
            .decode(&mut wire)
            .map_err(|err| format!("{} decode failed: {err}", decode_ops.codec_name))?
            .ok_or_else(|| format!("{} decode produced no frame", decode_ops.codec_name))?;
        if (decode_ops.frame_payload)(&frame).is_empty() {
            return Err(format!(
                "{} decode produced empty payload",
                decode_ops.codec_name
            ));
        }
    }

    Ok(())
}

fn run_prepared_decode_iterations(
    prepared_decode_input: &mut PreparedDecodeInput,
    iterations: u64,
) -> Result<(), String> {
    match prepared_decode_input {
        PreparedDecodeInput::LengthDelimited { encoded, decoder } => run_decode_iterations(
            decoder,
            encoded,
            iterations,
            &DecodeOps {
                codec_name: "length-delimited",
                frame_payload: LengthDelimitedFrameCodec::frame_payload,
            },
        ),
        PreparedDecodeInput::Hotline { encoded, decoder } => run_decode_iterations(
            decoder,
            encoded,
            iterations,
            &DecodeOps {
                codec_name: "hotline",
                frame_payload: HotlineFrameCodec::frame_payload,
            },
        ),
    }
}

fn benchmark_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/allocations");

    for workload in benchmark_workloads() {
        let wrap_allocations = count_allocations(|| {
            let _ = measure_encode(workload, VALIDATION_ITERATIONS)?;
            Ok(())
        });

        let mut prepared_decode_input = match prepare_decode_input(workload) {
            Ok(prepared_decode_input) => prepared_decode_input,
            Err(err) => panic!("allocation decode baseline setup failed: {err}"),
        };
        let decode_allocations = count_allocations(|| {
            run_prepared_decode_iterations(&mut prepared_decode_input, VALIDATION_ITERATIONS)
        });

        let label = allocation_label(
            workload,
            AllocationBaseline {
                wrap_allocations,
                decode_allocations,
            },
        );

        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter_custom(|iters| {
                let encode = match measure_encode(workload, iters) {
                    Ok(value) => value,
                    Err(err) => panic!("allocation encode benchmark failed: {err}"),
                };
                let decode = match measure_decode(workload, iters) {
                    Ok(value) => value,
                    Err(err) => panic!("allocation decode benchmark failed: {err}"),
                };
                black_box(encode.payload_bytes.saturating_add(decode.payload_bytes));
                encode.elapsed + decode.elapsed
            });
        });
    }

    group.finish();
}

/// Entrypoint for codec allocation baseline benchmarks.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    benchmark_allocations(&mut criterion);
    criterion.final_summary();
}
