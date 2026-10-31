//! The nested-`Arc` sensitivity probes, and the checks that prove the counter
//! resolves them.
//!
//! These run before any runtime or Criterion harness exists, because the counter
//! is process-global: a background thread allocating during a window would make
//! the zero case untrustworthy.

use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use wireframe_bench_counting::count_allocations;

/// Items per probe in the nested-`Arc` sensitivity experiment.
pub(crate) const ARC_PROBE_ITEMS: usize = 256;

/// Unwrap a benchmark helper's result, terminating with a diagnostic on error.
///
/// A failed workload cannot produce a meaningful sample, so stopping the
/// benchmark is the only honest outcome: recording a number for a scenario that
/// did not run would put a fabricated baseline on record.
pub(crate) fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Run one workload inside an allocation-counting window, returning the delta.
pub(crate) fn count_window<F>(context: &str, operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    expect_bench(count_allocations(operation), context)
}

/// Adapt an infallible probe to the fallible shape the counting window expects.
///
/// The nested-`Arc` probes have no failure mode, so they stay plain closures and
/// this supplies the `Result` the window expects. The alternative — making every
/// probe return `Ok(())` on a path that can never be `Err` — would be dead
/// machinery the compiler rightly flags.
pub(crate) fn infallible(probe: impl Fn()) -> impl Fn() -> Result<(), String> {
    move || {
        probe();
        Ok(())
    }
}

/// Adapt an infallible probe to the timing pass's duration-returning shape.
///
/// The probe itself has no failure mode, so the `Result` here only carries the
/// clock reading. Timing starts after any closure setup and stops before the
/// value is returned, which is what `time_iterations` sums.
pub(crate) fn timed_infallible(probe: impl Fn()) -> impl Fn() -> Result<Duration, String> {
    move || {
        let started = Instant::now();
        probe();
        Ok(started.elapsed())
    }
}

/// Clone one shared `Arc` per item, against one freshly allocated shared value.
///
/// A clone is an atomic increment of the strong count: it moves no allocation,
/// which is one half of what the sensitivity experiment must resolve. The lone
/// `Arc::new` *does* allocate — one control block, no matter how many items are
/// cloned — so the expected delta against [`probe_fresh_allocation`] is
/// `items - 1`, not `items`. The assertion in [`verify_allocation_harness`]
/// states that arithmetic explicitly.
fn probe_shared_clone(items: usize) {
    let shared = Arc::new(black_box(0u8));
    for _ in 0..items {
        black_box(Arc::clone(&shared));
    }
}

/// Allocate one fresh `Arc` per item.
///
/// Each construction heap-allocates its control block, so this is the shape
/// whose removal an allocation counter must be able to see.
fn probe_fresh_allocation(items: usize) {
    for _ in 0..items {
        black_box(Arc::new(black_box(0u8)));
    }
}

/// A named allocation probe: a label, and the function that allocates.
///
/// Named rather than inlined in [`arc_probes`]'s return type because the bare
/// pair of a `&'static str` and a function pointer exceeds the workspace's
/// type-complexity limit, and because the alias documents what the two halves
/// mean together.
///
/// The probe is infallible. Returning `Result` here would be dead machinery —
/// neither probe has a failure mode — and infallibility is what lets the two
/// probe functions stay plain loops instead of error-returning shapes they do
/// not need.
pub(crate) type ArcProbe = (&'static str, fn(usize));

/// The two nested-`Arc` probe shapes, paired with their labels.
pub(crate) fn arc_probes() -> Vec<ArcProbe> {
    vec![
        ("shared_clone", probe_shared_clone),
        ("fresh_per_item", probe_fresh_allocation),
    ]
}

/// Prove the allocation harness resolves what the runtime-ownership baselines
/// depend on, stopping the binary loudly if it cannot.
///
/// Runs before any runtime or Criterion harness exists, because the counter is
/// process-global: a background thread allocating during a window would make the
/// zero case untrustworthy. Three checks:
///
/// 1. a closure that allocates nothing reports exactly zero, catching a gate stuck open;
/// 2. a closure that allocates a known buffer reports a positive count, catching a gate that never
///    opens — which would otherwise record a silent baseline of zeros for every scope; and
/// 3. a per-item `Arc` construction reports one allocation per item more than cloning a shared
///    `Arc`, which is the resolution acceptance criterion 5 requires.
pub(crate) fn verify_allocation_harness() {
    let zero = count_window("zero-allocation probe", || Ok(()));
    assert_eq!(
        zero, 0,
        "an allocation-free closure must report zero; a non-zero count means the counting gate is \
         stuck open or something else ran during the window"
    );

    let positive = count_window("positive-allocation probe", || {
        let buffer: Vec<u8> = vec![0; 1024];
        black_box(buffer.len());
        Ok(())
    });
    assert!(
        positive > 0,
        "allocating a 1024-byte buffer must be observed; a zero count means the counting gate \
         never opens and every baseline below would be a silent zero"
    );

    let shared = count_window(
        "shared-clone probe",
        infallible(|| probe_shared_clone(ARC_PROBE_ITEMS)),
    );
    let fresh = count_window(
        "fresh-allocation probe",
        infallible(|| probe_fresh_allocation(ARC_PROBE_ITEMS)),
    );
    // The shared probe allocates once for its single `Arc::new`, and every clone
    // after that is a bare atomic increment, so the per-item difference is
    // `ARC_PROBE_ITEMS - 1` at minimum. Demanding the full item count would fail
    // on a correct counter for an off-by-one reason.
    let expected_delta = ARC_PROBE_ITEMS.saturating_sub(1);
    assert!(
        fresh >= shared.saturating_add(expected_delta),
        "per-item `Arc` construction must cost an allocation per item beyond the shared probe's \
         single allocation: {fresh} fresh vs {shared} shared across {ARC_PROBE_ITEMS} items, \
         expecting a delta of at least {expected_delta}"
    );
}
