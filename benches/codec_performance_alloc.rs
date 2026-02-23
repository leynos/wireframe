//! Criterion benchmarks recording allocation baselines for codec wrapping and decode paths.
//!
//! Allocation counts are captured with a counting global allocator in this bench
//! binary. Baseline counts are embedded in benchmark labels as
//! `wrap_allocs_<n>` and `decode_allocs_<n>`.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use criterion::{BenchmarkId, Criterion, black_box};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{FrameCodec, LengthDelimitedFrameCodec, examples::HotlineFrameCodec};

const SMALL_PAYLOAD_BYTES: usize = 32;
const LARGE_PAYLOAD_BYTES: usize = 64 * 1024;
const VALIDATION_ITERATIONS: u64 = 16;

#[derive(Debug, Clone, Copy)]
enum CodecUnderTest {
    LengthDelimited,
    Hotline,
}

impl CodecUnderTest {
    const fn label(self) -> &'static str {
        match self {
            Self::LengthDelimited => "length_delimited",
            Self::Hotline => "hotline",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PayloadClass {
    Small,
    Large,
}

impl PayloadClass {
    const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Large => "large",
        }
    }

    const fn len(self) -> usize {
        match self {
            Self::Small => SMALL_PAYLOAD_BYTES,
            Self::Large => LARGE_PAYLOAD_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BenchmarkWorkload {
    codec: CodecUnderTest,
    payload_class: PayloadClass,
}

impl BenchmarkWorkload {
    fn label(self) -> String { format!("{}_{}", self.codec.label(), self.payload_class.label()) }
}

#[derive(Debug, Clone, Copy)]
struct Measurement {
    payload_bytes: u64,
    elapsed: Duration,
}

struct CountingAllocator;

static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: This allocator forwards all allocation operations directly to
// `System` while incrementing an atomic counter. It does not change pointer
// ownership or layout semantics.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn benchmark_workloads() -> [BenchmarkWorkload; 4] {
    [
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Small,
        },
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Large,
        },
        BenchmarkWorkload {
            codec: CodecUnderTest::Hotline,
            payload_class: PayloadClass::Small,
        },
        BenchmarkWorkload {
            codec: CodecUnderTest::Hotline,
            payload_class: PayloadClass::Large,
        },
    ]
}

fn payload_for_class(class: PayloadClass) -> Bytes {
    let mut payload = Vec::with_capacity(class.len());
    let mut next_byte = 0_u8;
    for _ in 0..class.len() {
        payload.push(next_byte);
        next_byte = if next_byte == 250 { 0 } else { next_byte + 1 };
    }
    Bytes::from(payload)
}

fn measure_encode(workload: BenchmarkWorkload, iterations: u64) -> Result<Measurement, String> {
    let payload = payload_for_class(workload.payload_class);
    let payload_len = payload.len() as u64;

    match workload.codec {
        CodecUnderTest::LengthDelimited => {
            let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut encoder = codec.encoder();
            let mut wire = BytesMut::new();
            let started = Instant::now();
            for _ in 0..iterations {
                wire.clear();
                encoder
                    .encode(codec.wrap_payload(payload.clone()), &mut wire)
                    .map_err(|err| format!("length-delimited encode failed: {err}"))?;
            }
            Ok(Measurement {
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
        CodecUnderTest::Hotline => {
            let codec = HotlineFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut encoder = codec.encoder();
            let mut wire = BytesMut::new();
            let started = Instant::now();
            for _ in 0..iterations {
                wire.clear();
                encoder
                    .encode(codec.wrap_payload(payload.clone()), &mut wire)
                    .map_err(|err| format!("hotline encode failed: {err}"))?;
            }
            Ok(Measurement {
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
    }
}

fn measure_decode(workload: BenchmarkWorkload, iterations: u64) -> Result<Measurement, String> {
    let payload = payload_for_class(workload.payload_class);
    let payload_len = payload.len() as u64;

    match workload.codec {
        CodecUnderTest::LengthDelimited => {
            let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut seed_encoder = codec.encoder();
            let mut encoded = BytesMut::new();
            seed_encoder
                .encode(codec.wrap_payload(payload), &mut encoded)
                .map_err(|err| format!("length-delimited seed encode failed: {err}"))?;
            let encoded = encoded.freeze();

            let mut decoder = codec.decoder();
            let started = Instant::now();
            for _ in 0..iterations {
                let mut wire = BytesMut::from(encoded.as_ref());
                let frame = decoder
                    .decode(&mut wire)
                    .map_err(|err| format!("length-delimited decode failed: {err}"))?
                    .ok_or("length-delimited decode produced no frame")?;
                if LengthDelimitedFrameCodec::frame_payload(&frame).is_empty() {
                    return Err("length-delimited decode produced empty payload".to_string());
                }
            }
            Ok(Measurement {
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
        CodecUnderTest::Hotline => {
            let codec = HotlineFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut seed_encoder = codec.encoder();
            let mut encoded = BytesMut::new();
            seed_encoder
                .encode(codec.wrap_payload(payload), &mut encoded)
                .map_err(|err| format!("hotline seed encode failed: {err}"))?;
            let encoded = encoded.freeze();

            let mut decoder = codec.decoder();
            let started = Instant::now();
            for _ in 0..iterations {
                let mut wire = BytesMut::from(encoded.as_ref());
                let frame = decoder
                    .decode(&mut wire)
                    .map_err(|err| format!("hotline decode failed: {err}"))?
                    .ok_or("hotline decode produced no frame")?;
                if HotlineFrameCodec::frame_payload(&frame).is_empty() {
                    return Err("hotline decode produced empty payload".to_string());
                }
            }
            Ok(Measurement {
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
    }
}

fn count_allocations<F>(operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    let before = ALLOCATION_COUNT.load(Ordering::Relaxed);
    if let Err(err) = operation() {
        panic!("allocation baseline operation failed: {err}");
    }
    let after = ALLOCATION_COUNT.load(Ordering::Relaxed);
    after.saturating_sub(before)
}

fn allocation_label(
    workload: BenchmarkWorkload,
    wrap_allocations: usize,
    decode_allocations: usize,
) -> String {
    format!(
        "{}_wrap_allocs_{}_decode_allocs_{}",
        workload.label(),
        wrap_allocations,
        decode_allocations
    )
}

fn benchmark_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/allocations");

    for workload in benchmark_workloads() {
        let wrap_allocations = count_allocations(|| {
            for _ in 0..VALIDATION_ITERATIONS {
                let _ = measure_encode(workload, 1)?;
            }
            Ok(())
        });

        let decode_allocations = count_allocations(|| {
            for _ in 0..VALIDATION_ITERATIONS {
                let _ = measure_decode(workload, 1)?;
            }
            Ok(())
        });

        let label = allocation_label(workload, wrap_allocations, decode_allocations);

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
