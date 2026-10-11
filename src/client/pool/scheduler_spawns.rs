//! Observation of pool scheduler worker-task creation.
//!
//! [`PoolScheduler::kick`](super::scheduler::PoolScheduler::kick) spawns a
//! worker task only when it wins a compare-exchange from idle to servicing, so
//! the number of spawns is a process-wide measure of how often the scheduler
//! slow path was entered. Exposing the count lets benchmarks bracket the spawns
//! an uncontended acquire/drop burst can cause, and prove that a contended
//! scenario genuinely exercised the path. It does not let them assert that an
//! uncontended run spawns nothing; see [`POOL_SCHEDULER_TASK_SPAWNS`].
//!
//! The counter lives in its own module rather than in `scheduler.rs`, matching
//! the existing `connection::counter` arrangement, so the scheduler module
//! stays inside the workspace's 400-line module limit.

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide count of scheduler worker tasks spawned by
/// [`PoolScheduler::kick`](super::scheduler::PoolScheduler::kick).
///
/// This is the observable half of the "one scheduler task at a time"
/// invariant: `kick` wins its compare-exchange at most once per idle-to-busy
/// transition, so concurrent releases can never spawn more than one worker for
/// the same transition.
///
/// The counter is a *bound*, not a zero, for an uncontended run.
/// `PooledClientLease::drop` notifies the scheduler unconditionally, so a
/// release reaches `kick` even when nobody is waiting and the pool never
/// exhausts. A burst of `n` acquire/release cycles therefore spawns between 1
/// and `2n` workers — never 0, and never more than the transition count. Assert
/// against that bracket rather than asserting the count does not move; only a
/// contended scenario proves the slow path is reachable at all.
///
/// A plain counter rather than a `metrics` gauge because benchmark binaries
/// install no recorder, which would make a `metrics` counter unobservable
/// exactly where it is needed.
static POOL_SCHEDULER_TASK_SPAWNS: AtomicU64 = AtomicU64::new(0);

/// Record one scheduler worker-task spawn.
pub(super) fn record_scheduler_task_spawn() {
    POOL_SCHEDULER_TASK_SPAWNS.fetch_add(1, Ordering::Relaxed);
}

/// Return the number of pool scheduler worker tasks spawned in this process.
///
/// Useful for asserting that uncontended acquire/drop cycles do not create
/// additional scheduler tasks, and for confirming that a contended scenario
/// genuinely exercised the slow path.
///
/// # Examples
///
/// ```rust,no_run
/// # use wireframe::client::pool_scheduler_task_spawns;
/// let before = pool_scheduler_task_spawns();
/// // ... exercise a pooled client ...
/// assert!(pool_scheduler_task_spawns() >= before);
/// ```
#[must_use]
pub fn pool_scheduler_task_spawns() -> u64 { POOL_SCHEDULER_TASK_SPAWNS.load(Ordering::Relaxed) }
