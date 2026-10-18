//! Reusable allocation counting for benchmark binaries.
//!
//! A counting allocator must be installed as the process's `#[global_allocator]`,
//! and Rust permits exactly one such static per binary. That makes the allocator
//! itself unshareable, but the counting *mechanism* is not: the counter, the
//! enable gate, and the scoped measurement helper can all live here, leaving each
//! benchmark binary to declare only the one-line static.
//!
//! A binary opting in writes:
//!
//! ```rust,ignore
//! use wireframe_testing::benchmark_alloc_support::CountingAllocator;
//!
//! #[global_allocator]
//! static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;
//! ```
//!
//! and then measures with [`count_allocations`].
//!
//! # Measurement scope
//!
//! The recorded value is a process-global allocation delta across the measurement
//! window. It includes allocations from the benchmark harness, the async runtime,
//! closure captures, and anything else running in that window, not only the code
//! under test. Compare values across runs of the same binary rather than treating
//! them as absolute costs.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence},
};

/// Forwards allocations to the system allocator while counting the operations
/// observed during an enabled measurement window.
///
/// Install one as the process `#[global_allocator]`; the type is otherwise inert
/// and holds no state, because the counter is process-global.
pub struct CountingAllocator;

/// Number of allocation operations observed in the current measurement window.
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Gate keeping setup, teardown, and reporting allocations out of a count.
static ALLOCATION_COUNTING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Increment the shared counter only while a measurement window is active.
///
/// The acquire load pairs with the release fences [`count_allocations`] uses when
/// toggling the gate.
fn record_allocation_if_enabled() {
    if ALLOCATION_COUNTING_ENABLED.load(Ordering::Acquire) {
        ALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

// SAFETY: every method delegates to `System` with unchanged arguments and adds
// only an atomic increment. Pointer ownership, layout, and alignment semantics
// are exactly those of the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: delegates to the system allocator with unchanged arguments.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: delegates to the system allocator with unchanged arguments.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Reset the counter to zero without changing the gate.
///
/// Exposed so a caller can establish a clean baseline before starting a window
/// it manages itself.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::{allocation_count, reset_allocation_count};
///
/// reset_allocation_count();
/// assert_eq!(allocation_count(), 0);
/// ```
pub fn reset_allocation_count() { ALLOCATION_COUNT.store(0, Ordering::SeqCst); }

/// Read the current allocation count.
///
/// The value is only meaningful during or immediately after a measurement
/// window opened by [`count_allocations`]; outside one it lags the last reset.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::allocation_count;
///
/// let _count = allocation_count();
/// ```
#[must_use]
pub fn allocation_count() -> usize { ALLOCATION_COUNT.load(Ordering::SeqCst) }

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
/// // The counter is only driven by `CountingAllocator`, which a process must
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
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(true, Ordering::SeqCst);
    fence(Ordering::SeqCst);

    let outcome = operation();

    fence(Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);

    outcome.map(|()| ALLOCATION_COUNT.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    //! Coverage for the counting window's boundaries.
    //!
    //! Unit-test builds do not install `CountingAllocator`, so the counter here
    //! stays at zero whatever runs inside a window. These tests therefore cover
    //! only what is observable without the allocator: that the gate opens for the
    //! operation and closes again on both the success and error paths. The
    //! counting itself is checked where the allocator is actually installed —
    //! `benches/runtime_ownership_baselines_alloc/probes.rs` runs a zero-window and a
    //! positive-window self-check before measuring, so a gate that never opens is
    //! a loud failure rather than a silent baseline of zeros.
    //!
    //! Both tests are `serial`: the flag they assert on is process-global, so
    //! running them concurrently lets one test's open window be read by the
    //! other's closing assertion. That race made
    //! [`counting_window_closes_after_success`](counting_window_closes_after_success)
    //! fail roughly one run in five even in isolation, which is a flake in the
    //! test rather than a defect in the window.

    use serial_test::serial;

    use super::{count_allocations, is_allocation_counting_enabled};

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
        let error = count_allocations(|| Err("deliberate failure".to_string()))
            .expect_err("a failing operation must surface its error");
        assert_eq!(error, "deliberate failure");
        assert!(
            !is_allocation_counting_enabled(),
            "a failed measurement must still close the window"
        );
    }
}
