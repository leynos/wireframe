//! Reusable allocation counting for benchmark binaries.
//!
//! Counting is provided by [`stats_alloc`]'s [`INSTRUMENTED_SYSTEM`], an
//! instrumenting wrapper around the system allocator. A process must install it
//! as its `#[global_allocator]` for the counters to move; Rust permits exactly
//! one such static per binary, so each benchmark binary declares the one-line
//! reference and the counting *mechanism* lives here: the enable gate, the
//! baseline bookkeeping, and the scoped measurement helper.
//!
//! A binary opting in writes:
//!
//! ```rust,ignore
//! use std::alloc::System;
//! use wireframe_testing::benchmark_alloc_support::{INSTRUMENTED_SYSTEM, StatsAlloc};
//!
//! #[global_allocator]
//! static GLOBAL_ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
//! ```
//!
//! and then measures with [`count_allocations`].
//!
//! # Measurement scope
//!
//! The recorded value is a process-global allocation delta across the
//! measurement window. It counts both fresh allocations and reallocations,
//! because a buffer that grows in place is still a buffer that asked the
//! allocator for memory. It also includes allocations from the benchmark
//! harness, the async runtime, closure captures, and anything else running in
//! that window, not only the code under test. Compare values across runs of the
//! same binary rather than treating them as absolute costs.

pub use std::alloc::System;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};

pub use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};

/// Allocations observed by measurement windows that have already closed.
static ACCUMULATED_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
/// Allocator total recorded when the current window opened, or at the last
/// reset. The delta from here is the live window's contribution.
static BASELINE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
/// Gate keeping setup, teardown, and reporting allocations out of a count.
static ALLOCATION_COUNTING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Total allocation operations the instrumenting allocator has observed.
///
/// `stats_alloc` counts `alloc` and `alloc_zeroed` in `allocations` and
/// `realloc` in `reallocations`, so both fields are summed; otherwise a growing
/// buffer would look allocation-free. Every read is an atomic snapshot, so this
/// helper never allocates and is safe to call from inside a window.
fn total_allocations() -> usize {
    let stats = INSTRUMENTED_SYSTEM.stats();
    stats.allocations.saturating_add(stats.reallocations)
}

/// Adopt the current allocator total as the baseline for what follows.
fn record_baseline() { BASELINE_ALLOCATIONS.store(total_allocations(), Ordering::SeqCst); }

/// Fold the current window's delta into the accumulated count.
///
/// The baseline advances to the current total in the same step, so calling this
/// twice without an intervening window adds nothing the second time.
fn accumulate_current_window() {
    let current = total_allocations();
    let baseline = BASELINE_ALLOCATIONS.swap(current, Ordering::SeqCst);
    let delta = current.saturating_sub(baseline);
    ACCUMULATED_ALLOCATIONS.fetch_add(delta, Ordering::SeqCst);
}

/// Reset the counter to zero without changing the gate.
///
/// Exposed so a caller can establish a clean baseline before starting a window
/// it manages itself. When a window is already open, a later
/// [`allocation_count`] reports only what happened after this call.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::{allocation_count, reset_allocation_count};
///
/// reset_allocation_count();
/// assert_eq!(allocation_count(), 0);
/// ```
pub fn reset_allocation_count() {
    ACCUMULATED_ALLOCATIONS.store(0, Ordering::SeqCst);
    record_baseline();
}

/// Read the current allocation count.
///
/// While a window is open the value includes its live delta; otherwise it is
/// the total accumulated by windows that have closed since the last reset.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::allocation_count;
///
/// let _count = allocation_count();
/// ```
#[must_use]
pub fn allocation_count() -> usize {
    let accumulated = ACCUMULATED_ALLOCATIONS.load(Ordering::SeqCst);
    if !ALLOCATION_COUNTING_ENABLED.load(Ordering::SeqCst) {
        return accumulated;
    }
    let baseline = BASELINE_ALLOCATIONS.load(Ordering::SeqCst);
    let live = total_allocations().saturating_sub(baseline);
    accumulated.saturating_add(live)
}

/// Report whether a measurement window is currently open.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::is_allocation_counting_enabled;
///
/// assert!(!is_allocation_counting_enabled());
/// ```
#[must_use]
pub fn is_allocation_counting_enabled() -> bool {
    ALLOCATION_COUNTING_ENABLED.load(Ordering::SeqCst)
}

/// Run `operation` inside an allocation-counting window and return the count.
///
/// Resets the counter, opens the window, runs `operation`, then closes the
/// window and returns the delta. The window is closed even when `operation`
/// fails, so a failed measurement cannot leave counting enabled for the rest of
/// the process.
///
/// # Errors
///
/// Returns the error string produced by `operation` after closing the window.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::{
///     count_allocations,
///     is_allocation_counting_enabled,
/// };
///
/// let count = count_allocations(|| {
///     let buffer: Vec<u8> = vec![0; 1024];
///     std::hint::black_box(buffer.len());
///     Ok(())
/// })?;
/// std::hint::black_box(count);
///
/// // The counter is only driven by `INSTRUMENTED_SYSTEM`, which a process must
/// // install as its `#[global_allocator]`; a benchmark binary that does so
/// // observes a non-zero count here. What holds either way is that the window
/// // closed, so counting cannot leak into the rest of the process.
/// assert!(!is_allocation_counting_enabled(), "the window must close");
/// # Ok::<(), String>(())
/// ```
pub fn count_allocations<F>(operation: F) -> Result<usize, String>
where
    F: FnOnce() -> Result<(), String>,
{
    reset_allocation_count();
    ALLOCATION_COUNTING_ENABLED.store(true, Ordering::SeqCst);
    fence(Ordering::SeqCst);

    let outcome = operation();

    fence(Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
    accumulate_current_window();

    outcome.map(|()| ACCUMULATED_ALLOCATIONS.load(Ordering::SeqCst))
}

/// Run `operation` in a counting window, panicking with `context` on failure.
///
/// Benchmark binaries treat a failed measurement as fatal, but the panic must
/// not fire while the window is open. This adapter therefore delegates to
/// [`count_allocations`], which closes the window first, and only then reports
/// the failure.
///
/// # Panics
///
/// Panics with `context` and the operation's error when `operation` fails.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::count_allocations_or_panic;
///
/// let count = count_allocations_or_panic("no-op measurement", || Ok(()));
/// assert_eq!(count, 0);
/// ```
#[must_use]
pub fn count_allocations_or_panic<F>(context: &str, operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    match count_allocations(operation) {
        Ok(count) => count,
        Err(err) => panic!("{context}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    //! Coverage for the counting window's boundaries.
    //!
    //! Unit-test builds do not install `INSTRUMENTED_SYSTEM`, so the counter
    //! here stays at zero whatever runs inside a window. These tests therefore
    //! cover only what is observable without the allocator: that the gate opens
    //! for the operation and closes again on both the success and error paths.
    //! The counting itself is checked where the allocator is actually installed
    //! — `benches/runtime_ownership_baselines_alloc/probes.rs` runs a zero-window
    //! and a positive-window self-check before measuring, so a gate that never
    //! opens is a loud failure rather than a silent baseline of zeros.
    //!
    //! Both tests are `serial`: the flag they assert on is process-global, so
    //! running them concurrently lets one test's open window be read by the
    //! other's closing assertion. That race made
    //! [`counting_window_closes_after_success`](counting_window_closes_after_success)
    //! fail roughly one run in five even in isolation, which is a flake in the
    //! test rather than a defect in the window.

    use serial_test::serial;

    use super::{allocation_count, count_allocations, is_allocation_counting_enabled};

    #[test]
    #[serial]
    fn counting_window_closes_after_success() {
        let _count = count_allocations(|| Ok(())).expect("no-op measurement must succeed");
        assert!(
            !is_allocation_counting_enabled(),
            "a successful measurement must close the window"
        );
    }

    #[test]
    #[serial]
    fn counting_window_closes_after_failure() {
        let error = count_allocations(|| Err("deliberate failure".to_owned()))
            .expect_err("a failing operation must surface its error");
        assert_eq!(
            error, "deliberate failure",
            "error should equal \"deliberate failure\""
        );
        assert!(
            !is_allocation_counting_enabled(),
            "a failed measurement must still close the window"
        );
    }

    #[test]
    #[serial]
    fn empty_window_counts_nothing() {
        let count = count_allocations(|| Ok(())).expect("no-op measurement must succeed");
        assert_eq!(count, 0, "an empty window must report exactly zero");
        assert_eq!(
            allocation_count(),
            0,
            "the accumulated count must match an empty window"
        );
    }
}
