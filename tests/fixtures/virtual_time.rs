//! Virtual-time helpers shared by the paused-clock BDD fixtures.

use std::time::Duration;

/// One timer-wheel tick, the granularity Tokio rounds deadlines up to.
const TIMER_TICK_SLACK: Duration = Duration::from_millis(1);

/// Extend an explicit virtual-time advance so a timer of that duration expires.
///
/// Tokio converts a deadline to a wheel tick by rounding up to the end of the
/// millisecond (`deadline_to_tick` adds just under 1 ms), while the current
/// time converts by truncation. Advancing a paused clock by exactly a timer's
/// duration therefore leaves that timer one tick in the future, and it only
/// fires once the driver parks and auto-advances. A fixture that keeps yielding
/// instead of parking never lets that happen, so the frame-processing signal
/// would stay pending even though the pivot time has passed.
///
/// One extra millisecond clears the rounding boundary for any sub-millisecond
/// remainder. The slack is far smaller than the pacing delay under test, so a
/// delay that regressed beyond the requested advance still leaves the signal
/// pending and the scenario still fails.
pub(crate) fn advance_duration(millis: u64) -> Duration {
    Duration::from_millis(millis).saturating_add(TIMER_TICK_SLACK)
}
