//! The shared runtime helpers, the workload defaults, and the label contract.
//!
//! Every case in this target drives a real runtime through one of the three
//! `block_on` flavours below. They live here rather than beside the cases that
//! use them because the pool cases need both the multi-thread and the
//! current-thread variant, and importing them through one module is what keeps
//! each case file reading as a list of assertions.

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{
    ActorWorkload,
    DEFAULT_CONNECTIONS,
    DEFAULT_FRAME_COUNT,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_ROUTES,
    PoolMeasurement,
    PoolWorkload,
    PushWorkload,
    StartupWorkload,
};

/// Workload iterations used by the assertion tests.
///
/// One suffices for every count asserted here: each helper reports the *last*
/// iteration's counters, so repeating an iteration would only re-assert the same
/// expected value. Iteration-count behaviour has its own cases.
pub(super) const ITERATIONS: u64 = 1;

/// Attempts allowed before a contended burst is reported as a failure.
///
/// The contended acquire path intermittently fails a burst with a spurious
/// `disconnected`; on the named baseline machine the post-#535 rate reaches
/// roughly one burst in 73 at 8 contenders on the FIFO policy. A burst could
/// also stall until [`CONTENDED_BURST_TIMEOUT`] converts the stall into an
/// error, though the stall has not reproduced since the pre-#535 scheduler and
/// is retained here only so a recurrence is retried rather than fatal. Both are
/// pre-existing defects recorded against issue #647. The case below asserts
/// what a contended burst *can* produce, so it retries a bounded number of
/// times instead of failing the suite on an interleaving unrelated to the
/// change under review.
///
/// [`CONTENDED_BURST_TIMEOUT`]: wireframe_testing::runtime_ownership_benchmarks::CONTENDED_BURST_TIMEOUT
const CONTENDED_BURST_ATTEMPTS: usize = 8;

/// Run a future to completion on a fresh multi-thread runtime.
///
/// Multi-thread for the workload assertions because the contended pool cases
/// park callers on a barrier and need real parallelism to exhaust the pool. The
/// one case that reads the live-task gauge instead uses
/// [`block_on_current_thread`], for the reason documented there.
///
/// Returns `Result` rather than panicking on a runtime-construction failure:
/// this helper lives outside any `#[test]`, so the workspace's test-only
/// `expect_used` allowance does not cover it. The error type is the workloads'
/// own `String` so the result flattens into the one `Result` each caller
/// already matches on, which keeps the failure surfaced without adding a panic
/// path to the helper.
pub(super) fn block_on<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    run_on(tokio::runtime::Builder::new_multi_thread(), future)
}

/// Build `builder`, enable all drivers, and run `future` on it.
///
/// Both `block_on` flavours differ only in which runtime they construct, so the
/// build-and-drive tail lives here once: a runtime that failed to build must
/// surface as the same `String` whichever flavour asked for it.
fn run_on<F, T>(mut builder: tokio::runtime::Builder, future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    builder
        .enable_all()
        .build()
        .map_err(|error| format!("failed to build the test runtime: {error}"))?
        .block_on(future)
}

/// Run a future to completion on a fresh current-thread runtime.
///
/// Only the live-task-gauge assertion uses this, and the choice is forced by
/// measurement rather than taste. `RuntimeMetrics::num_alive_tasks` counts
/// *every* task on the runtime, including ones this workload never created, so
/// on a multi-thread runtime it is a whole-runtime gauge, not a gauge on the
/// pool. Sampling it there produced deltas from −12 to +10 with a long tail
/// above +2 in roughly a third of runs, at every pool size, on a workload whose
/// own task churn is a fixed small constant.
///
/// On a current-thread runtime the same workload is deterministic: 180 samples
/// across pool sizes 1, 4, and 16 recorded only −1 and 0, with no outlier. That
/// is the regime in which the reading answers the question the assertion asks,
/// so the assertion is made here and the multi-thread flavour is left to the
/// cases that need its parallelism.
///
/// Uncontended acquisition does not need parallelism to be exercised: it is a
/// straight-line acquire/release loop. Only the contended cases park on a
/// barrier, and they assert counts rather than the gauge.
///
/// Error handling matches [`block_on`].
pub(super) fn block_on_current_thread<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    run_on(tokio::runtime::Builder::new_current_thread(), future)
}

/// Run one contended burst, retrying the acquire path's known spurious failures.
///
/// Returns the first successful observation, or the last error if every attempt
/// failed. Consumes a future factory rather than a future because each attempt
/// needs a fresh measurement; the caller supplies it so this helper carries no
/// policy about what is being measured.
pub(super) fn contended_burst_with_retry<F, Fut>(mut attempt: F) -> Result<PoolMeasurement, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<PoolMeasurement, String>>,
{
    let mut last_error = None;
    for _ in 0..CONTENDED_BURST_ATTEMPTS {
        match block_on(attempt()) {
            Ok(measurement) => return Ok(measurement),
            Err(error) => last_error = Some(error),
        }
    }

    Err(format!(
        "contended burst failed in all {CONTENDED_BURST_ATTEMPTS} attempts; last: {}",
        last_error.as_deref().unwrap_or("no error recorded")
    ))
}

#[test]
fn default_workloads_are_usable_without_configuration() {
    let startup = StartupWorkload::default();
    assert_eq!(startup.routes, DEFAULT_ROUTES);
    assert_eq!(startup.middleware_layers, DEFAULT_MIDDLEWARE_LAYERS);
    assert_eq!(startup.connections, DEFAULT_CONNECTIONS);

    let pool = PoolWorkload::default();
    assert!(
        pool.pool_size > 0,
        "a default pool must have at least one slot"
    );

    assert_eq!(ActorWorkload::default().frames, DEFAULT_FRAME_COUNT);
}

#[rstest]
#[case::actor(ActorWorkload::default().label())]
#[case::push(PushWorkload::default().label())]
#[case::startup(StartupWorkload::default().label())]
#[case::pool(PoolWorkload::default().label())]
fn labels_are_identifier_safe(#[case] label: String) {
    assert!(!label.is_empty(), "labels must not be empty");
    assert!(
        !label.contains(char::is_whitespace),
        "labels become Criterion identifiers, so they must not contain whitespace: {label}"
    );
}
