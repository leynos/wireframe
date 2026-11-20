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
//!
//! A window reports two quantities: the number of allocation operations and the
//! bytes requested. The operation count answers "did this stage allocate at
//! all", while the byte total lets a test bound a stage by its payload size
//! instead of an absolute count that moves with dependency churn.
//!
//! # Module layout
//!
//! The `gate` submodule owns the process-global state and the window that closes
//! on every exit path, `traffic` owns the value a window reports, and `harness`
//! owns the self-checks a benchmark binary must run before measuring. This
//! module is the public surface; the submodules are an implementation detail.

pub use std::alloc::System;

pub use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};

mod gate;
mod harness;
mod traffic;
mod window;

#[cfg(test)]
mod tests;

use gate::{CountingWindow, count as gate_count, is_open, reset as gate_reset};
pub use harness::{count_allocations_or_panic, verify_allocation_harness};
pub use traffic::AllocationTraffic;

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
pub fn reset_allocation_count() { gate_reset(); }

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
pub fn allocation_count() -> usize { gate_count() }

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
pub fn is_allocation_counting_enabled() -> bool { is_open() }

/// Run `operation` inside an allocation-counting window and return the count.
///
/// Opens the window, resets the counter, runs `operation`, then samples the
/// closing total before closing the window and returns the delta. The window is
/// closed even when `operation` fails, so a failed measurement cannot leave
/// counting enabled for the rest of the process.
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
    count_allocation_traffic(operation).map(|traffic| traffic.operations)
}

/// Run `operation` inside an allocation-counting window and report both the
/// operation count and the bytes requested.
///
/// The window is the same one [`count_allocations`] uses; the extra reading is
/// the allocator's byte total, which lets a caller bound a stage by its payload
/// size. The window closes on every exit path, including an unwind out of
/// `operation`.
///
/// # Errors
///
/// Returns the error string produced by `operation` after closing the window.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::benchmark_alloc_support::{
///     count_allocation_traffic,
///     is_allocation_counting_enabled,
/// };
///
/// let traffic = count_allocation_traffic(|| Ok(()))?;
/// std::hint::black_box(traffic);
///
/// // Only a binary that installs `INSTRUMENTED_SYSTEM` observes non-zero
/// // traffic. What holds either way is that the window closed.
/// assert!(!is_allocation_counting_enabled(), "the window must close");
/// # Ok::<(), String>(())
/// ```
pub fn count_allocation_traffic<F>(operation: F) -> Result<AllocationTraffic, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let mut window = CountingWindow::open();
    let outcome = operation();
    // Close before mapping the outcome, so a failed measurement still counts
    // the traffic it produced and cannot leave the gate open.
    let traffic = window.close().unwrap_or_default();
    outcome.map(|()| traffic)
}
