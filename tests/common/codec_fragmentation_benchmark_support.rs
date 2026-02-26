//! Fragmentation-specific helpers for codec benchmark execution and test validation.
//!
//! This module defines the fragmentation overhead measurement helpers used by
//! criterion benches, rstest unit tests, and rstest-bdd behavioural tests.
//! It depends on the core benchmark support module for shared types and payload
//! generation.

use std::num::NonZeroUsize;

use bytes::Bytes;
use wireframe::{
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    fragment::{Fragmenter, encode_fragment_payload},
};

use super::codec_benchmark_support::{
    LARGE_PAYLOAD_BYTES,
    Measurement,
    PayloadClass,
    payload_for_class,
};

/// Extension trait adding timing analysis to [`Measurement`].
///
/// This lives in the fragmentation module because only fragmentation and test
/// consumers use per-operation timing; the allocation bench does not.
pub trait MeasurementExt {
    /// Mean nanoseconds per operation.
    #[must_use]
    fn nanos_per_op(self) -> Option<u128>;
}

impl MeasurementExt for Measurement {
    fn nanos_per_op(self) -> Option<u128> {
        let operations = u32::try_from(self.operations).ok()?;
        self.elapsed
            .checked_div(operations)
            .map(|duration| duration.as_nanos())
    }
}

/// Fragment payload cap used for fragmentation-overhead benchmarks.
pub const FRAGMENT_PAYLOAD_CAP_BYTES: usize = 1024;

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
        let fragmented_operations = u32::try_from(self.fragmented.operations).ok()?;
        let unfragmented_operations = u32::try_from(self.unfragmented.operations).ok()?;
        let fragmented_mean = self.fragmented.elapsed.checked_div(fragmented_operations)?;
        let unfragmented_mean = self
            .unfragmented
            .elapsed
            .checked_div(unfragmented_operations)?;
        if unfragmented_mean.is_zero() {
            return None;
        }
        Some(fragmented_mean.div_duration_f64(unfragmented_mean))
    }
}

/// Measure unfragmented payload wrapping performance for the default codec.
pub fn measure_unfragmented_wrap(payload_class: PayloadClass, iterations: u64) -> Measurement {
    let payload = payload_for_class(payload_class);
    let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
    let started = std::time::Instant::now();
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
    let started = std::time::Instant::now();

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
