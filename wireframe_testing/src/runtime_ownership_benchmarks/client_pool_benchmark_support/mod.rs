//! Workloads and measurement for client pool acquisition.
//!
//! Pool acquisition has two regimes the issue asks to separate. An
//! **uncontended** acquire finds a free slot on the fast path and parks no
//! waiter. A **contended** acquire exhausts the pool, parks waiters in the
//! scheduler's queues, and waits for a permit race to observe a release.
//!
//! # Measured task churn
//!
//! Each pool starts exactly one scheduler task when it is constructed. Every
//! acquisition is a command to that task, and every release is a permit drop
//! that the task's in-flight race observes, so a measured acquisition phase
//! spawns **no** further tasks. `scheduler_spawns` is a lifetime counter on the
//! pool, so the assertion is a delta of zero over a phase; a non-zero delta now
//! means a regression to spawn-on-demand servicing.
//!
//! Earlier baselines recorded one or two spawns per burst, when every lease
//! release kicked a transient service worker. Those figures are retained in
//! `docs/runtime-ownership-baselines.md` as the pre-#647 comparison.
//!
//! # What the counters are compared against
//!
//! Because both regimes now report a zero spawn delta, the spawn counter is a
//! regression guard rather than a contention witness. The evidence that a burst
//! really contends is [`PoolMeasurement::capacity_waits`], which remains
//! discriminating in both directions: an uncontended workload must report zero,
//! because its single caller never exhausts the pool; a contended one must
//! report at least its contender count, because the burst empties the pool and
//! holds it empty while every contender asks for capacity. Contender counts and
//! start barriers only make waiting *possible* — a task that wins a permit can
//! finish several immediately-ready acquisitions before another is polled — so
//! the contended assertion counts the waiting rather than inferring it from the
//! shape of the burst.
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
//! # Contended acquisition and Criterion
//!
//! Contended acquisition remains outside Criterion's timed groups. A burst can
//! still stall, and a failure during warm-up would abort the whole run, so
//! [`CONTENDED_BURST_TIMEOUT`] bounds that case and the contended counters are
//! reported as observations after the summary.
//!
//! The spurious `disconnected` refusal recorded in the pre-#647 baselines is
//! retired by the persistent scheduler: it prunes cancelled waiters before
//! selecting one and reports typed pool-life-cycle errors instead. A 400-round
//! regression test asserts that no round is refused for that reason. The stall
//! was never reproduced on the post-#535 tree and remains bounded rather than
//! diagnosed. The figures and experiments behind both defects are in
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
    contended_fifo_pool_workloads,
};
