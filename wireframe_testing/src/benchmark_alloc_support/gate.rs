//! The process-global counting gate and its bookkeeping.
//!
//! Every reported figure is a delta over an interval during which the gate was
//! open, so the state lives here rather than in the measurement helpers. The
//! gate is process-global by nature: `stats_alloc` instruments one allocator
//! per process, and a second, independent window cannot exist.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};

use super::{AllocationTraffic, INSTRUMENTED_SYSTEM, window::WindowState};

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

/// Total bytes the instrumenting allocator has been asked for.
///
/// `stats_alloc` adds the requested size on every allocation and the growth on
/// every growing reallocation, so the delta across a window measures memory
/// requested rather than memory held. Reading it allocates nothing, so it is
/// safe to sample inside a window.
fn total_allocated_bytes() -> usize { INSTRUMENTED_SYSTEM.stats().bytes_allocated }

/// An open allocation window that closes itself when it is dropped.
///
/// The gate is process-global, so it must close on every exit path, including
/// an unwind out of the measured operation. Closing from `Drop` is what makes
/// that unconditional: a caller cannot forget it, and a panic cannot skip it.
pub(super) struct CountingWindow {
    /// Allocated-byte total when the window opened.
    baseline_bytes: usize,
    /// Whether the window has already been closed, so `Drop` is a no-op then.
    is_closed: bool,
}

impl CountingWindow {
    /// Open the gate and record the baseline both counters measure from.
    pub(super) fn open() -> Self {
        // Open the gate before the baseline is sampled, and sample the closing
        // total before the gate closes. Anything allocated by another thread
        // outside that interval then falls on the far side of a boundary and
        // stays out of the reported delta.
        ALLOCATION_COUNTING_ENABLED.store(true, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        let baseline_bytes = total_allocated_bytes();
        let opened = WindowState::opened_at(total_allocations());
        ACCUMULATED_ALLOCATIONS.store(opened.accumulated, Ordering::SeqCst);
        BASELINE_ALLOCATIONS.store(opened.baseline, Ordering::SeqCst);
        Self {
            baseline_bytes,
            is_closed: false,
        }
    }

    /// Sample the closing totals and fold the window into the accumulated
    /// count.
    ///
    /// Returns `None` when the window is already closed, so calling this twice
    /// cannot count the same traffic again.
    pub(super) fn close(&mut self) -> Option<AllocationTraffic> {
        if self.is_closed {
            return None;
        }
        fence(Ordering::SeqCst);
        let window_total = total_allocations();
        let window_bytes = total_allocated_bytes();
        ALLOCATION_COUNTING_ENABLED.store(false, Ordering::SeqCst);
        accumulate_window_at(window_total);
        self.is_closed = true;
        Some(AllocationTraffic {
            operations: ACCUMULATED_ALLOCATIONS.load(Ordering::SeqCst),
            bytes: window_bytes.saturating_sub(self.baseline_bytes),
        })
    }
}

impl Drop for CountingWindow {
    fn drop(&mut self) {
        // The traffic is not wanted here: `count_allocation_traffic` has
        // already read it on the success path, and on a panic there is no
        // caller left to report to. Only the gate transition matters.
        let _closed = self.close();
    }
}

/// Read the process-global window state as a value.
fn snapshot() -> WindowState {
    WindowState {
        accumulated: ACCUMULATED_ALLOCATIONS.load(Ordering::SeqCst),
        baseline: BASELINE_ALLOCATIONS.load(Ordering::SeqCst),
        is_open: ALLOCATION_COUNTING_ENABLED.load(Ordering::SeqCst),
    }
}

/// Fold a window's delta into the accumulated count.
///
/// `window_total` is the allocator total sampled while the gate was still open,
/// so allocations that happen after the window closes cannot inflate the count.
/// The baseline advances to that same total, so folding twice without an
/// intervening window adds nothing the second time.
fn accumulate_window_at(window_total: usize) {
    let baseline = BASELINE_ALLOCATIONS.swap(window_total, Ordering::SeqCst);
    let delta = WindowState::delta(baseline, window_total);
    ACCUMULATED_ALLOCATIONS.fetch_add(delta, Ordering::SeqCst);
}

/// Reset the accumulated count without changing the gate.
pub(super) fn reset() {
    let reset = snapshot().reset_at(total_allocations());
    ACCUMULATED_ALLOCATIONS.store(reset.accumulated, Ordering::SeqCst);
    BASELINE_ALLOCATIONS.store(reset.baseline, Ordering::SeqCst);
}

/// Read the current allocation count, including a live window's delta.
pub(super) fn count() -> usize { snapshot().count_at(total_allocations()) }

/// Report whether a measurement window is currently open.
pub(super) fn is_open() -> bool { ALLOCATION_COUNTING_ENABLED.load(Ordering::SeqCst) }
