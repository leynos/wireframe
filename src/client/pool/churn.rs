//! Per-pool observation of scheduler task churn and capacity waiting.
//!
//! [`PoolScheduler::kick`](super::scheduler::PoolScheduler::kick) spawns a
//! worker task only when it wins a compare-exchange from idle to servicing, so
//! the number of spawns measures how often the scheduler slow path was entered.
//! Exposing the count lets benchmarks bracket the spawns an uncontended
//! acquire/drop burst can cause, and prove that a contended scenario genuinely
//! exercised the path. It does not let them assert that an uncontended run
//! spawns nothing; see [`PoolTaskChurn`].
//!
//! # Why the counters are per-pool and not process-wide
//!
//! An earlier revision kept these counts in a `static`, which made them
//! process-global. That is unusable from a test binary: every pool in the
//! process — including pools another test is driving concurrently on another
//! thread — contributed to the same number, so a delta read across one
//! workload silently absorbed every other workload's churn. Serializing only
//! the assertion would not have fixed it, because the contamination happens
//! while the *other* workload runs, not while the assertion reads. Moving the
//! counters onto the scheduler makes a delta unambiguous: it can only be
//! attributed to the pool the observer was taken from.
//!
//! The counters live in their own module rather than in `scheduler.rs` so the
//! scheduler module stays inside the workspace's 400-line module limit.
//!
//! # Cost
//!
//! Both counters are relaxed atomic increments on a cache line owned by a
//! single pool. They sit on the acquire path, but only on the path that is
//! already taking a lock and racing futures, so they do not change the shape of
//! the path being measured. See [`PoolTaskChurnObserver`] for the no-op claim
//! that matters — that observing changes nothing.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Counters describing one pool's scheduler activity.
///
/// A plain struct of integers rather than a handle, so a caller can snapshot
/// it, store it, and subtract one snapshot from another without holding the
/// pool alive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolTaskChurn {
    /// Scheduler worker tasks spawned by
    /// [`PoolScheduler::kick`](super::scheduler::PoolScheduler::kick).
    ///
    /// This is the observable half of the "one scheduler task at a time"
    /// invariant: `kick` wins its compare-exchange at most once per
    /// idle-to-busy transition, so concurrent releases can never spawn more
    /// than one worker for the same transition.
    ///
    /// The count is a *bound*, not a floor, for an uncontended run.
    /// [`PooledClientLease::drop`](super::lease::PooledClientLease) notifies the
    /// scheduler unconditionally, so a release reaches `kick` even when nobody
    /// is waiting and the pool never exhausts. A burst of `n` acquire/release
    /// cycles therefore spawns between 0 and `2n` workers: at most one per
    /// transition, and possibly none at all, because a worker the burst itself
    /// woke still holds servicing while the remaining releases arrive — every
    /// one of those loses the compare-exchange. Assert against that bracket
    /// rather than asserting the count does not move; only a contended scenario
    /// proves the slow path is reachable at all.
    ///
    /// A lower bound of one is sound only when the scheduler is known to be idle
    /// as the burst begins. Do not assume it: a preceding phase can leave a
    /// worker running.
    pub scheduler_spawns: u64,
    /// Acquisitions that could not take a lease immediately and had to await
    /// capacity.
    ///
    /// This is the direct observation that a pool was genuinely exhausted:
    /// [`PoolScheduler::acquire_for_handle`](super::scheduler::PoolScheduler)
    /// fast-paths when any slot has a free permit, so a non-zero delta means
    /// this pool ran out of capacity while a caller was asking for it. Contender
    /// counts and start barriers only make that *possible*; this counts it
    /// happening, which is what lets a contended scenario assert that it
    /// contended rather than assume it.
    pub capacity_waits: u64,
}

impl PoolTaskChurn {
    /// Counters accumulated since an earlier snapshot.
    ///
    /// Saturates rather than wrapping. The counters only ever increase, so a
    /// larger earlier reading means the two snapshots came from different
    /// pools — a caller error this cannot repair, but which it must not turn
    /// into a huge bogus delta.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::PoolTaskChurn;
    ///
    /// let before = PoolTaskChurn {
    ///     scheduler_spawns: 3,
    ///     capacity_waits: 1,
    /// };
    /// let after = PoolTaskChurn {
    ///     scheduler_spawns: 5,
    ///     capacity_waits: 4,
    /// };
    /// assert_eq!(
    ///     after.since(before),
    ///     PoolTaskChurn {
    ///         scheduler_spawns: 2,
    ///         capacity_waits: 3
    ///     }
    /// );
    /// ```
    #[must_use]
    pub const fn since(self, before: Self) -> Self {
        Self {
            scheduler_spawns: self
                .scheduler_spawns
                .saturating_sub(before.scheduler_spawns),
            capacity_waits: self.capacity_waits.saturating_sub(before.capacity_waits),
        }
    }

    /// Add two readings, saturating at the counters' own ceiling.
    ///
    /// Used to fold several phases' churn together when they all ran against
    /// the same pool, so a total is the sum of the parts rather than the last
    /// part standing in for the whole.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::PoolTaskChurn;
    ///
    /// let first = PoolTaskChurn {
    ///     scheduler_spawns: 1,
    ///     capacity_waits: 0,
    /// };
    /// let second = PoolTaskChurn {
    ///     scheduler_spawns: 2,
    ///     capacity_waits: 5,
    /// };
    /// assert_eq!(
    ///     first.saturating_add(second),
    ///     PoolTaskChurn {
    ///         scheduler_spawns: 3,
    ///         capacity_waits: 5
    ///     }
    /// );
    /// ```
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self {
            scheduler_spawns: self.scheduler_spawns.saturating_add(other.scheduler_spawns),
            capacity_waits: self.capacity_waits.saturating_add(other.capacity_waits),
        }
    }
}

/// The live counters behind a [`PoolTaskChurnObserver`].
///
/// Atomic because the scheduler is shared across threads and increments from
/// whichever task is acquiring or releasing.
#[derive(Debug, Default)]
pub(crate) struct PoolTaskChurnCounts {
    /// Spawns recorded by `PoolScheduler::kick`.
    scheduler_spawns: AtomicU64,
    /// Waiters that had to park because no slot had a free permit.
    capacity_waits: AtomicU64,
}

impl PoolTaskChurnCounts {
    /// Record one scheduler worker-task spawn.
    pub(crate) fn record_scheduler_spawn(&self) {
        self.scheduler_spawns.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one acquisition that had to await capacity.
    pub(crate) fn record_capacity_wait(&self) {
        self.capacity_waits.fetch_add(1, Ordering::Relaxed);
    }

    /// Read both counters without any ordering stronger than the increments need.
    fn snapshot(&self) -> PoolTaskChurn {
        PoolTaskChurn {
            scheduler_spawns: self.scheduler_spawns.load(Ordering::Relaxed),
            capacity_waits: self.capacity_waits.load(Ordering::Relaxed),
        }
    }
}

/// A read-only handle onto one pool's task-churn counters.
///
/// Returned by `WireframeClientPool::task_churn_observer`. Cloning it is cheap
/// and shares the same counters; dropping every clone drops the counters once
/// the pool itself is gone.
///
/// Observing is passive. Taking an observer, snapshotting it, and dropping it
/// performs no acquire, spawns no task, and mutates no counter, so a bracket
/// built from snapshots cannot perturb the pool it measures. That matters
/// because the counts being bracketed are themselves affected by any
/// acquisition the bracket's own instrumentation might perform.
///
/// # Examples
///
/// ```rust,no_run
/// # async fn example(pool: &wireframe::client::WireframeClientPool<
/// #     wireframe::serializer::BincodeSerializer, (), (),
/// # >) -> Result<(), String> {
/// let observer = pool.task_churn_observer();
/// let before = observer.snapshot();
/// // ... exercise pooled handles ...
/// let churn = observer.snapshot().since(before);
/// // A bound, not a floor: a worker left servicing by an earlier phase can make
/// // every release in this window lose the idle-to-busy compare-exchange.
/// assert!(churn.scheduler_spawns <= 2 * 4);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct PoolTaskChurnObserver {
    /// Counters belonging to the pool this observer was taken from.
    counts: Arc<PoolTaskChurnCounts>,
}

impl PoolTaskChurnObserver {
    /// Wrap a pool's counters.
    pub(crate) fn new(counts: Arc<PoolTaskChurnCounts>) -> Self { Self { counts } }

    /// Read the counters' current values.
    #[must_use]
    pub fn snapshot(&self) -> PoolTaskChurn { self.counts.snapshot() }
}
