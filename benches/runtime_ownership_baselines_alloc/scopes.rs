//! The labelled allocation scopes: what is counted, and how each one brackets it.
//!
//! Every scope is data — a label, a closure that counts, and a closure that
//! replays — so one list feeds both the allocation and timing groups. The pool
//! scope is the exception in shape: its bracket is narrower than its replay, and
//! `pool_bracket` owns both halves of that.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::runtime::Runtime;
use wireframe_testing::{
    benchmark_alloc_support::count_allocations,
    runtime_ownership_benchmarks::{
        AcquisitionMode,
        ActorWorkload,
        CancellationMode,
        POOL_SIZES,
        PoolWorkload,
        PushScenario,
        PushWorkload,
        StartupPath,
        StartupWorkload,
        measure_actor_drain,
        measure_push,
        measure_startup,
    },
};

use crate::pool_bracket::{count_pool_acquisition_window, run_pool_acquisition};

/// Workload iterations performed inside each allocation window.
///
/// Small on purpose: the window's count grows with the iteration count, and a
/// large one would make every label a four-digit number that is harder to
/// compare at a glance.
pub(crate) const ALLOCATION_ITERATIONS: u64 = 4;

/// Frames per iteration in the actor allocation workload.
const ACTOR_FRAMES: usize = 256;

/// Per-queue capacity in the actor allocation workload.
///
/// At least half [`ACTOR_FRAMES`], so the workload fits its queues and does not
/// block on back-pressure.
const ACTOR_QUEUE_CAPACITY: usize = 128;

/// Sends per iteration in the push allocation workloads.
const PUSH_SENDS: usize = 256;

/// Queue capacity for the drained push allocation workload.
///
/// Deliberately under [`PUSH_SENDS`], so the producer can outrun its consumer.
/// Whether it did is neither asserted nor reported here: this binary records
/// allocation counts only, and the wait counters appear in the timing binary.
const PUSH_QUEUE_CAPACITY: usize = 64;

/// Queue capacity for the overflow push allocation workload.
const PUSH_OVERFLOW_CAPACITY: usize = 8;

/// A labelled workload, the closure that counts it, and the closure that replays
/// it.
///
/// Counting and replaying are separate closures rather than one, because most
/// scopes count by wrapping their workload in a counting window while the pool
/// scope must count something narrower than it replays — see
/// [`count_pool_acquisition_window`]. Splitting them is what lets a scope whose
/// bracket is not "wrap the whole workload" still take part in the same list.
///
/// Both closures are boxed so scopes with different captured workloads share one
/// list type, which keeps the allocation and timing groups iterating one set.
pub(crate) struct AllocationScope<'a> {
    /// Label carried into the benchmark id, naming the workload's parameters.
    pub(crate) label: String,
    /// Take this scope's allocation count, in a fresh counting window.
    ///
    /// Called once per scope, before the timing passes. It is a closure rather
    /// than a plain number so each scope owns its bracketing, and it returns a
    /// `Result` so a scope that cannot be counted stops the run rather than
    /// recording a fabricated zero.
    pub(crate) count: Box<dyn Fn() -> Result<usize, String> + 'a>,
    /// Replay the same workload without counting, for the timing passes.
    ///
    /// Returns the duration the scope itself wants timed, not the caller's wall
    /// clock around the closure. The pool scope needs that difference: its
    /// replay builds and closes a pool either side of the acquisition rounds,
    /// and only the rounds belong in the figure the label describes.
    pub(crate) replay: Box<dyn Fn() -> Result<Duration, String> + 'a>,
}

impl<'a> AllocationScope<'a> {
    /// A scope counted by wrapping its whole workload in a counting window.
    ///
    /// The common case: the workload has no region whose cost needs excluding, so
    /// counting it and replaying it are the same call. One operation is shared by
    /// both closures rather than written twice, so the count and the timing
    /// cannot drift apart onto different workloads — the failure that writing
    /// the call out twice invites.
    fn windowed(label: String, operation: impl Fn() -> Result<(), String> + 'a) -> Self {
        // An `Arc` rather than a clone bound: the closure's captures are not
        // necessarily `Clone`, and sharing one instance is what guarantees both
        // closures run the same workload.
        let shared = Arc::new(operation);
        let counted = Arc::clone(&shared);
        Self {
            label,
            count: Box::new(move || count_allocations(|| (counted)())),
            // The whole call is the scope here, so timing around it is the same
            // figure the pool scope computes from the inside.
            replay: Box::new(move || {
                let started = Instant::now();
                (shared)()?;
                Ok(started.elapsed())
            }),
        }
    }

    /// Box this scope's replay closure for the timing pass.
    pub(crate) fn into_replay(self) -> (String, Box<dyn Fn() -> Result<Duration, String> + 'a>) {
        (self.label, self.replay)
    }
}

/// Drive a measurement future to completion, discarding its counters.
///
/// This binary records only the allocation delta of the counting window, so the
/// measurement's own counters are dropped: a scope closure's only job is to
/// perform the work the counting window observes. Adapting every scope through
/// one helper is what keeps [`allocation_scopes`] short enough to read and keeps
/// the scopes visibly identical in shape.
fn run_measurement<T, E: std::fmt::Display>(
    runtime: &Runtime,
    measurement: impl std::future::Future<Output = Result<T, E>>,
) -> Result<(), String> {
    runtime
        .block_on(measurement)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The workload parameters this binary records allocations for, by scope.
///
/// Split from [`allocation_scopes`] so its closures stay below the workspace's
/// function-size limit. Keeping the parameters in one block also makes them
/// easy to compare at a glance, which is their point: each differs from the
/// timing binary's defaults deliberately, to keep allocation windows short.
pub(crate) struct AllocationWorkloads {
    /// Actor drain, with cancellation disabled so the window covers the loop.
    actor: ActorWorkload,
    /// Awaited send into a draining queue that may not keep up.
    push_drained: PushWorkload,
    /// Non-blocking send against a deliberately small, full queue.
    push_overflow: PushWorkload,
    /// The shared prepared root, which prepares once for all connections.
    startup_prepared: StartupWorkload,
    /// The per-connection factory path, which prepares once per connection.
    startup_per_connection: StartupWorkload,
    /// Uncontended acquire/release at the named baseline pool sizes.
    pool_workloads: [PoolWorkload; POOL_SIZES.len()],
}

impl AllocationWorkloads {
    /// Build the scopes at their allocation-window parameters.
    pub(crate) fn new() -> Self {
        Self {
            actor: ActorWorkload {
                frames: ACTOR_FRAMES,
                queue_capacity: ACTOR_QUEUE_CAPACITY,
                cancellation: CancellationMode::Disabled,
            },
            push_drained: PushWorkload {
                // Drained, not uncontended: at this send count the queue cannot
                // hold the run, so the previous `UncontendedHigh` label claimed a
                // no-back-pressure property the workload could not support.
                scenario: PushScenario::DrainedHigh,
                queue_capacity: PUSH_QUEUE_CAPACITY,
                sends: PUSH_SENDS,
            },
            push_overflow: PushWorkload {
                scenario: PushScenario::OverflowDrop,
                queue_capacity: PUSH_OVERFLOW_CAPACITY,
                sends: PUSH_SENDS,
            },
            startup_prepared: StartupWorkload {
                path: StartupPath::PreparedRoot,
                ..StartupWorkload::default()
            },
            startup_per_connection: StartupWorkload {
                path: StartupPath::PerConnectionFactory,
                ..StartupWorkload::default()
            },
            pool_workloads: POOL_SIZES.map(|pool_size| PoolWorkload {
                pool_size,
                mode: AcquisitionMode::Uncontended,
                ..PoolWorkload::default()
            }),
        }
    }
}

/// Build the runtime-ownership workloads this binary records allocations for.
///
/// One list feeds both the allocation and timing groups, so adding a scope can
/// not silently skip the timing side. The contended pool scope is excluded
/// because it cannot be timed at all; see `reporting::observe_contended_pool_allocations`.
pub(crate) fn allocation_scopes(runtime: &Runtime) -> Vec<AllocationScope<'_>> {
    let workloads = AllocationWorkloads::new();
    let iterations = ALLOCATION_ITERATIONS;

    let mut scopes = vec![
        AllocationScope::windowed(workloads.actor.label(), move || {
            run_measurement(runtime, measure_actor_drain(workloads.actor, iterations))
        }),
        AllocationScope::windowed(workloads.push_drained.label(), move || {
            run_measurement(runtime, measure_push(workloads.push_drained, iterations))
        }),
        AllocationScope::windowed(workloads.push_overflow.label(), move || {
            run_measurement(runtime, measure_push(workloads.push_overflow, iterations))
        }),
        AllocationScope::windowed(workloads.startup_prepared.label(), move || {
            run_measurement(
                runtime,
                measure_startup(workloads.startup_prepared, iterations),
            )
        }),
        AllocationScope::windowed(workloads.startup_per_connection.label(), move || {
            run_measurement(
                runtime,
                measure_startup(workloads.startup_per_connection, iterations),
            )
        }),
    ];
    scopes.extend(
        workloads
            .pool_workloads
            .into_iter()
            .map(|workload| AllocationScope {
                label: workload.label(),
                count: Box::new(move || {
                    count_pool_acquisition_window(runtime, workload, iterations)
                }),
                replay: Box::new(move || run_pool_acquisition(runtime, workload, iterations)),
            }),
    );
    scopes
}
