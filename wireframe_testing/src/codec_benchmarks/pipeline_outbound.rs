//! Outbound encode measurement for the zero-copy benchmark matrix.
//!
//! The stage is the two-step `Serializer::serialize` then `wrap_payload`
//! hand-off performed by `encode_message_frame`. The migration removes the seam
//! between the two steps, so the passes are reported separately: a single
//! combined number would hide exactly the change being measured.
//!
//! The harness serializer mirrors bincode's behaviour rather than a naive
//! vector. `bincode::encode_to_vec` sizes its output first and allocates
//! exactly, so the vector's length equals its capacity and `Bytes::from` keeps
//! the allocation. Copying that here keeps the hand-off honest instead of
//! manufacturing a copy the default serializer does not make.
//!
//! `wrap_payload` produces a frame; it does not write into a transport buffer.
//! The third copy seam therefore belongs to the codec's `Encoder::encode`, and
//! [`measure_framed_copies`] probes it separately. Reporting the framing copy
//! from the wrap pass would attribute a cost to a stage that never incurs it.

use std::{error::Error, hint::black_box, time::Instant};

use wireframe::{
    app::benchmark_encode_message_frame,
    benchmark_counters::{self, CopiedBytes},
    codec::{FrameCodec, LengthDelimitedFrameCodec, examples::HotlineFrameCodec},
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

use super::{
    codec_benchmark_support::{
        BenchmarkWorkload,
        CodecUnderTest,
        LARGE_PAYLOAD_BYTES,
        Measurement,
        measure_encode,
        payload_for_class,
    },
    pipeline_benchmark_support::empty_measurement,
};

/// Serializer that materializes a message into an owned vector.
struct VectorSerializer;

/// Message whose serialized form is a fixed payload.
struct PayloadMessage {
    /// Bytes the serializer writes into its owned output vector.
    payload: Vec<u8>,
}

impl EncodeWith<VectorSerializer> for PayloadMessage {
    fn encode_with(
        &self,
        _serializer: &VectorSerializer,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let mut bytes = Vec::with_capacity(self.payload.len());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
}

impl Serializer for VectorSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
        Self: Sized,
    {
        value.encode_with(self)
    }

    fn deserialize<M>(&self, _bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
        Self: Sized,
    {
        Err(Box::new(std::io::Error::other(
            "deserialization is not used by this harness",
        )))
    }
}

/// Serializer-only and serializer-plus-frame costs for one outbound workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboundMeasurement {
    /// Cost of `Serializer::serialize` alone.
    pub serialize_only: Measurement,
    /// Cost of `Serializer::serialize` followed by `FrameCodec::wrap_payload`.
    pub serialize_and_wrap: Measurement,
    /// Per-operation copied bytes for the serializer-only pass.
    pub serialize_only_copies: CopiedBytes,
    /// Per-operation copied bytes for the serialize-and-wrap pass.
    pub serialize_and_wrap_copies: CopiedBytes,
}

/// Reads the copied-byte counters as a per-operation average.
fn copied_per_operation(operations: u64) -> CopiedBytes {
    benchmark_counters::snapshot()
        .per_operation(operations)
        .unwrap_or_default()
}

/// Builds the message the outbound passes serialize.
fn payload_message(workload: BenchmarkWorkload) -> PayloadMessage {
    PayloadMessage {
        payload: payload_for_class(workload.payload_class).to_vec(),
    }
}

/// Measures the serializer-only pass for one workload.
///
/// This is the seam without `wrap_payload`, so it isolates the cost the
/// migration's serializer amendment would change.
///
/// # Errors
///
/// Returns a description when serialization fails.
pub fn measure_outbound_serialize(
    workload: BenchmarkWorkload,
    iterations: u64,
) -> Result<Measurement, String> {
    if iterations == 0 {
        return Ok(empty_measurement());
    }
    let payload_len = workload.payload_class.len() as u64;
    let message = payload_message(workload);
    measure_serializer_only(&VectorSerializer, &message, payload_len, iterations)
}

/// Measures the serializer-only pass.
fn measure_serializer_only(
    serializer: &VectorSerializer,
    message: &PayloadMessage,
    payload_len: u64,
    iterations: u64,
) -> Result<Measurement, String> {
    benchmark_counters::reset();
    let started = Instant::now();
    for _ in 0..iterations {
        let bytes = serializer
            .serialize(message)
            .map_err(|error| format!("serializer failed: {error}"))?;
        // This pass calls the serializer directly rather than going through a
        // production boundary, so it records the materialization itself. The
        // serialize-and-wrap pass must not repeat this: it reaches
        // `encode_message_frame`, which already records.
        benchmark_counters::record_serialized(bytes.len());
        // Hand the whole buffer to `black_box`, not just its length. Observing
        // only the length lets the optimizer fold it to a constant and delete
        // both the allocation and the fill, which measures the loop, not the
        // serializer.
        let _ = black_box(bytes.as_slice());
    }
    Ok(Measurement {
        operations: iterations,
        payload_bytes: iterations.saturating_mul(payload_len),
        elapsed: started.elapsed(),
    })
}

/// Measures the serializer-plus-`wrap_payload` pass for one workload.
///
/// This is the seam the migration keeps; pairing it with
/// [`measure_outbound_serialize`] isolates the cost of `wrap_payload` alone.
///
/// # Errors
///
/// Returns a description when serialization or framing fails.
pub fn measure_outbound_wrap(
    workload: BenchmarkWorkload,
    iterations: u64,
) -> Result<Measurement, String> {
    if iterations == 0 {
        return Ok(empty_measurement());
    }
    let payload_len = workload.payload_class.len() as u64;
    let message = payload_message(workload);
    let serializer = VectorSerializer;
    benchmark_counters::reset();
    // Each arm builds its codec before starting the clock. Codec construction
    // is setup, and timing it would bias the first Criterion samples, where the
    // iteration count is still small.
    let elapsed = match workload.codec {
        CodecUnderTest::LengthDelimited => {
            let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let started = Instant::now();
            for _ in 0..iterations {
                let frame = benchmark_encode_message_frame(&serializer, &codec, &message)
                    .map_err(|error| format!("length-delimited encode failed: {error}"))?;
                // The payload slice escapes so the serializer's fill cannot be
                // optimized away; observing its length alone would allow that.
                let _ = black_box(LengthDelimitedFrameCodec::frame_payload(&frame));
            }
            started.elapsed()
        }
        CodecUnderTest::Hotline => {
            let codec = HotlineFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
            let started = Instant::now();
            for _ in 0..iterations {
                let frame = benchmark_encode_message_frame(&serializer, &codec, &message)
                    .map_err(|error| format!("hotline encode failed: {error}"))?;
                // See the length-delimited arm: the slice, not its length,
                // keeps the serializer's fill observable.
                let _ = black_box(HotlineFrameCodec::frame_payload(&frame));
            }
            started.elapsed()
        }
    };
    Ok(Measurement {
        operations: iterations,
        payload_bytes: iterations.saturating_mul(payload_len),
        elapsed,
    })
}

/// Measures the codec framing copy for one workload.
///
/// The probe runs the codec's own `wrap_payload` then `Encoder::encode` path,
/// which is where the payload is copied into the transport write buffer. It
/// reports zero `serialized` and zero `handoff` by construction, so a non-zero
/// value in either field means the probe has drifted onto a different seam.
///
/// # Errors
///
/// Returns a description when the selected codec rejects the frame.
///
/// # Examples
///
/// ```ignore
/// let copies = measure_framed_copies(workload, 16)?;
/// assert_eq!(copies.framed, 16 * workload.payload_class.len() as u64);
/// ```
pub fn measure_framed_copies(
    workload: BenchmarkWorkload,
    iterations: u64,
) -> Result<CopiedBytes, String> {
    if iterations == 0 {
        return Ok(CopiedBytes::default());
    }

    // `measure_encode` builds its payload and codec before timing but after
    // this reset. Neither step touches a counted seam: the payload is built as
    // `Bytes` directly, and only `Encoder::encode` records. Resetting here
    // therefore isolates the framing copy instead of counting setup work.
    benchmark_counters::reset();
    let measurement = measure_encode(workload, iterations)?;
    Ok(copied_per_operation(measurement.operations))
}

/// Measures the outbound encode stage for one workload.
///
/// The serializer and the codec are constructed once, outside the timer. The
/// two passes are reported separately because the migration removes the seam
/// between them; a single combined number would hide that change.
///
/// # Errors
///
/// Returns a description when serialization or framing fails.
///
/// # Examples
///
/// ```ignore
/// let measurement = measure_outbound_encode(workload, 16)?;
/// assert_eq!(measurement.serialize_only.operations, 16);
/// ```
pub fn measure_outbound_encode(
    workload: BenchmarkWorkload,
    iterations: u64,
) -> Result<OutboundMeasurement, String> {
    if iterations == 0 {
        return Ok(OutboundMeasurement {
            serialize_only: empty_measurement(),
            serialize_and_wrap: empty_measurement(),
            serialize_only_copies: CopiedBytes::default(),
            serialize_and_wrap_copies: CopiedBytes::default(),
        });
    }

    let payload_len = workload.payload_class.len() as u64;
    let message = payload_message(workload);
    let serializer = VectorSerializer;

    let serialize_only = measure_serializer_only(&serializer, &message, payload_len, iterations)?;
    let serialize_only_copies = copied_per_operation(iterations);

    let serialize_and_wrap = measure_outbound_wrap(workload, iterations)?;
    let serialize_and_wrap_copies = copied_per_operation(iterations);

    Ok(OutboundMeasurement {
        serialize_only,
        serialize_and_wrap,
        serialize_only_copies,
        serialize_and_wrap_copies,
    })
}

#[cfg(test)]
mod tests {
    //! Tests for the outbound encode helpers.

    use serial_test::serial;

    use super::*;
    use crate::codec_benchmarks::{CodecUnderTest, PayloadClass, SMALL_PAYLOAD_BYTES};

    // The copied-byte counters are process-global, and these helpers reset
    // them. Tests in this binary share that state with unrelated code, so the
    // per-seam counts are asserted in `wireframe_testing/tests/copied_bytes.rs`
    // instead, where the whole test binary is dedicated to counter windows.
    // What is checked here is the helper's own contract: both passes run every
    // iteration, and an empty batch does no work.
    /// Small length-delimited workload used by the helper tests.
    fn small_workload() -> BenchmarkWorkload {
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Small,
        }
    }

    #[test]
    #[serial]
    fn outbound_encode_reports_both_passes() {
        let measurement =
            measure_outbound_encode(small_workload(), 4).expect("outbound measurement");
        assert_eq!(measurement.serialize_only.operations, 4);
        assert_eq!(measurement.serialize_and_wrap.operations, 4);
        // Both passes traverse the payload, so the recorded byte totals are
        // four payloads each. `copied_bytes.rs` asserts the per-seam split.
        assert_eq!(
            measurement.serialize_only.payload_bytes,
            4 * SMALL_PAYLOAD_BYTES as u64
        );
        assert_eq!(
            measurement.serialize_and_wrap.payload_bytes,
            4 * SMALL_PAYLOAD_BYTES as u64
        );
    }

    #[test]
    fn an_empty_batch_reports_zero_work_on_both_passes() {
        let measurement = measure_outbound_encode(small_workload(), 0).expect("empty measurement");
        assert_eq!(measurement.serialize_only.operations, 0);
        assert_eq!(measurement.serialize_and_wrap.operations, 0);
    }
}
