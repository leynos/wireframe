//! Checks on the pipeline stages: middleware pass-through and hook dispatch.
//!
//! Pointer identity and allocator traffic are the two halves of the read-only
//! claim. The seeded copying faults are what prove each half runs at all, and
//! the payload sweeps keep the byte bounds from resting on two sampled sizes.

use wireframe_testing::{
    benchmark_alloc_support::AllocationTraffic,
    codec_benchmarks::{
        AccessMode,
        LARGE_PAYLOAD_BYTES,
        PayloadClass,
        SMALL_PAYLOAD_BYTES,
        StageOutcome,
        VALIDATION_ITERATIONS,
        hook_payload_handoff,
        middleware_payload_handoff,
        payload_for_class,
    },
};

use crate::support::{
    copying_hook,
    hook_traffic,
    hook_traffic_for_class,
    hook_traffic_with_buffer,
    middleware_traffic,
    middleware_traffic_with_buffer,
    read_only_hook,
    read_only_hook_pass,
    test_runtime,
    workload,
};

/// Drives one middleware payload pass and reports what it did to the buffer.
fn middleware_pass(payload_class: PayloadClass, access_mode: AccessMode) -> StageOutcome {
    test_runtime().block_on(middleware_payload_handoff(
        workload(payload_class),
        access_mode,
    ))
}

/// Drives one hook payload pass and reports what it did to the buffer.
fn hook_pass(payload_class: PayloadClass, access_mode: AccessMode) -> StageOutcome {
    hook_payload_handoff(workload(payload_class), access_mode)
}

/// Asserts a read-only stage returns the buffer it was handed, unchanged.
///
/// `stage` names the row in the failure message, and `pass` runs the stage for
/// one payload class, so both read-only rows assert the same two claims.
fn assert_read_only_stage_preserves(stage: &str, pass: impl Fn(PayloadClass) -> StageOutcome) {
    for payload_class in [PayloadClass::Small, PayloadClass::Large] {
        let outcome = pass(payload_class);

        assert!(
            outcome.is_payload_preserved(),
            "read-only {stage} relocated the {payload_class:?} payload buffer: {} -> {}",
            outcome.arrival_address,
            outcome.departure_address
        );
        assert_eq!(
            outcome.payload.as_slice(),
            payload_for_class(payload_class),
            "read-only {stage} must not rewrite the {payload_class:?} payload"
        );
    }
}

/// Asserts the seeded fault moved the buffer, which is what makes a row prove
/// the pointer comparison runs at all.
fn assert_copying_stage_relocates(stage: &str, outcome: &StageOutcome) {
    assert!(
        !outcome.is_payload_preserved(),
        "a copying {stage} must allocate a new buffer; it reported {} on both sides",
        outcome.departure_address
    );
}

/// The read-only middleware returns the buffer it was handed.
pub(super) fn middleware_preserves_buffer() {
    assert_read_only_stage_preserves("middleware", |payload_class| {
        middleware_pass(payload_class, AccessMode::ReadOnly)
    });
}

/// The read-only hook returns the buffer it was handed.
pub(super) fn hooks_preserve_buffer() {
    assert_read_only_stage_preserves("hook dispatch", |payload_class| {
        hook_pass(payload_class, AccessMode::ReadOnly)
    });
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
    let outcome = hook_pass(PayloadClass::Large, AccessMode::Copying);

    assert_copying_stage_relocates("hook", &outcome);
}

/// The seeded fault must break the pointer assertion for the middleware row.
pub(super) fn copying_middleware_relocates_buffer() {
    let outcome = middleware_pass(PayloadClass::Large, AccessMode::Copying);

    assert_copying_stage_relocates("middleware", &outcome);
}

/// Asserts a stage allocates the same number of times for either payload class.
///
/// Both batches run the same number of iterations, so a count that grows with
/// the payload is the signature of a copy; the byte bounds are what then reject
/// it.
fn assert_batch_counts_match(stage: &str, small: AllocationTraffic, large: AllocationTraffic) {
    assert_eq!(
        small.operations, large.operations,
        "a {stage} batch allocated {} times for a {SMALL_PAYLOAD_BYTES}-byte payload and {} times \
         for a {LARGE_PAYLOAD_BYTES}-byte payload; {stage} must not scale with the payload",
        small.operations, large.operations
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

    assert_batch_counts_match("read-only middleware", small, large);
}

/// Smallest payload the stage byte probes sweep.
///
/// One eighth of the large class. The floor keeps per-call plumbing — the
/// boxed futures `async_trait` allocates — far below the bound, so the sweep
/// tests for a payload copy rather than for fixed overhead.
const STAGE_SWEEP_FLOOR_BYTES: usize = 8 * 1024;

/// Payload sizes the stage byte probes sweep.
///
/// A geometric sweep from [`STAGE_SWEEP_FLOOR_BYTES`] to twice the large class,
/// so the top doubles the largest class, which a copy would track.
fn stage_probe_sizes() -> impl Iterator<Item = usize> {
    std::iter::successors(Some(STAGE_SWEEP_FLOOR_BYTES), |size| {
        size.checked_mul(2)
            .filter(|next| *next <= 2 * LARGE_PAYLOAD_BYTES)
    })
}

/// One read-only middleware pass allocates nothing payload-sized, across the
/// payload range.
///
/// A copy costs at least the payload at every size, while the pass's own byte
/// cost stays roughly constant, so the sweep catches a copy at every size it
/// samples rather than at two fixed points. The bound is stated against large
/// payloads because a small payload costs less than the per-call control blocks
/// `async_trait` allocates, and those are plumbing, not copies.
pub(super) fn read_only_middleware_pass_stays_below_payload() {
    for payload_bytes in stage_probe_sizes() {
        let traffic =
            middleware_traffic_with_buffer(AccessMode::ReadOnly, vec![0x5a; payload_bytes], 1);

        assert!(
            traffic.bytes < payload_bytes,
            "one read-only middleware pass allocated {} bytes for a {payload_bytes}-byte payload; \
             a single payload copy would already reach the bound",
            traffic.bytes
        );
    }
}

/// A read-only middleware batch allocates less than `P`.
///
/// V-4 states the byte bound over a batch. `async_trait` boxes one future per
/// call at a payload-independent cost, so a sixteen-pass batch costs a few
/// kilobytes and stays two orders of magnitude below the large class, while a
/// single payload copy in the batch would exceed the bound. The count relation
/// over the same batch proves the cost does not scale with the payload.
pub(super) fn read_only_middleware_batch_stays_below_payload() {
    let traffic = middleware_traffic(
        AccessMode::ReadOnly,
        PayloadClass::Large,
        VALIDATION_ITERATIONS,
    );

    assert!(
        traffic.bytes < LARGE_PAYLOAD_BYTES,
        "a {VALIDATION_ITERATIONS}-pass read-only middleware batch allocated {} bytes for a \
         {LARGE_PAYLOAD_BYTES}-byte payload; one payload copy would exceed the bound",
        traffic.bytes
    );
}

/// Read-only hook batches allocate the same number of times for either payload.
///
/// The hook seam is synchronous, so this is the hook counterpart of the
/// middleware count relation: a batch that scales with the payload is the
/// signature of a copy the byte bound must then reject.
pub(super) fn read_only_hook_counts_match() {
    let small =
        hook_traffic_for_class(read_only_hook(), PayloadClass::Small, VALIDATION_ITERATIONS);
    let large =
        hook_traffic_for_class(read_only_hook(), PayloadClass::Large, VALIDATION_ITERATIONS);

    assert_batch_counts_match("read-only hook", small, large);
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

/// Read-only hooks preserve the payload at every size the sweep probes.
///
/// The pointer identity and the byte bound are the two halves of the read-only
/// claim, and both are checked here for sizes between the sampled classes, so
/// the claim does not rest on the two sizes the workload matrix happens to use.
pub(super) fn read_only_hooks_hold_across_payload_sizes() {
    for payload_bytes in stage_probe_sizes() {
        let buffer = vec![0x5a; payload_bytes];
        let traffic = hook_traffic_with_buffer(read_only_hook(), buffer.clone(), 1);

        assert!(
            traffic.bytes < payload_bytes,
            "a read-only hook pass allocated {} bytes for a {payload_bytes}-byte payload; a \
             single payload copy would already reach the bound",
            traffic.bytes
        );

        let (arrival, departure, payload) = read_only_hook_pass(buffer);

        assert_eq!(
            departure, arrival,
            "a read-only hook relocated the {payload_bytes}-byte payload: {arrival} -> {departure}"
        );
        assert!(
            payload.iter().all(|byte| *byte == 0x5a),
            "a read-only hook rewrote the {payload_bytes}-byte payload"
        );
    }
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
