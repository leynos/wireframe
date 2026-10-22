//! The workload parameters each Criterion group measures, and the budgets they
//! run under.
//!
//! Everything here is data: constants and the builders that cross them into
//! workload lists. Keeping the scenarios declarative is what lets `driver`
//! register all four groups through one path, and what keeps the recorded
//! baseline comparable across an ownership refactor.

use std::time::Duration;

use wireframe_testing::runtime_ownership_benchmarks::{
    AcquisitionMode,
    ActorWorkload,
    CancellationMode,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_PUSH_QUEUE_CAPACITY,
    POOL_SIZES,
    PoolWorkload,
    PushScenario,
    PushWorkload,
    StartupPath,
    StartupWorkload,
    contended_fifo_pool_workloads,
};

/// Criterion sample count for every runtime-ownership group.
///
/// Lower than Criterion's default of 100 because each iteration here is
/// expensive: it builds queues, spawns tasks, or opens real loopback
/// connections. A hundred samples per scenario would spend minutes apiece
/// without making the recorded baseline any more useful.
pub(crate) const SAMPLE_SIZE: usize = 20;

/// Criterion measurement time for every runtime-ownership group.
pub(crate) const MEASUREMENT_TIME: Duration = Duration::from_secs(10);

/// Frame counts exercised by the actor drain group.
const ACTOR_FRAME_COUNTS: [usize; 3] = [256, 512, 1024];

/// Cancellation regimes exercised by the actor drain group.
const ACTOR_CANCELLATIONS: [CancellationMode; 3] = [
    CancellationMode::Disabled,
    CancellationMode::CancelledBeforeRun,
    CancellationMode::CancelledDuringDrain,
];

/// Per-queue capacity for every actor drain workload.
///
/// At least half the largest frame count, so no workload blocks on
/// back-pressure and each measures the regime its label claims.
const ACTOR_QUEUE_CAPACITY: usize = 512;

/// Queue capacity for the overflow push workloads.
///
/// Deliberately small: the overflow scenarios fill the queue to capacity first,
/// so a large capacity would spend most of the iteration in the fill rather
/// than in the non-blocking branch under measurement.
const PUSH_OVERFLOW_CAPACITY: usize = 8;

/// Sends per iteration in the overflow push workloads.
const PUSH_OVERFLOW_SENDS: usize = 256;

/// Route counts exercised by the connection startup group.
///
/// The application shape is held small on purpose: the axis this scope has to
/// resolve is how the two preparation paths differ as connections multiply, and
/// a route count large enough to dominate the figure would obscure that.
/// [`STARTUP_CONNECTION_COUNTS`] carries the comparison.
const STARTUP_ROUTE_COUNTS: [usize; 2] = [1, 2];

/// Connection counts exercised by the connection startup group.
///
/// Crossed with [`STARTUP_ROUTE_COUNTS`], so the group covers 1, 4, and 16
/// connections. The largest case exists because the two paths only diverge with
/// connection count: the prepared root pays preparation once and amortizes it,
/// so a one- and a four-connection run show a difference but not its shape. Sixteen
/// is the same ceiling the actor and pool scopes use, which keeps the four
/// scopes' "large" cases at one scale rather than four unrelated ones. It is not
/// a claim about a particular deployment; it is the point on this axis where the
/// per-connection path's linear growth is unambiguous against the prepared
/// root's sub-linear growth.
const STARTUP_CONNECTION_COUNTS: [usize; 3] = [1, 4, 16];

/// Pool size used for the contended acquisition workloads.
///
/// Contenders must outnumber slots for the pool to be exhausted, so this stays
/// small enough that [`CONTENDED_POOL_CONTENDERS`] exceeds it comfortably.
const CONTENDED_POOL_SIZE: usize = 4;

/// Contending callers used for the contended acquisition workloads.
const CONTENDED_POOL_CONTENDERS: usize = 16;

/// Convert a workload count to the `u64` Criterion's throughput expects.
///
/// `try_from` rather than `as`: the conversion is lossless only on a 64-bit
/// host, and a silently truncated element count would corrupt the reported
/// throughput rather than fail visibly. Saturating keeps the label usable on an
/// exotic target instead of panicking inside a benchmark.
pub(crate) fn elements(count: usize) -> u64 { u64::try_from(count).unwrap_or(u64::MAX) }

/// Unwrap a benchmark helper's result, terminating with a diagnostic on error.
///
/// A Criterion iteration closure cannot return a `Result`, and a failed
/// workload cannot produce a meaningful sample. Stopping the benchmark is
/// therefore the only honest outcome: recording a time for a scenario that did
/// not run would put a fabricated number in the baseline.
pub(crate) fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Send count for the push no-back-pressure control.
///
/// Must not exceed the queue capacity, because that is what makes the control
/// a control: a producer sending no more frames than the queue holds cannot
/// park whatever the consumer is doing. See [`PUSH_QUEUE_CAPACITY`].
const PUSH_CONTROL_SENDS: usize = 64;

/// Send count for the push throughput workloads.
///
/// Deliberately larger than the queue capacity, so the producer can outrun a
/// consumer. Whether it does is the observation, reported through the
/// measurement's `awaited_capacity_waits` counter rather than asserted.
const PUSH_DRAINED_SENDS: usize = 1024;

/// Queue capacity for the push backpressure workload.
///
/// Small, and much smaller than [`PUSH_DRAINED_SENDS`], because the scenario
/// has no consumer: the queue must fill within the run for the producer to park
/// at all. A capacity that could hold the whole run would measure the
/// empty-queue path under a backpressure label.
const PUSH_BACKPRESSURE_CAPACITY: usize = 8;

/// The actor-drain workloads: a cancellation regime per frame count.
pub(crate) fn actor_workloads() -> Vec<ActorWorkload> {
    ACTOR_CANCELLATIONS
        .into_iter()
        .flat_map(|cancellation| {
            ACTOR_FRAME_COUNTS
                .into_iter()
                .map(move |frames| ActorWorkload {
                    frames,
                    queue_capacity: ACTOR_QUEUE_CAPACITY,
                    cancellation,
                })
        })
        .collect()
}

/// The push workloads the send group measures.
///
/// The awaited scenarios carry the throughput case and so get the full send
/// count. The three overflow scenarios share a small queue and a shorter run
/// because each exercises one non-blocking branch behind a policy check.
pub(crate) fn push_workloads() -> Vec<PushWorkload> {
    let mut workloads = vec![
        // The no-back-pressure control: the queue holds the whole run, so a
        // zero wait count is a property of the workload rather than luck.
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: PUSH_CONTROL_SENDS,
            sends: PUSH_CONTROL_SENDS,
        },
        // The backpressure case: nothing drains the queue, so the producer must
        // park against a full one. Kept separate from the drained scenarios
        // because a draining consumer does not guarantee the producer never
        // waits, and `reported_queue_full` does not observe awaited waits at all.
        PushWorkload {
            scenario: PushScenario::BackpressureHigh,
            queue_capacity: PUSH_BACKPRESSURE_CAPACITY,
            sends: PUSH_CONTROL_SENDS,
        },
        // The throughput case: the queue cannot hold the run, so the producer
        // may park and the reported wait count says whether it did.
        PushWorkload {
            scenario: PushScenario::DrainedHigh,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: PUSH_DRAINED_SENDS,
        },
        PushWorkload {
            scenario: PushScenario::DrainedLow,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: PUSH_DRAINED_SENDS,
        },
    ];

    for scenario in [
        PushScenario::OverflowReturnError,
        PushScenario::OverflowDrop,
        PushScenario::OverflowWarnAndDrop,
    ] {
        workloads.push(PushWorkload {
            scenario,
            queue_capacity: PUSH_OVERFLOW_CAPACITY,
            sends: PUSH_OVERFLOW_SENDS,
        });
    }

    workloads
}

/// The connection-startup workloads the startup group times.
///
/// Both preparation paths are crossed with every route and connection count, in
/// the order the recorded baseline table lists them. The path is the outer loop
/// so the two paths' rows stay grouped in the Criterion report rather than
/// interleaved, which is what makes the per-connection-versus-prepared contrast
/// readable at a glance.
pub(crate) fn startup_workloads() -> Vec<StartupWorkload> {
    [StartupPath::PerConnectionFactory, StartupPath::PreparedRoot]
        .into_iter()
        .flat_map(|path| {
            STARTUP_ROUTE_COUNTS.into_iter().flat_map(move |routes| {
                STARTUP_CONNECTION_COUNTS
                    .into_iter()
                    .map(move |connections| StartupWorkload {
                        path,
                        routes,
                        middleware_layers: DEFAULT_MIDDLEWARE_LAYERS,
                        connections,
                    })
            })
        })
        .collect()
}

/// The uncontended pool workloads the acquisition group times.
///
/// Only uncontended acquisition is timed. Contended acquisition is excluded
/// because it is not reliable enough to sample: sustained contention
/// intermittently stalls the acquire path outright, so a Criterion run would
/// block during warm-up rather than record a number. That defect is pre-existing
/// and belongs to ADR 013 / issue #647; `observe_contended_acquisition` records
/// its counters under a timeout instead of timing it. See
/// `docs/runtime-ownership-baselines.md`.
pub(crate) fn uncontended_pool_workloads() -> Vec<PoolWorkload> {
    POOL_SIZES
        .into_iter()
        .map(|pool_size| PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            ..PoolWorkload::default()
        })
        .collect()
}

/// Contended workloads reported as bounded timing observations.
///
/// FIFO runs cover all baseline pool sizes. Round-robin remains the existing
/// size-four comparison so its label and workload stay directly comparable.
pub(crate) fn contended_pool_workloads() -> Vec<PoolWorkload> {
    let mut workloads = contended_fifo_pool_workloads().to_vec();
    workloads.push(PoolWorkload {
        pool_size: CONTENDED_POOL_SIZE,
        mode: AcquisitionMode::ContendedRoundRobin,
        contenders: CONTENDED_POOL_CONTENDERS,
        ..PoolWorkload::default()
    });
    workloads
}
