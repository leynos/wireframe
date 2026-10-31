//! Criterion benchmarks recording allocation baselines for codec wrapping and decode paths.
//!
//! Allocation counts are captured with a counting global allocator in this bench
//! binary. Baseline counts are embedded in benchmark labels as
//! `wrap_allocs_<n>` and `decode_allocs_<n>`.

#![forbid(unsafe_code)]

use std::hint::black_box;

use bytes::{Bytes, BytesMut};
use criterion::{BenchmarkId, Criterion};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{
    FrameCodec,
    LengthDelimitedDecoder,
    LengthDelimitedFrameCodec,
    examples::{HotlineAdapter, HotlineFrameCodec},
};
use wireframe_bench_alloc::CountingAllocator;
use wireframe_bench_counting::count_allocations;
use wireframe_testing::codec_benchmarks::{
    AllocationBaseline,
    BenchmarkWorkload,
    CodecUnderTest,
    LARGE_PAYLOAD_BYTES,
    VALIDATION_ITERATIONS,
    allocation_label,
    benchmark_workloads,
    measure_decode,
    measure_encode,
    payload_for_class,
};

/// Process allocator used by this benchmark so encode/decode baselines cover
/// the same allocation path as the code under test.
///
/// The type itself lives in the audited boundary crate, because implementing
/// `GlobalAlloc` requires `unsafe`. Only the one-per-binary static belongs here.
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// Decoder type used by the length-delimited workload, kept as an alias so
/// the prepared-input enum and its macro-generated constructor share a clear
/// type-level contract.
type LengthDelimitedFrameDecoder = LengthDelimitedDecoder;
/// Decoder type used by the Hotline workload, paired with the encoded bytes
/// it consumes during each prepared decode iteration.
type HotlineFrameDecoder = HotlineAdapter;

/// Measure allocations in a scoped window, terminating on a failed workload.
///
/// The shared helper reports a failed operation as `Err`; a benchmark that
/// cannot run its workload must stop rather than record a fabricated baseline,
/// so this bench turns that error into a panic. The count itself is a
/// process-global allocation delta during `operation`, not a precise codec-only
/// measurement: it includes allocations from benchmark harness code, closures,
/// iterator state, and any other activity in the window. Treat the value as a
/// noisy relative baseline for comparison across runs.
fn measure_allocations<F>(operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    match count_allocations(operation) {
        Ok(count) => count,
        Err(err) => panic!("allocation baseline operation failed: {err}"),
    }
}

/// Owns encoded input and its stateful decoder so repeated decode measurements
/// reuse the same prepared wire representation without measuring setup work.
enum PreparedDecodeInput {
    /// Prepared length-delimited bytes and decoder state.
    LengthDelimited {
        /// Frozen encoded frame copied into a fresh mutable buffer per decode.
        encoded: Bytes,
        /// Stateful decoder retained across iterations for protocol framing.
        decoder: LengthDelimitedFrameDecoder,
    },
    /// Prepared Hotline bytes and decoder state.
    Hotline {
        /// Frozen encoded Hotline frame used as the source for each iteration.
        encoded: Bytes,
        /// Hotline decoder retained across iterations while each buffer is new.
        decoder: HotlineFrameDecoder,
    },
}

/// Generates a constructor that encodes one payload before a benchmark starts;
/// this keeps allocation measurement focused on decoding and preserves the
/// decoder's ownership of protocol state.
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

/// Builds the codec-specific prepared input and reports seed-encoding failures
/// before timing begins, so setup errors cannot be mistaken for decode results.
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

/// Supplies the codec-specific naming and payload projection needed to apply
/// one generic decode loop to both frame formats.
struct DecodeOps<D: Decoder> {
    /// Label included in errors so a failed iteration identifies its workload.
    codec_name: &'static str,
    /// Extracts the payload to assert that framing yielded usable data.
    frame_payload: fn(&D::Item) -> &[u8],
}

/// Repeatedly decodes independent copies of one frame and checks that each
/// result has a payload; a fresh buffer prevents consumed bytes from leaking
/// between iterations while the decoder state remains owned by the caller.
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

/// Dispatches prepared input to its matching decoder and payload invariant,
/// preserving the codec-specific state while sharing iteration accounting.
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

/// Registers allocation-baseline labels and timed encode/decode measurements;
/// each workload completes its validation count before Criterion iterations
/// begin, keeping labels stable and making regressions comparable across runs.
fn benchmark_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/allocations");

    for workload in benchmark_workloads() {
        let wrap_allocations = measure_allocations(|| {
            let _ = measure_encode(workload, VALIDATION_ITERATIONS)?;
            Ok(())
        });

        let mut prepared_decode_input = match prepare_decode_input(workload) {
            Ok(prepared_decode_input) => prepared_decode_input,
            Err(err) => panic!("allocation decode baseline setup failed: {err}"),
        };
        let decode_allocations = measure_allocations(|| {
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
