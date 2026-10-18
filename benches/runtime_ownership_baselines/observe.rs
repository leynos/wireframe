//! The counters reported as `tracing` events after the timed groups close.
//!
//! A timing alone cannot say which path a scenario took, and two of the
//! scenarios that matter most here cannot be timed at all: the awaited push
//! scenarios may or may not have parked, and contended pool acquisition is
//! unreliable enough that Criterion would block on it. Both are still worth
//! observing, so their counters are sampled in a bounded way and published as
//! structured events rather than entered into the Criterion report.

use tokio::runtime::Runtime;
use wireframe_testing::runtime_ownership_benchmarks::{
    CONTENDED_BURST_TIMEOUT,
    CONTENDED_LATENCY_SAMPLES,
    ContendedLatency,
    measure_contended_latency,
    measure_push,
};

use crate::scenarios::{contended_pool_workloads, expect_bench, push_workloads};

/// Emit each push workload's awaited-wait counters as an observation.
///
/// A timing alone cannot say which path a drained scenario took: a fast run may
/// have found the queue's fast path or its back-pressure path, and the two are
/// not comparable. The wait counters say which one it was, and they are not
/// timings, so `tracing` carries them rather than the Criterion report.
///
/// Taken from a one-iteration measurement, once per workload, rather than from
/// `time_push`: that runs on every Criterion iteration, and an event there
/// would bury the counters it exists to publish. The workloads are the ones
/// `push_scope` times, so the observation describes the same run.
pub(crate) fn observe_push_waits(runtime: &Runtime) {
    for workload in push_workloads() {
        let measurement = expect_bench(
            runtime.block_on(measure_push(workload, 1)),
            "push wait observation failed",
        );
        tracing::info!(
            workload = %workload.label(),
            awaited_capacity_waits = measurement.awaited_capacity_waits,
            capacity_probes = measurement.capacity_probes,
            "push awaited-wait observation"
        );
    }
}

/// Emit a completed contended sampling run's distribution and counters.
///
/// A single elapsed value was the previous shape of this event, and it was
/// ambiguous: the contended acquire path fails intermittently, so one figure
/// could be the duration of a burst that produced leases or a number for a run
/// that produced none. The distribution separates the two, and the failure
/// count says how much of the run the percentiles do not describe.
///
/// The churn counters are reported per run rather than per sample: they are the
/// pool's cumulative counters read across the sampling window, so a
/// per-burst figure would imply a precision the counters do not have.
///
/// Kept as its own function because a `tracing` macro expands to an
/// `enabled`/not-enabled conditional, and two of them inside one `match` push
/// the caller past the workspace's cognitive-complexity limit.
fn report_contended_success(label: &str, latency: &ContendedLatency) {
    let churn = latency.churn();
    tracing::info!(
        workload = %label,
        burst_timeout = ?CONTENDED_BURST_TIMEOUT,
        attempts = latency.successes() + latency.failure_count(),
        successes = latency.successes(),
        failures = latency.failure_count(),
        scheduler_spawns = churn.scheduler_spawns,
        capacity_waits = churn.capacity_waits,
        min = ?latency.min(),
        median = ?latency.median(),
        p90 = ?latency.p90(),
        max = ?latency.max(),
        mean = ?latency.mean(),
        "contended pool acquisition distribution"
    );
    for failure in latency.failures() {
        report_contended_failure(label, failure);
    }
}

/// Emit the reason a contended burst did not complete as a warning.
fn report_contended_failure(label: &str, error: &str) {
    tracing::warn!(
        workload = %label,
        burst_timeout = ?CONTENDED_BURST_TIMEOUT,
        %error,
        "contended pool acquisition burst did not complete; known acquire-path defect, see \
         docs/runtime-ownership-baselines.md"
    );
}

/// Emit the outcome of one contended sampling run as a structured event.
fn report_contended_outcome(label: &str, outcome: Result<ContendedLatency, String>) {
    match outcome {
        Ok(latency) => report_contended_success(label, &latency),
        Err(error) => report_contended_failure(label, &error),
    }
}

/// Sample contended acquisition as a bounded, untimed observation.
///
/// This deliberately does not register a Criterion group. Sustained contention
/// intermittently stalls the pool acquire path, and Criterion would block during
/// warm-up sampling a scenario that never returns — taking the whole benchmark
/// run with it. The measurement helper converts that stall into a recorded
/// failure after [`CONTENDED_BURST_TIMEOUT`], so the observation always
/// terminates and the failures are reported beside the samples rather than in
/// place of them.
///
/// The churn counters are the issue's task-churn evidence at this scope:
/// whether contended bursts genuinely enter the scheduler's slow path, and what
/// task churn they cost. They are reported per run, beside the distribution
/// rather than as a substitute for it.
pub(crate) fn observe_contended_acquisition(runtime: &Runtime) {
    for workload in contended_pool_workloads() {
        let label = workload.label();
        let outcome = runtime.block_on(measure_contended_latency(
            workload,
            CONTENDED_LATENCY_SAMPLES,
        ));
        report_contended_outcome(&label, outcome);
    }
}

/// Install a subscriber for the benchmark's own events only.
///
/// The contended-acquisition observation reports counters as structured events,
/// and an event with no subscriber is silently discarded. The filter is set to
/// this binary's own target rather than left at a global level: the benchmark
/// drives connection startup, which logs an INFO span per prepared connection,
/// and at 256 connections per iteration that noise buries the counters the
/// observation exists to report. Criterion owns the timing output; `tracing`
/// carries only the counters, which are not timings and must not be read as
/// such.
pub(crate) fn init_observation_subscriber() {
    use tracing_subscriber::filter::LevelFilter;

    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(LevelFilter::INFO)
        .with_env_filter(tracing_subscriber::EnvFilter::new(env!("CARGO_CRATE_NAME")))
        .init();
}
