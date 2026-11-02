//! Request-hook dispatch measurement for the zero-copy benchmark matrix.
//!
//! Dispatch goes through the client's own before-send loop rather than a
//! harness replica, so the harness and the production path cannot drift apart
//! without changing the client. The hook is registered once and the buffer is
//! built once, outside the timer.

use std::{hint::black_box, sync::Arc, time::Instant};

use wireframe::client::{BeforeSendHook, benchmark_run_before_send_hooks};

use super::{
    codec_benchmark_support::{BenchmarkWorkload, Measurement, payload_for_class},
    pipeline_benchmark_support::{AccessMode, empty_measurement},
};

/// Builds the single before-send hook a measured dispatch runs.
fn hook_for_mode(mode: AccessMode) -> BeforeSendHook {
    match mode {
        AccessMode::ReadOnly => Arc::new(|bytes: &mut Vec<u8>| {
            let _ = black_box(bytes.len());
        }),
        AccessMode::Mutating => Arc::new(|bytes: &mut Vec<u8>| {
            if let Some(first) = bytes.first_mut() {
                *first = first.wrapping_add(1);
            }
        }),
    }
}

/// Measures before-send hook dispatch for one workload and access mode.
///
/// # Examples
///
/// ```ignore
/// let measurement = measure_hook_dispatch(workload, AccessMode::Mutating, 16);
/// assert_eq!(measurement.operations, 16);
/// ```
#[must_use]
pub fn measure_hook_dispatch(
    workload: BenchmarkWorkload,
    mode: AccessMode,
    iterations: u64,
) -> Measurement {
    if iterations == 0 {
        return empty_measurement();
    }
    let payload_len = workload.payload_class.len() as u64;
    let hooks = [hook_for_mode(mode)];
    let mut buffer = payload_for_class(workload.payload_class).to_vec();
    let started = Instant::now();
    for _ in 0..iterations {
        benchmark_run_before_send_hooks(&hooks, &mut buffer);
        let _ = black_box(buffer.len());
    }
    Measurement {
        operations: iterations,
        payload_bytes: iterations.saturating_mul(payload_len),
        elapsed: started.elapsed(),
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the request-hook dispatch helper.

    use super::*;
    use crate::codec_benchmarks::{CodecUnderTest, PayloadClass};

    /// Small length-delimited workload used by the helper tests.
    fn small_workload() -> BenchmarkWorkload {
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Small,
        }
    }

    #[test]
    fn hook_dispatch_records_no_iterations_for_an_empty_batch() {
        let measurement = measure_hook_dispatch(small_workload(), AccessMode::ReadOnly, 0);
        assert_eq!(measurement.operations, 0);
        assert_eq!(measurement.payload_bytes, 0);
    }

    #[test]
    fn a_read_only_hook_leaves_the_buffer_unchanged() {
        let mut bytes = vec![1_u8, 2, 3];
        benchmark_run_before_send_hooks(&[hook_for_mode(AccessMode::ReadOnly)], &mut bytes);
        assert_eq!(bytes, vec![1_u8, 2, 3]);
    }

    #[test]
    fn a_mutating_hook_rewrites_the_first_byte() {
        let mut bytes = vec![1_u8, 2, 3];
        benchmark_run_before_send_hooks(&[hook_for_mode(AccessMode::Mutating)], &mut bytes);
        assert_eq!(bytes, vec![2_u8, 2, 3]);
    }
}
