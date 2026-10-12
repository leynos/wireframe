//! Measurement helpers for the runtime-ownership hot paths.
//!
//! Epic #635 proposes to change the ownership topology of four hot paths. Before
//! that change can be judged, each path needs a reproducible baseline, and a
//! baseline is only useful if the same scenario can be pointed at the old and
//! the new implementation without rewriting it. Each sibling module therefore
//! keeps its scenarios as data — workload enums and structs — with a single
//! `measure_*` entry point that returns a
//! [`Result`] rather than panicking, so a benchmark binary, a test, or a
//! one-off experiment can all drive the same code.
//!
//! | Scope | Module | Entry point |
//! | --- | --- | --- |
//! | Connection actor event processing | [`connection_actor_benchmark_support`] | [`measure_actor_drain`] |
//! | Push handle send path | [`push_benchmark_support`] | [`measure_push`] |
//! | Application connection startup | [`connection_startup_benchmark_support`] | [`measure_startup`] |
//! | Client pool acquisition | [`client_pool_benchmark_support`] | [`measure_pool_acquisition`] |
//!
//! This module mirrors [`codec_benchmarks`](crate::codec_benchmarks), which is
//! the repository's existing benchmark-support arrangement. No second benchmark
//! framework is introduced: the helpers report raw [`Duration`] and counter
//! values, and Criterion does the statistics.
//!
//! # Recorded parameters
//!
//! Every workload carries the parameters a baseline must state — frame and send
//! counts, queue capacities, pool sizes, fairness policy, route and middleware
//! counts, cancellation regime — and
//! [`RuntimeOwnershipWorkload::label`] renders them for benchmark identifiers.
//! Measurements that are only meaningful with an exact count (factory calls,
//! middleware transformations, scheduler task spawns) report the count rather
//! than an inferred proxy.
//!
//! # Feature gate
//!
//! Gated behind the `runtime-benchmarks` feature, which enables the `test-support`
//! and `pool` features of `wireframe`. The pool scenarios need `pool` for the
//! client pool itself; the startup scenarios need `test-support` for the
//! connection-startup fixtures. Gating here keeps
//! `cargo check -p wireframe_testing` working with default features.
//!
//! [`Duration`]: std::time::Duration

pub mod client_pool_benchmark_support;
pub mod connection_actor_benchmark_support;
pub mod connection_startup_benchmark_support;
pub mod push_benchmark_support;

pub use client_pool_benchmark_support::{
    AcquisitionMode,
    CONTENDED_BURST_TIMEOUT,
    CONTENDED_LATENCY_SAMPLES,
    CONTENDED_ROUNDS_PER_CONTENDER,
    ContendedLatency,
    DEFAULT_ACQUISITIONS,
    DEFAULT_CONTENDERS,
    POOL_SIZES,
    PoolHarness,
    PoolMeasurement,
    PoolWorkload,
    measure_contended_latency,
    measure_pool_acquisition,
};
pub use connection_actor_benchmark_support::{
    ActorMeasurement,
    ActorWorkload,
    CancellationMode,
    DEFAULT_FRAME_COUNT,
    DEFAULT_QUEUE_CAPACITY as DEFAULT_ACTOR_QUEUE_CAPACITY,
    measure_actor_drain,
};
pub use connection_startup_benchmark_support::{
    DEFAULT_CONNECTIONS,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_ROUTES,
    StartupMeasurement,
    StartupPath,
    StartupWorkload,
    measure_startup,
};
pub use push_benchmark_support::{
    DEFAULT_QUEUE_CAPACITY as DEFAULT_PUSH_QUEUE_CAPACITY,
    DEFAULT_SENDS,
    PushMeasurement,
    PushScenario,
    PushWorkload,
    measure_push,
};

/// A workload that can describe itself in a benchmark identifier.
///
/// Labels embed the recorded parameters so a baseline table can be read without
/// consulting the source, and so a scenario whose parameters changed cannot be
/// silently compared against an older baseline.
pub trait RuntimeOwnershipWorkload {
    /// Render this workload's parameters as a stable benchmark label.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{
    ///     RuntimeOwnershipWorkload,
    ///     StartupWorkload,
    /// };
    ///
    /// let workload = StartupWorkload::default();
    /// assert!(workload.label().starts_with("prepared_root_routes_"));
    /// ```
    #[must_use]
    fn label(&self) -> String;
}

impl RuntimeOwnershipWorkload for connection_actor_benchmark_support::ActorWorkload {
    fn label(&self) -> String { (*self).label() }
}

impl RuntimeOwnershipWorkload for push_benchmark_support::PushWorkload {
    fn label(&self) -> String { (*self).label() }
}

impl RuntimeOwnershipWorkload for connection_startup_benchmark_support::StartupWorkload {
    fn label(&self) -> String { (*self).label() }
}

impl RuntimeOwnershipWorkload for client_pool_benchmark_support::PoolWorkload {
    fn label(&self) -> String { (*self).label() }
}

#[cfg(test)]
mod tests {
    //! Coverage for the label contract the baseline table depends on.

    use super::{
        AcquisitionMode,
        ActorWorkload,
        CancellationMode,
        PoolWorkload,
        PushScenario,
        PushWorkload,
        RuntimeOwnershipWorkload,
        StartupPath,
        StartupWorkload,
    };

    /// Collect labels through the trait so the trait method is what is tested.
    fn labels_of(workloads: &[&dyn RuntimeOwnershipWorkload]) -> Vec<String> {
        workloads.iter().map(|workload| workload.label()).collect()
    }

    #[test]
    fn every_workload_renders_a_non_empty_label() {
        let labels = labels_of(&[
            &ActorWorkload {
                frames: 256,
                queue_capacity: 512,
                cancellation: CancellationMode::Disabled,
            },
            &PushWorkload {
                scenario: PushScenario::OverflowDrop,
                queue_capacity: 4,
                sends: 8,
            },
            &StartupWorkload {
                path: StartupPath::PreparedRoot,
                routes: 2,
                middleware_layers: 3,
                connections: 4,
            },
            &PoolWorkload {
                pool_size: 16,
                mode: AcquisitionMode::ContendedRoundRobin,
                acquisitions: 32,
                contenders: 64,
            },
        ]);

        for label in labels {
            assert!(!label.is_empty(), "labels must not be empty");
            assert!(
                !label.contains(char::is_whitespace),
                "labels must be identifier-safe: {label}"
            );
        }
    }

    #[test]
    fn labels_distinguish_workloads_that_differ_in_one_parameter() {
        let base = StartupWorkload {
            path: StartupPath::PreparedRoot,
            routes: 2,
            middleware_layers: 2,
            connections: 4,
        };
        let more_routes = StartupWorkload { routes: 3, ..base };
        let labels = labels_of(&[&base, &more_routes]);
        assert_ne!(
            labels.first(),
            labels.get(1),
            "a changed parameter must change the label"
        );
    }
}
