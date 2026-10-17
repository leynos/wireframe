//! Workloads and measurement for client pool acquisition.
//!
//! Pool acquisition has two regimes the issue asks to separate. An
//! **uncontended** acquire finds a free slot on the fast path and parks no
//! waiter, so the pool never needs a worker to hand capacity on. A
//! **contended** acquire exhausts the pool, parks waiters, and enters the
//! scheduler slow path.
//!
//! # Measured task churn, and why the obvious claim is false
//!
//! It is tempting to assert that uncontended acquisition spawns no scheduler
//! task. Measurement shows otherwise, and the numbers are recorded rather than
//! asserted away. `PooledClientLease::drop` unconditionally calls
//! `notify_capacity_available` (`src/client/pool/lease.rs`), so *every* release
//! reaches `PoolScheduler::kick`, and `try_acquire_immediately` arms the lease
//! with a release handle even on the fast path
//! (`src/client/pool/client_pool.rs`). `kick` then wins its
//! idle-to-busy compare-exchange and spawns a worker that immediately finds an
//! empty queue and exits.
//!
//! So the observable contract is **bounded**, not zero: a burst spawns at least
//! one worker and at most one per idle-to-busy transition. Measured deltas for
//! acquisition counts of 1, 16, and 256 were 1, 1, and 2 respectively, across
//! pool sizes 1, 4, and 16. That per-release churn is a genuine cost of the
//! current ownership topology and is exactly the kind of finding issue #639
//! exists to record before #637/#638 change it.
//!
//! # What the counters are compared against
//!
//! Because a non-zero delta is expected on both paths, the uncontended case is
//! not a control for the spawn counter itself. The evidence that the counter can
//! move is the *contended* case, which forces repeated entry to the slow path;
//! the uncontended case bounds how far the fast path is allowed to drift.
//!
//! The second counter, [`PoolMeasurement::capacity_waits`], *is* discriminating
//! in both directions, and the two regimes assert opposite things about it. An
//! uncontended workload must report zero, because its single caller never
//! exhausts the pool; a contended one must report at least its contender count,
//! because the burst empties the pool and holds it empty while every contender
//! asks for capacity. Contender counts and start barriers only make waiting
//! *possible* — a task that wins a permit can finish several immediately-ready
//! acquisitions before another is polled — so the contended assertion counts the
//! waiting rather than inferring it from the shape of the burst.
//!
//! Both counters come from the pool under measurement rather than from the
//! process: `WireframeClientPool::task_churn_observer` hands back a read-only
//! handle onto that one pool's counters, so a delta cannot absorb churn from a
//! pool another test is driving concurrently. An earlier revision kept them in a
//! `static`, which made every delta process-wide and uninterpretable under a
//! parallel test binary.
//!
//! # What is bracketed
//!
//! Pool construction and warm-up are excluded from the timing, the counters, and
//! the live-task bracket. `bb8` connects eagerly, so building a pool of size N
//! leaves N connection tasks alive; measuring the task gauge across construction
//! would report `pool_size` as acquisition churn. The pool is built and warmed
//! once per call in `measurement`, and construction cost is reported separately
//! on [`PoolMeasurement::construction`] rather than discarded.
//!
//! # Known defects on the acquire path
//!
//! Contended acquisition is **not** reliable enough to time with Criterion,
//! for two separate reasons that were originally observed together.
//!
//! The first is a spurious `disconnected` error, which is **still live**: a
//! contended round can report `connection closed by peer` while the pool is
//! still serving traffic, reproducible on a long-lived pool. Measured on the
//! named baseline machine, it reaches roughly 1 round in 73 at 8 contenders on
//! the FIFO policy.
//!
//! The second is a stall — every contender parked with waiters still queued, so
//! no burst completes. This was observed against the pre-#535 scheduler and
//! **no longer reproduces**: two re-tests on the post-#535 tree found zero
//! stalls across 80,000 rounds at both recorded contender counts. The result is
//! one-sided, so read it as "no longer reproducible" rather than "fixed".
//! [`CONTENDED_BURST_TIMEOUT`] still bounds a stall, because a bounded failure
//! is worth more than an unbounded one and the disconnect defect still fails
//! bursts.
//!
//! Both are pre-existing: they reproduce without any change to `scheduler.rs`,
//! and a candidate fix that suppressed the error was rejected because it
//! converted a fast failure into an intermittent hang, which is strictly worse.
//! That rejection was measured against the pre-#535 scheduler, so it is not a
//! verdict on a repair built on the current state machine.
//!
//! The defects are therefore *recorded*, not fixed, which is the correct scope
//! for issue #639: it establishes baselines before Epic #635 changes ownership
//! topology, and the pool acquire path is ADR 013 / issue #647's subject. That
//! issue is a proposal to rework the path, not a defect tracker; the live
//! disconnect defect belongs to the same path.
//!
//! The figures and the experiments behind them are in
//! `docs/runtime-ownership-baselines.md`.
//!
//! # Transport
//!
//! Pool construction connects eagerly through `bb8`, and the crate exposes no
//! in-memory pool constructor, so this scenario needs a live peer. It uses the
//! loopback `PoolTestServer` (`src/test_helpers/pool_client.rs`), which is the
//! issue's documented transport-I/O exception. The peer only answers the
//! connection preamble; no frames are exchanged during a measurement, so the
//! timed work is acquisition and release rather than request round-trips.
//!
//! # Layout
//!
//! `workload` holds the scenarios as data; `measurement` drives them. That
//! module splits again, because the two regimes are separate control flows: the
//! bracketed acquisition phase in `measurement::phase`, and the racing burst in
//! `measurement::contended`.

mod measurement;
mod workload;

#[cfg(test)]
mod tests;

pub use measurement::{
    CONTENDED_BURST_TIMEOUT,
    CONTENDED_LATENCY_SAMPLES,
    ContendedLatency,
    PoolHarness,
    measure_contended_latency,
    measure_pool_acquisition,
};
pub use workload::{
    AcquisitionMode,
    CONTENDED_ROUNDS_PER_CONTENDER,
    DEFAULT_ACQUISITIONS,
    DEFAULT_CONTENDERS,
    POOL_SIZES,
    PoolMeasurement,
    PoolWorkload,
};
