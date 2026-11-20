//! The checks the invariants binary runs, one function per claim.
//!
//! Each function either returns or panics with the measurement that broke it,
//! so a failure names the byte count or address rather than only the claim.

use std::hint::black_box;

use wireframe_testing::{
    benchmark_alloc_support::{AllocationTraffic, verify_allocation_harness},
    codec_benchmarks::{
        AccessMode,
        LARGE_PAYLOAD_BYTES,
        PayloadClass,
        SMALL_PAYLOAD_BYTES,
        VALIDATION_ITERATIONS,
        hook_payload_handoff,
        middleware_payload_handoff,
        payload_for_class,
    },
};

use super::support::{
    NOISE_FLOOR_TRIALS,
    WINDOW_SLACK_BYTES,
    copying_hook,
    hook_traffic,
    middleware_traffic,
    read_only_hook,
    test_runtime,
    traffic,
    workload,
};

/// The binary must be instrumented at all.
///
/// A target without the harness does not inherit the benchmarks' allocator, so
/// this asserts the installed one before any bound depends on it.
pub(super) fn harness_observes_real_traffic() { verify_allocation_harness(); }

/// A window reports the bytes it asked for, at every payload scale.
pub(super) fn windows_report_their_bytes() {
    for bytes in [SMALL_PAYLOAD_BYTES, 1_024, LARGE_PAYLOAD_BYTES] {
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

/// A growing reallocation is charged its growth, not its new size.
pub(super) fn growing_reallocation_reports_growth() {
    let base = WINDOW_SLACK_BYTES;
    let traffic = traffic("growth probe", || {
        let mut block = vec![0_u8; base];
        block.reserve(base);
        black_box(&block);
        Ok(())
    });

    assert!(
        traffic.bytes >= 2 * base,
        "a vector grown from {base} to at least {} bytes reported {} bytes; a growth that is not \
         charged would make every byte bound understate a copy",
        2 * base,
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

/// The read-only middleware returns the buffer it was handed.
pub(super) fn middleware_preserves_buffer() {
    for payload_class in [PayloadClass::Small, PayloadClass::Large] {
        let outcome = test_runtime().block_on(middleware_payload_handoff(
            workload(payload_class),
            AccessMode::ReadOnly,
        ));

        assert!(
            outcome.is_payload_preserved(),
            "read-only middleware relocated the {payload_class:?} payload buffer: {} -> {}",
            outcome.arrival_address,
            outcome.departure_address
        );
        assert_eq!(
            outcome.payload.as_slice(),
            payload_for_class(payload_class),
            "read-only middleware must not rewrite the {payload_class:?} payload"
        );
    }
}

/// The read-only hook returns the buffer it was handed.
pub(super) fn hooks_preserve_buffer() {
    for payload_class in [PayloadClass::Small, PayloadClass::Large] {
        let outcome = hook_payload_handoff(workload(payload_class), AccessMode::ReadOnly);

        assert!(
            outcome.is_payload_preserved(),
            "read-only hook dispatch relocated the {payload_class:?} payload buffer: {} -> {}",
            outcome.arrival_address,
            outcome.departure_address
        );
        assert_eq!(
            outcome.payload.as_slice(),
            payload_for_class(payload_class),
            "read-only hook dispatch must not rewrite the {payload_class:?} payload"
        );
    }
}

/// A mutating hook must be seen to change the buffer, or the hook row proves
/// nothing about dispatch having run at all.
pub(super) fn mutating_hook_changes_buffer() {
    let outcome = hook_payload_handoff(workload(PayloadClass::Small), AccessMode::Mutating);

    assert_ne!(
        outcome.payload.as_slice(),
        payload_for_class(PayloadClass::Small),
        "a mutating hook must be observed to change the payload"
    );
}

/// The seeded fault must break the pointer assertion for the hook row.
pub(super) fn copying_hook_relocates_buffer() {
    let outcome = hook_payload_handoff(workload(PayloadClass::Large), AccessMode::Copying);

    assert!(
        !outcome.is_payload_preserved(),
        "a copying hook must allocate a new buffer; it reported {} on both sides",
        outcome.departure_address
    );
}

/// The seeded fault must break the pointer assertion for the middleware row.
pub(super) fn copying_middleware_relocates_buffer() {
    let outcome = test_runtime().block_on(middleware_payload_handoff(
        workload(PayloadClass::Large),
        AccessMode::Copying,
    ));

    assert!(
        !outcome.is_payload_preserved(),
        "a copying middleware must allocate a new buffer; it reported {} on both sides",
        outcome.departure_address
    );
}

/// Traversal allocates the same number of times whatever the payload size.
pub(super) fn read_only_middleware_counts_match() {
    let small = middleware_traffic(
        AccessMode::ReadOnly,
        PayloadClass::Small,
        VALIDATION_ITERATIONS,
    );
    let large = middleware_traffic(
        AccessMode::ReadOnly,
        PayloadClass::Large,
        VALIDATION_ITERATIONS,
    );

    assert_eq!(
        small.operations, large.operations,
        "a read-only batch allocated {} times for a {SMALL_PAYLOAD_BYTES}-byte payload and {} \
         times for a {LARGE_PAYLOAD_BYTES}-byte payload; traversal must not scale with the payload",
        small.operations, large.operations
    );
}

/// One read-only middleware pass allocates nothing payload-sized.
pub(super) fn read_only_middleware_pass_stays_below_payload() {
    let traffic = middleware_traffic(AccessMode::ReadOnly, PayloadClass::Large, 1);

    assert!(
        traffic.bytes < LARGE_PAYLOAD_BYTES,
        "one read-only middleware pass allocated {} bytes for a {LARGE_PAYLOAD_BYTES}-byte \
         payload; a single payload copy would already reach the bound",
        traffic.bytes
    );
}

/// A read-only hook batch allocates nothing payload-sized.
pub(super) fn read_only_hooks_stay_below_payload() {
    let traffic = hook_traffic(read_only_hook(), VALIDATION_ITERATIONS);

    assert!(
        traffic.bytes < LARGE_PAYLOAD_BYTES,
        "read-only hooks allocated {} bytes over a batch for a {LARGE_PAYLOAD_BYTES}-byte payload",
        traffic.bytes
    );
}

/// The copying middleware keeps the count relation, which is why the byte bound
/// exists.
///
/// The relation compares two payload classes and the fault adds one allocation
/// to each, so the counts stay equal while the traffic does not.
pub(super) fn copying_middleware_counts_match() {
    let small = middleware_traffic(
        AccessMode::Copying,
        PayloadClass::Small,
        VALIDATION_ITERATIONS,
    );
    let large = middleware_traffic(
        AccessMode::Copying,
        PayloadClass::Large,
        VALIDATION_ITERATIONS,
    );

    assert_eq!(
        small.operations, large.operations,
        "the seeded fault was expected to keep the count relation blind: {} small operations, {} \
         large",
        small.operations, large.operations
    );
}

/// One copying middleware pass must break the byte bound.
pub(super) fn copying_middleware_pass_exceeds_payload() {
    let traffic = middleware_traffic(AccessMode::Copying, PayloadClass::Large, 1);

    assert!(
        traffic.bytes >= LARGE_PAYLOAD_BYTES,
        "a copying middleware must cost at least one payload copy per pass; it reported {} bytes \
         against a {LARGE_PAYLOAD_BYTES}-byte payload",
        traffic.bytes
    );
}

/// The copying hook must break the byte bound.
pub(super) fn copying_hooks_exceed_payload() {
    let traffic = hook_traffic(copying_hook(), VALIDATION_ITERATIONS);

    assert!(
        traffic.bytes >= LARGE_PAYLOAD_BYTES,
        "a copying hook must cost at least one payload copy per batch; it reported {} bytes \
         against a {LARGE_PAYLOAD_BYTES}-byte payload",
        traffic.bytes
    );
}
