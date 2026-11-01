//! Per-pool observation of scheduler task churn and capacity waiting.
//!
//! A pool spawns exactly one scheduler task, when the pool is constructed.
//! Every later acquisition is a command to that task, so an acquire/drop burst
//! adds nothing to [`PoolTaskChurn::scheduler_spawns`]. The counter therefore
//! witnesses construction, not slow-path entry; [`PoolTaskChurn::capacity_waits`]
//! is the direct observation that callers genuinely queued for capacity.
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
//! single pool. They are written by the scheduler task alone — construction
//! records the spawn, and the acquire handler records a wait — so they add no
//! synchronization to a caller's path. See [`PoolTaskChurnObserver`] for the
//! no-op claim that matters — that observing changes nothing.

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
    /// Scheduler tasks spawned for this pool.
    ///
    /// Exactly one task serves a pool and it is started when the pool is
    /// constructed, so this counter is `1` for the pool's whole life. It is the
    /// observable half of the "one scheduler task per pool" invariant: an
    /// acquire/drop burst is mailbox traffic handled by the task that already
    /// runs, and adds nothing here.
    ///
    /// Read it as a construction witness, not as evidence that a slow path was
    /// reached. [`Self::capacity_waits`] is the counter that shows callers
    /// actually queued.
    pub scheduler_spawns: u64,
    /// Acquisitions that could not take a lease immediately and had to await
    /// capacity.
    ///
    /// This is the direct observation that a pool was genuinely exhausted: the
    /// scheduler grants from any slot with a free permit before parking a
    /// request, so a non-zero delta means this pool ran out of capacity while a
    /// caller was asking for it. Contender counts and start barriers only make
    /// that *possible*; this counts it happening, which is what lets a
    /// contended scenario assert that it contended rather than assume it.
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
/// Atomic so the observer can read them from any thread while the scheduler
/// task writes them from its own.
#[derive(Debug, Default)]
pub(crate) struct PoolTaskChurnCounts {
    /// Spawns recorded when a pool's scheduler task is started.
    scheduler_spawns: AtomicU64,
    /// Waiters that had to park because no slot had a free permit.
    capacity_waits: AtomicU64,
}

impl PoolTaskChurnCounts {
    /// Record one scheduler task spawn.
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
/// // The pool already owns its scheduler task, so acquiring and releasing
/// // leases spawns nothing.
/// assert_eq!(churn.scheduler_spawns, 0);
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
    pub(crate) const fn new(counts: Arc<PoolTaskChurnCounts>) -> Self { Self { counts } }

    /// Read the counters' current values.
    #[must_use]
    pub fn snapshot(&self) -> PoolTaskChurn { self.counts.snapshot() }
}
