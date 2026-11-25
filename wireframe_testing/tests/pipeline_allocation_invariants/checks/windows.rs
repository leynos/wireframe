//! Checks on the allocation window itself.
//!
//! These run before any stage bound: a window that never opens, never closes,
//! or charges the wrong figure would make every later check pass for the wrong
//! reason. The idle-window probe is also the target's own guard against a
//! harness that spawns threads.

use std::hint::black_box;

use wireframe_testing::{
    benchmark_alloc_support::{AllocationTraffic, verify_allocation_harness},
    codec_benchmarks::{LARGE_PAYLOAD_BYTES, SMALL_PAYLOAD_BYTES},
};

use crate::support::{NOISE_FLOOR_TRIALS, WINDOW_SLACK_BYTES, traffic};

/// The binary must be instrumented at all.
///
/// A target without the harness does not inherit the benchmarks' allocator, so
/// this asserts the installed one before any bound depends on it.
pub(super) fn harness_observes_real_traffic() { verify_allocation_harness(); }

/// Payload sizes the byte-reporting probe sweeps.
///
/// A geometric sweep from the small class up to the large one, so the claim
/// holds across the range of payloads the harness measures rather than at three
/// sampled points.
fn probe_sizes() -> impl Iterator<Item = usize> {
    std::iter::successors(Some(SMALL_PAYLOAD_BYTES), |size| {
        size.checked_mul(2)
            .filter(|next| *next < LARGE_PAYLOAD_BYTES)
    })
    .chain(std::iter::once(LARGE_PAYLOAD_BYTES))
}

/// A window reports the bytes it asked for, across the payload range.
pub(super) fn windows_report_their_bytes() {
    for bytes in probe_sizes() {
        let traffic = traffic("size probe", || {
            let block = vec![0_u8; bytes];
            black_box(&block);
            Ok(())
        });

        // `>= 1` rather than `== 1`: the assertion is that the allocation was
        // seen, and any stray traffic only adds to the count.
        assert!(
            traffic.operations >= 1,
            "allocating {bytes} bytes must be observed; a zero count means the gate never opened"
        );
        assert!(
            traffic.bytes >= bytes,
            "a {bytes}-byte allocation reported only {} bytes",
            traffic.bytes
        );
        assert!(
            traffic.bytes <= bytes + WINDOW_SLACK_BYTES,
            "a {bytes}-byte allocation reported {} bytes, beyond the {WINDOW_SLACK_BYTES}-byte \
             slack; the counter is charging more than the allocator was asked for",
            traffic.bytes
        );
    }
}

/// Bytes the growing-reallocation probe starts from.
///
/// Sixteen times [`WINDOW_SLACK_BYTES`], so charging a reallocation's whole new
/// size overshoots the slack by far more than any control block could explain.
const GROWTH_PROBE_BYTES: usize = 16 * WINDOW_SLACK_BYTES;

/// A growing reallocation is charged its growth, not its new size.
///
/// Growth-only accounting charges the first allocation plus the capacity delta,
/// which is exactly the vector's final capacity. Charging the full new size
/// instead reports the final capacity twice and fails the upper bound.
pub(super) fn growing_reallocation_reports_growth() {
    let base = GROWTH_PROBE_BYTES;
    let mut grown_capacity = 0;
    let traffic = traffic("growth probe", || {
        let mut block = vec![0_u8; base];
        block.reserve(base);
        grown_capacity = block.capacity();
        black_box(&block);
        Ok(())
    });

    assert!(
        grown_capacity > base,
        "the probe must grow the vector: {base} bytes reported a capacity of {grown_capacity}"
    );
    assert!(
        traffic.bytes >= grown_capacity,
        "a vector grown to {grown_capacity} bytes reported only {} bytes; a growth that is not \
         charged would make every byte bound understate a copy",
        traffic.bytes
    );
    assert!(
        traffic.bytes <= grown_capacity + WINDOW_SLACK_BYTES,
        "a vector grown to {grown_capacity} bytes reported {} bytes, beyond the \
         {WINDOW_SLACK_BYTES}-byte slack; the reallocation is being charged its whole new size \
         instead of its growth",
        traffic.bytes
    );
}

/// One thousand idle windows must all report nothing.
pub(super) fn idle_windows_report_no_traffic() {
    let noisy: Vec<(usize, AllocationTraffic)> = (0..NOISE_FLOOR_TRIALS)
        .filter_map(|trial| {
            let traffic = traffic("idle", || Ok(()));
            (traffic != AllocationTraffic::default()).then_some((trial, traffic))
        })
        .collect();

    assert!(
        noisy.is_empty(),
        "{} of {NOISE_FLOOR_TRIALS} idle windows reported traffic, starting with {:?}; Stage A's \
         rule is that this target must not run under a harness that spawns threads",
        noisy.len(),
        noisy.first()
    );
}
