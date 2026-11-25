//! Shared plumbing for the allocation invariants: windows and stage traffic.
//!
//! Each helper builds its scenario before the window opens. That matters more
//! here than in a timing measurement: a payload buffer built inside the window
//! is a payload-sized allocation the window would report, and no bound
//! afterwards could tell it apart from the stage making a copy.

use std::{hint::black_box, sync::Arc};

use wireframe::{
    app::{Envelope, benchmark_encode_message_frame},
    client::{BeforeSendHook, benchmark_run_before_send_hooks},
    codec::LengthDelimitedFrameCodec,
    serializer::BincodeSerializer,
};
use wireframe_testing::{
    benchmark_alloc_support::{AllocationTraffic, count_allocation_traffic},
    codec_benchmarks::{
        AccessMode,
        BenchmarkWorkload,
        CodecUnderTest,
        LARGE_PAYLOAD_BYTES,
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
/// The buffer is always the large class: the byte bounds are stated against it.
/// Callers that compare payload classes use [`hook_traffic_for_class`].
pub(crate) fn hook_traffic(hook: BeforeSendHook, iterations: u64) -> AllocationTraffic {
    hook_traffic_for_class(hook, PayloadClass::Large, iterations)
}

/// Opens a window around a hook batch for one payload class.
pub(crate) fn hook_traffic_for_class(
    hook: BeforeSendHook,
    payload_class: PayloadClass,
    iterations: u64,
) -> AllocationTraffic {
    hook_traffic_with_buffer(hook, payload_for_class(payload_class).to_vec(), iterations)
}

/// Opens a window around one hook pass over an arbitrary payload buffer.
///
/// The buffer is built before the window opens, so a byte bound cannot charge
/// the caller's own allocation to the hook.
pub(crate) fn hook_traffic_with_buffer(
    hook: BeforeSendHook,
    mut buffer: Vec<u8>,
    iterations: u64,
) -> AllocationTraffic {
    let hooks = [hook];
    traffic("hook batch", move || {
        for _ in 0..iterations {
            benchmark_run_before_send_hooks(&hooks, &mut buffer);
            let _len = black_box(buffer.len());
        }
        Ok(())
    })
}

/// Runs one read-only hook pass over `buffer`, reporting its identity claims.
///
/// This is the synchronous counterpart of `hook_payload_handoff` for an
/// arbitrary buffer size, so a sweep can assert that a read-only hook neither
/// relocates nor rewrites the payload at sizes between the sampled classes.
pub(crate) fn read_only_hook_pass(mut buffer: Vec<u8>) -> (usize, usize, Vec<u8>) {
    let hooks = [read_only_hook()];
    let arrival_address = buffer.as_ptr() as usize;
    benchmark_run_before_send_hooks(&hooks, &mut buffer);
    (arrival_address, buffer.as_ptr() as usize, buffer)
}

/// Opens a window around the production serialize-then-wrap operation.
///
/// The envelope, its payload, and the codec are all built before the window
/// opens. `wrap_payload` for the length-delimited codec hands the `Bytes` on
/// without copying, so the window sees exactly the serializer's own allocation
/// and the optional `Bytes` control block — never the framing copy, which
/// happens later in the codec's transport write.
pub(crate) fn outbound_traffic(payload_class: PayloadClass) -> AllocationTraffic {
    let envelope = Envelope::new(1, Some(7), payload_for_class(payload_class).to_vec());
    let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);
    traffic("production outbound", move || {
        let frame = benchmark_encode_message_frame(&BincodeSerializer, &codec, &envelope)
            .map_err(|err| format!("the production outbound path must encode: {err}"))?;
        let _len = black_box(frame.len());
        Ok(())
    })
}
