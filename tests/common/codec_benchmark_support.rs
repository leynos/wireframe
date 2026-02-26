//! Core helpers for codec benchmark execution and test validation.
//!
//! This module defines the benchmark workload matrix, deterministic payload
//! generation, and encode/decode measurement helpers shared by all codec
//! benchmark targets. Fragmentation and allocation helpers live in dedicated
//! sibling modules.

use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{FrameCodec, LengthDelimitedFrameCodec, examples::HotlineFrameCodec};

/// Payload size used for small-frame benchmark cases.
pub const SMALL_PAYLOAD_BYTES: usize = 32;
/// Payload size used for large-frame benchmark cases.
pub const LARGE_PAYLOAD_BYTES: usize = 64 * 1024;
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
            measure_encode_length_delimited(payload, payload_len, iterations)
        }
        CodecUnderTest::Hotline => measure_encode_hotline(payload, payload_len, iterations),
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

macro_rules! codec_measure_encode_fn {
    ($fn_name:ident, $codec_type:ty, $label_str:literal) => {
        fn $fn_name(
            payload: Bytes,
            payload_len: u64,
            iterations: u64,
        ) -> Result<Measurement, String> {
            let codec = <$codec_type>::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut encoder = codec.encoder();
            let mut wire = BytesMut::new();
            let started = Instant::now();
            for _ in 0..iterations {
                wire.clear();
                encoder
                    .encode(codec.wrap_payload(payload.clone()), &mut wire)
                    .map_err(|err| format!("{} encode failed: {err}", $label_str))?;
            }
            Ok(Measurement {
                operations: iterations,
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
    };
}

codec_measure_encode_fn!(
    measure_encode_length_delimited,
    LengthDelimitedFrameCodec,
    "length-delimited"
);
codec_measure_encode_fn!(measure_encode_hotline, HotlineFrameCodec, "hotline");

macro_rules! codec_measure_decode_fn {
    ($fn_name:ident, $codec_type:ty, $label_str:literal, $frame_payload_path:path) => {
        fn $fn_name(
            payload: Bytes,
            payload_len: u64,
            iterations: u64,
        ) -> Result<Measurement, String> {
            let codec = <$codec_type>::new(LARGE_PAYLOAD_BYTES + 4096);
            let mut seed_encoder = codec.encoder();
            let mut encoded = BytesMut::new();
            seed_encoder
                .encode(codec.wrap_payload(payload), &mut encoded)
                .map_err(|err| format!("{} seed encode failed: {err}", $label_str))?;
            let encoded = encoded.freeze();
            let mut decoder = codec.decoder();
            let started = Instant::now();
            for _ in 0..iterations {
                let mut wire = BytesMut::from(encoded.as_ref());
                let frame = decoder
                    .decode(&mut wire)
                    .map_err(|err| format!("{} decode failed: {err}", $label_str))?
                    .ok_or_else(|| format!("{} decode produced no frame", $label_str))?;
                if $frame_payload_path(&frame).is_empty() {
                    return Err(format!("{} decode produced empty payload", $label_str));
                }
            }
            Ok(Measurement {
                operations: iterations,
                payload_bytes: iterations.saturating_mul(payload_len),
                elapsed: started.elapsed(),
            })
        }
    };
}

codec_measure_decode_fn!(
    measure_decode_length_delimited,
    LengthDelimitedFrameCodec,
    "length-delimited",
    LengthDelimitedFrameCodec::frame_payload
);
codec_measure_decode_fn!(
    measure_decode_hotline,
    HotlineFrameCodec,
    "hotline",
    HotlineFrameCodec::frame_payload
);
