//! Process-global counting allocator for allocation-baseline benchmarks.
//!
//! Each allocation-baseline benchmark binary installs [`CountingAllocator`] as
//! its `#[global_allocator]` and brackets the measured window with
//! [`count_allocations`]. The counter lives here rather than in one bench so
//! several benchmark binaries can share the same accounting contract instead
//! of drifting apart.
//!
//! The count is a process-global delta, not a codec-only or stage-only
//! measurement. It includes allocations made by harness code inside the
//! window, so it is a relative baseline for comparison across runs.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence},
};

/// Number of allocation operations observed in the current measurement window.
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Gate that keeps setup and reporting allocations out of a baseline count.
static ALLOCATION_COUNTING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Forwards allocations to the system allocator while counting only those
/// observed while a measurement window is enabled.
pub struct CountingAllocator;

/// Increments the shared counter only while an allocation baseline is active.
///
/// The acquire load pairs with the measurement fences that toggle the gate.
fn record_allocation_if_enabled() {
    if ALLOCATION_COUNTING_ENABLED.load(Ordering::Acquire) {
        ALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

// SAFETY: This allocator forwards every operation directly to `System` while
// incrementing an atomic counter. It does not change pointer ownership, layout
// semantics, or alignment behaviour.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged `layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: Delegates to the system allocator with unchanged arguments.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Measures allocations in a scoped window using the process-global counter.
///
/// The result is a global allocation delta during `operation`, not a precise
/// stage-only measurement. Treat the value as a noisy relative baseline for
/// comparison across runs.
///
/// # Panics
///
/// Panics when `operation` reports an error, after disabling the counting gate
/// so a failed baseline cannot poison later measurements.
///
/// # Examples
///
/// ```ignore
/// let allocations = count_allocations(|| {
///     measure_decode(workload, 16)?;
///     Ok(())
/// });
/// ```
pub fn count_allocations<F>(operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(true, Ordering::SeqCst);
    fence(Ordering::SeqCst);

    if let Err(err) = operation() {
        fence(Ordering::SeqCst);
        ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
        panic!("allocation baseline operation failed: {err}");
    }

    fence(Ordering::SeqCst);
    ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
    ALLOCATION_COUNT.load(Ordering::SeqCst)
}
