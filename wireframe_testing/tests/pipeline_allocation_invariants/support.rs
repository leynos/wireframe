//! Shared plumbing for the allocation invariants: windows and stage traffic.
//!
//! Each helper builds its scenario before the window opens. That matters more
//! here than in a timing measurement: a payload buffer built inside the window
//! is a payload-sized allocation the window would report, and no bound
//! afterwards could tell it apart from the stage making a copy.

use std::{hint::black_box, sync::Arc};

use wireframe::client::{BeforeSendHook, benchmark_run_before_send_hooks};
use wireframe_testing::{
    benchmark_alloc_support::{AllocationTraffic, count_allocation_traffic},
    codec_benchmarks::{
        AccessMode,
        BenchmarkWorkload,
        CodecUnderTest,
        PayloadClass,
        measure_middleware_passthrough_buffer,
        payload_for_class,
    },
};

/// Bytes a window may charge beyond what it was asked to allocate.
///
/// The slack covers control blocks: the boxed futures `async_trait` builds per
/// middleware call and the runtime's own bookkeeping. Those are hundreds of
/// bytes per pass, two orders of magnitude below the 64 KiB payload class. A
/// payload copy is not slack — it costs at least `P` — which is why every byte
/// bound can tell a copy from plumbing.
pub(crate) const WINDOW_SLACK_BYTES: usize = 4096;

/// Idle windows the noise-floor probe opens, per Stage A's decision rule.
pub(crate) const NOISE_FLOOR_TRIALS: usize = 1_000;

/// Builds one workload for the invariants.
pub(crate) const fn workload(payload_class: PayloadClass) -> BenchmarkWorkload {
    BenchmarkWorkload {
        codec: CodecUnderTest::LengthDelimited,
        payload_class,
    }
}

/// Builds the current-thread runtime the async stage helpers need.
pub(crate) fn test_runtime() -> tokio::runtime::Runtime {
    match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => panic!("a current-thread runtime must build: {err}"),
    }
}

/// Builds the read-only hook dispatch runs, mirroring the measured row.
pub(crate) fn read_only_hook() -> BeforeSendHook {
    Arc::new(|bytes: &mut Vec<u8>| {
        let _ = black_box(bytes.len());
    })
}

/// Builds the copying hook the byte bound must reject.
pub(crate) fn copying_hook() -> BeforeSendHook {
    Arc::new(|bytes: &mut Vec<u8>| {
        *bytes = bytes.clone();
    })
}

/// Opens one window, failing the check if the window cannot close.
pub(crate) fn traffic(
    description: &str,
    operation: impl FnOnce() -> Result<(), String>,
) -> AllocationTraffic {
    match count_allocation_traffic(operation) {
        Ok(traffic) => traffic,
        Err(err) => panic!("the {description} window did not close: {err}"),
    }
}

/// Opens a window around a middleware batch whose buffer was built outside it.
pub(crate) fn middleware_traffic(
    mode: AccessMode,
    payload_class: PayloadClass,
    iterations: u64,
) -> AllocationTraffic {
    middleware_traffic_with_buffer(mode, payload_for_class(payload_class).to_vec(), iterations)
}

/// Opens a window around a middleware batch for an arbitrary payload buffer.
///
/// Callers choose the size when a claim is stated across the payload range
/// rather than at the sampled classes. The buffer is always built before the
/// window opens, so its allocation cannot be mistaken for the stage's.
pub(crate) fn middleware_traffic_with_buffer(
    mode: AccessMode,
    buffer: Vec<u8>,
    iterations: u64,
) -> AllocationTraffic {
    let runtime = test_runtime();
    traffic("middleware batch", move || {
        let measurement = runtime.block_on(measure_middleware_passthrough_buffer(
            mode, buffer, iterations,
        ));
        let _measurement = black_box(measurement);
        Ok(())
    })
}

/// Opens a window around a hook batch whose buffer was built outside it.
///
/// The buffer is always the large class: the byte bounds are stated against it,
/// and the count relation is asserted for the middleware alone.
pub(crate) fn hook_traffic(hook: BeforeSendHook, iterations: u64) -> AllocationTraffic {
    let hooks = [hook];
    let mut buffer = payload_for_class(PayloadClass::Large).to_vec();
    traffic("hook batch", move || {
        for _ in 0..iterations {
            benchmark_run_before_send_hooks(&hooks, &mut buffer);
            let _len = black_box(buffer.len());
        }
        Ok(())
    })
}
