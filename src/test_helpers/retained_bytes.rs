//! Test-only observation of the bytes one connection retains.
//!
//! A socket-level back-pressure test can show that a peer blocks without
//! showing what the server was holding when it did. These endpoints let a test
//! arm one aggregate limit and read the highest total recorded against it, so
//! the assertion names the budget it is checking instead of inferring it from
//! timing.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Aggregate limit the observing test armed; zero when nothing is armed.
static ARMED_LIMIT: AtomicUsize = AtomicUsize::new(0);
/// Highest retained total recorded against [`ARMED_LIMIT`].
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// Start observing connections whose aggregate limit is `limit`.
///
/// Recording is scoped by the limit so another scenario running concurrently in
/// the same test binary, with its own budgets, cannot contribute to this peak.
pub fn arm(limit: usize) {
    PEAK.store(0, Ordering::SeqCst);
    ARMED_LIMIT.store(limit, Ordering::SeqCst);
}

/// Stop observing, leaving the recorded peak readable.
pub fn disarm() { ARMED_LIMIT.store(0, Ordering::SeqCst); }

/// Report the bytes one connection retains while a chunk waits for admission.
///
/// The total covers every holder the connection counts: chunks queued or
/// reserved on the byte account, bytes retained by assembly, fragmentation and
/// parked messages, and the chunk still awaiting admission.
pub(crate) fn record(limit: usize, retained: usize) {
    let armed = ARMED_LIMIT.load(Ordering::SeqCst);
    if armed != 0 && armed == limit {
        PEAK.fetch_max(retained, Ordering::SeqCst);
    }
}

/// Return the highest retained total recorded since [`arm`].
///
/// Zero means no connection with the armed limit reached an admission drive,
/// which is a failed observation rather than a connection that held nothing.
pub fn peak() -> usize { PEAK.load(Ordering::SeqCst) }
