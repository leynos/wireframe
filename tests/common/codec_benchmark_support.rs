//! Shared helpers for codec benchmark execution and test validation.
//!
//! This module defines the benchmark workload matrix, deterministic payload
//! generation, and lightweight measurement helpers used by criterion benches,
//! rstest unit tests, and rstest-bdd behavioural tests.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    codec::{FrameCodec, LengthDelimitedFrameCodec, examples::HotlineFrameCodec},
    fragment::{Fragmenter, encode_fragment_payload},
};

/// Payload size used for small-frame benchmark cases.
pub const SMALL_PAYLOAD_BYTES: usize = 32;
/// Payload size used for large-frame benchmark cases.
pub const LARGE_PAYLOAD_BYTES: usize = 64 * 1024;
/// Fragment payload cap used for fragmentation-overhead benchmarks.
pub const FRAGMENT_PAYLOAD_CAP_BYTES: usize = 1024;
/// Shared iteration count for fast validation probes in tests.
pub const VALIDATION_ITERATIONS: u64 = 16;

/// Codec variants exercised by codec performance benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecUnderTest {
    /// `LengthDelimitedFrameCodec` default framing path.
    LengthDelimited,
    /// `HotlineFrameCodec` custom framing path.
    Hotline,
}

impl CodecUnderTest {
    /// Human-readable benchmark label for this codec.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LengthDelimited => "length_delimited",
            Self::Hotline => "hotline",
        }
    }
}

/// Payload classes exercised by codec performance benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadClass {
    /// Small frame payload used for latency-sensitive paths.
    Small,
    /// Large frame payload used for high-throughput and fragmentation paths.
    Large,
}

impl PayloadClass {
    /// Human-readable benchmark label for this payload class.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Large => "large",
        }
    }

    /// Payload length in bytes for this class.
    #[must_use]
    pub const fn len(self) -> usize {
        match self {
            Self::Small => SMALL_PAYLOAD_BYTES,
            Self::Large => LARGE_PAYLOAD_BYTES,
        }
    }
}

/// A single benchmark workload definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchmarkWorkload {
    /// Codec under test.
    pub codec: CodecUnderTest,
    /// Payload class under test.
    pub payload_class: PayloadClass,
}

impl BenchmarkWorkload {
    /// Stable benchmark id segment for this workload.
    #[must_use]
    pub fn label(self) -> String {
        format!("{}_{}", self.codec.label(), self.payload_class.label())
    }
}

/// Deterministic matrix covering both codecs over both payload classes.
#[must_use]
pub const fn benchmark_workloads() -> [BenchmarkWorkload; 4] {
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

/// Deterministic payload bytes for a payload class.
#[must_use]
pub fn payload_for_class(class: PayloadClass) -> Bytes {
    let mut payload = Vec::with_capacity(class.len());
    let mut next_byte = 0_u8;
    for _ in 0..class.len() {
        payload.push(next_byte);
        next_byte = if next_byte == 250 { 0 } else { next_byte + 1 };
    }
    Bytes::from(payload)
}

/// Time and volume summary for a measured benchmark operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measurement {
    /// Number of operations executed.
    pub operations: u64,
    /// Total payload bytes processed.
    pub payload_bytes: u64,
    /// Elapsed wall-clock duration.
    pub elapsed: Duration,
}

impl Measurement {
    /// Mean nanoseconds per operation.
    #[must_use]
    pub fn nanos_per_op(self) -> Option<u128> {
        let operations = u32::try_from(self.operations).ok()?;
        self.elapsed
            .checked_div(operations)
            .map(|duration| duration.as_nanos())
    }
}

/// Fragmentation-overhead summary comparing fragmented and unfragmented paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentationOverhead {
    /// Measurement for unfragmented payload wrapping.
    pub unfragmented: Measurement,
    /// Measurement for fragmented payload wrapping.
    pub fragmented: Measurement,
}

impl FragmentationOverhead {
    /// Ratio of fragmented to unfragmented mean nanoseconds per operation.
    #[must_use]
    pub fn nanos_ratio(self) -> Option<f64> {
        if self.fragmented.operations == 0 || self.unfragmented.operations == 0 {
            return None;
        }
        if self.unfragmented.elapsed.is_zero() {
            return None;
        }
        Some(
            self.fragmented
                .elapsed
                .div_duration_f64(self.unfragmented.elapsed),
        )
    }
}

/// Allocation baseline values captured during allocation benchmark setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationBaseline {
    /// Baseline allocation count observed while wrapping payloads.
    pub wrap_allocations: usize,
    /// Baseline allocation count observed while decoding payloads.
    pub decode_allocations: usize,
}

/// Label fragment embedding allocation baseline counts.
#[must_use]
pub fn allocation_label(workload: BenchmarkWorkload, baseline: AllocationBaseline) -> String {
    format!(
        "{}_wrap_allocs_{}_decode_allocs_{}",
        workload.label(),
        baseline.wrap_allocations,
        baseline.decode_allocations
    )
}

/// Measure encode performance for one workload.
pub fn measure_encode(workload: BenchmarkWorkload, iterations: u64) -> Result<Measurement, String> {
    let payload = payload_for_class(workload.payload_class);
    let payload_len = payload.len() as u64;

    if iterations == 0 {
        return Ok(Measurement {
            operations: 0,
            payload_bytes: 0,
            elapsed: Duration::ZERO,
        });
    }

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
                operations: iterations,
                payload_bytes: iterations * payload_len,
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
                operations: iterations,
                payload_bytes: iterations * payload_len,
                elapsed: started.elapsed(),
            })
        }
    }
}

/// Measure decode performance for one workload.
pub fn measure_decode(workload: BenchmarkWorkload, iterations: u64) -> Result<Measurement, String> {
    if iterations == 0 {
        return Ok(Measurement {
            operations: 0,
            payload_bytes: 0,
            elapsed: Duration::ZERO,
        });
    }

    let payload = payload_for_class(workload.payload_class);
    let payload_len = payload.len() as u64;

    match workload.codec {
        CodecUnderTest::LengthDelimited => {
            measure_decode_length_delimited(payload, payload_len, iterations)
        }
        CodecUnderTest::Hotline => measure_decode_hotline(payload, payload_len, iterations),
    }
}

fn measure_decode_length_delimited(
    payload: Bytes,
    payload_len: u64,
    iterations: u64,
) -> Result<Measurement, String> {
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
        operations: iterations,
        payload_bytes: iterations * payload_len,
        elapsed: started.elapsed(),
    })
}

fn measure_decode_hotline(
    payload: Bytes,
    payload_len: u64,
    iterations: u64,
) -> Result<Measurement, String> {
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
        operations: iterations,
        payload_bytes: iterations * payload_len,
        elapsed: started.elapsed(),
    })
}

/// Measure unfragmented payload wrapping performance for the default codec.
pub fn measure_unfragmented_wrap(payload_class: PayloadClass, iterations: u64) -> Measurement {
    let payload = payload_for_class(payload_class);
    let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
    let started = Instant::now();
    let mut total_bytes = 0_u64;

    for _ in 0..iterations {
        let frame = codec.wrap_payload(payload.clone());
        total_bytes += LengthDelimitedFrameCodec::frame_payload(&frame).len() as u64;
    }

    Measurement {
        operations: iterations,
        payload_bytes: total_bytes,
        elapsed: started.elapsed(),
    }
}

/// Measure fragmented payload wrapping performance for the default codec.
pub fn measure_fragmented_wrap(
    payload_class: PayloadClass,
    iterations: u64,
    fragment_payload_cap: usize,
) -> Result<Measurement, String> {
    let payload = payload_for_class(payload_class);
    let fragment_payload_cap =
        NonZeroUsize::new(fragment_payload_cap).ok_or("fragment payload cap must be non-zero")?;
    let fragmenter = Fragmenter::new(fragment_payload_cap);
    let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
    let started = Instant::now();

    let mut total_operations = 0_u64;
    let mut total_bytes = 0_u64;

    for _ in 0..iterations {
        let batch = fragmenter
            .fragment_bytes(payload.as_ref())
            .map_err(|err| format!("fragment split failed: {err}"))?;
        for fragment in batch.fragments() {
            let encoded = encode_fragment_payload(*fragment.header(), fragment.payload())
                .map_err(|err| format!("fragment payload encode failed: {err}"))?;
            let frame = codec.wrap_payload(Bytes::from(encoded));
            total_bytes += LengthDelimitedFrameCodec::frame_payload(&frame).len() as u64;
            total_operations += 1;
        }
    }

    Ok(Measurement {
        operations: total_operations,
        payload_bytes: total_bytes,
        elapsed: started.elapsed(),
    })
}

/// Measure fragmentation overhead for one payload class.
pub fn measure_fragmentation_overhead(
    payload_class: PayloadClass,
    iterations: u64,
    fragment_payload_cap: usize,
) -> Result<FragmentationOverhead, String> {
    let unfragmented = measure_unfragmented_wrap(payload_class, iterations);
    let fragmented = measure_fragmented_wrap(payload_class, iterations, fragment_payload_cap)?;
    Ok(FragmentationOverhead {
        unfragmented,
        fragmented,
    })
}
