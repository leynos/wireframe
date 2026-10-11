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
//! not a control for the counter itself. The evidence that the counter can move
//! is the *contended* case, which forces repeated entry to the slow path; the
//! uncontended case bounds how far the fast path is allowed to drift. Both
//! report [`pool_scheduler_task_spawns`] and the live task counts.
//!
//! # What is bracketed
//!
//! Pool construction is excluded from both counters. `bb8` connects eagerly, so
//! building a pool of size N leaves N connection tasks alive; measuring the
//! task gauge across construction would report `pool_size` as acquisition
//! churn. See `pool_once` in the `measurement::phase` submodule.
//!
//! # Known defect: sustained contention stalls the acquire path
//!
//! Contended acquisition is **not** reliable enough to time with Criterion.
//! Sustained contention intermittently leaves every contender stalled with
//! waiters still queued: no worker is servicing the queue, so no burst
//! completes. Both fairness policies stall, on a pool rebuilt per iteration and
//! on one long-lived pool, and with fresh and with reused handles, so the
//! trigger is contention itself. The observed first-stall burst numbers on the
//! named baseline machine are recorded in [`CONTENDED_BURST_TIMEOUT`].
//!
//! Contention also surfaces spurious `disconnected` errors: a contended round
//! can report `connection closed by peer` while the pool is still serving
//! traffic, reproducible on a long-lived pool at burst 913 of 20,000. Both
//! symptoms belong to the same acquire path and are pre-existing — they are
//! reproduced without any change to `scheduler.rs`, and a candidate fix that
//! suppressed the error was rejected because it converted a fast failure into
//! an intermittent hang, which is strictly worse.
//!
//! The defect is therefore *recorded*, not fixed, which is the correct scope
//! for issue #639: it establishes baselines before Epic #635 changes ownership
//! topology, and the pool acquire path is ADR 013 / issue #647's subject.
//! [`CONTENDED_BURST_TIMEOUT`] turns a stall into a reported error so a
//! benchmark or `cargo test --all-targets` run cannot block forever.
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
//!
//! [`pool_scheduler_task_spawns`]: wireframe::client::pool_scheduler_task_spawns

mod measurement;
mod workload;

#[cfg(test)]
mod tests;

pub use measurement::{CONTENDED_BURST_TIMEOUT, measure_pool_acquisition};
pub use workload::{
    AcquisitionMode,
    CONTENDED_ROUNDS_PER_CONTENDER,
    DEFAULT_ACQUISITIONS,
    DEFAULT_CONTENDERS,
    POOL_SIZES,
    PoolMeasurement,
    PoolWorkload,
};
