//! Self-checks and panic-free adapters for benchmark binaries.
//!
//! A benchmark binary has no test harness to fail for it: a counting gate that
//! never opens would make every later baseline a silent zero. These helpers
//! turn that failure into a loud one before any measurement starts, and adapt a
//! failed measurement into a panic that fires after the window has closed.

use super::{AllocationTraffic, count_allocation_traffic, count_allocations};

/// Buffer size the harness self-check allocates, large enough that the
/// allocator cannot merge it with bookkeeping traffic but small enough to stay
/// below any payload class the pipeline invariants measure.
const HARNESS_PROBE_BYTES: usize = 1024;

/// Prove that the allocation harness resolves what the pipeline invariants
/// depend on, stopping the calling benchmark loudly if it cannot.
///
/// A benchmark binary must call this before measuring anything: the counters
/// are process-global, and a gate stuck open or never opened would turn every
/// later assertion into a silent pass. Three checks:
///
/// 1. a closure that allocates nothing reports zero operations and zero bytes, catching a gate
///    stuck open;
/// 2. a closure that allocates a known buffer reports at least one operation and at least the
///    buffer's size in bytes, catching a gate that never opens; and
/// 3. growing an existing buffer reports more operations and more bytes than allocating it once,
///    proving reallocations are counted rather than only fresh allocations.
///
/// # Panics
///
/// Panics with the failing check's description when the harness is not
/// reporting correctly.
pub fn verify_allocation_harness() {
    let idle = measured_traffic("idle window", || Ok(()));
    assert_eq!(
        idle,
        AllocationTraffic::default(),
        "an allocation-free closure must report no traffic; a non-zero reading means the counting \
         gate is stuck open or something else ran during the window"
    );

    let single = measured_traffic("single-allocation probe", || {
        let buffer: Vec<u8> = Vec::with_capacity(HARNESS_PROBE_BYTES);
        std::hint::black_box(&buffer);
        Ok(())
    });
    assert!(
        single.operations > 0 && single.bytes >= HARNESS_PROBE_BYTES,
        "allocating a {HARNESS_PROBE_BYTES}-byte buffer must be observed; a zero reading means \
         the counting gate never opens and every baseline would be a silent zero: {} operations, \
         {} bytes",
        single.operations,
        single.bytes
    );

    let growth = measured_traffic("growth-allocation probe", || {
        let mut buffer: Vec<u8> = Vec::with_capacity(HARNESS_PROBE_BYTES);
        buffer.resize(HARNESS_PROBE_BYTES * 4, 0);
        std::hint::black_box(&buffer);
        Ok(())
    });
    assert!(
        growth.operations > single.operations && growth.bytes > single.bytes,
        "growing a buffer must cost at least one more counted operation and more bytes than \
         allocating it once: {growth:?} with growth vs {single:?} without; equality means \
         reallocations are not counted"
    );
}

/// Run one harness probe, panicking with `description` when it fails.
fn measured_traffic<F>(description: &str, operation: F) -> AllocationTraffic
where
    F: FnOnce() -> Result<(), String>,
{
    match count_allocation_traffic(operation) {
        Ok(traffic) => traffic,
        Err(err) => panic!("{description} failed: {err}"),
    }
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
