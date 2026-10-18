//! Criterion benchmarks for the runtime-ownership hot paths of Epic #635.
//!
//! Four scopes, one group each:
//!
//! | Group | Scope | Helper |
//! | --- | --- | --- |
//! | `actor/drain` | connection actor event processing | `measure_actor_drain` |
//! | `push/send` | push handle send path | `measure_push` |
//! | `startup/connection` | application connection startup | `measure_startup` |
//! | `pool/acquisition` | uncontended client pool acquisition | `measure_pool_acquisition` |
//!
//! Contended pool acquisition is deliberately absent from that table. It is
//! reported after `Criterion::final_summary` as bounded `tracing` events rather
//! than timed, because sustained contention intermittently stalls the pool
//! acquire path: Criterion would block during warm-up sampling a scenario that
//! never returns, taking the whole run with it. It is sampled repeatedly rather
//! than once — a single elapsed value for a scenario that also fails is
//! ambiguous — and the run is reported as a latency distribution beside the
//! pool's churn counters, with failed bursts counted rather than retried into
//! the samples. Those counters are what the issue asks for at this scope, and
//! they are recorded as observations, not as timings. See
//! `docs/runtime-ownership-baselines.md`.
//!
//! The workloads and the measurement logic live in
//! [`wireframe_testing::runtime_ownership_benchmarks`]; this binary only
//! sequences them for Criterion and pins the sample budget. Keeping the
//! scenarios as data in the support crate is what makes the baseline survive
//! the refactor: the same binary measures the old and the new ownership
//! topology, so a comparison needs no rewrite of the scenario.
//!
//! # Layout
//!
//! `scenarios` holds the workload parameters and the labels they produce;
//! `driver` registers them with Criterion through one shared path; `observe`
//! carries everything reported as a `tracing` event after the timed groups
//! have closed. Splitting them keeps each file inside the repository's length
//! limit without separating a scope from the constants it is built from.
//!
//! # Asynchrony
//!
//! Every scenario needs a Tokio runtime. Criterion's `async` feature is not
//! enabled in this workspace, so `Bencher::to_async` is unavailable. Each
//! iteration therefore drives the helper with `Runtime::block_on` inside
//! `iter_custom` and returns the helper's own elapsed duration, which is the
//! arrangement `benches/connection_startup.rs` already uses.
//!
//! A multi-thread runtime is required rather than merely convenient: the
//! contended pool scenarios park callers on a barrier and need real parallelism
//! to exhaust the pool.
//!
//! # Timings are not a gate
//!
//! Criterion stores its own baseline under `target/criterion` and reports a
//! change as a percentage, not a pass or fail. Nothing in continuous
//! integration runs these benchmarks; regression handling is by ratio and
//! manual review, as recorded in `docs/runtime-ownership-baselines.md`.

mod driver;
mod observe;
mod scenarios;

use criterion::Criterion;
use tokio::runtime::Runtime;

use crate::{
    driver::{actor_scope, pool_scope, push_scope, run_scope, startup_scope},
    observe::{init_observation_subscriber, observe_contended_acquisition, observe_push_waits},
    scenarios::expect_bench,
};

/// Entrypoint for the runtime-ownership baseline benchmarks.
fn main() {
    init_observation_subscriber();
    let runtime = expect_bench(Runtime::new(), "benchmark runtime failed");
    let mut criterion = Criterion::default().configure_from_args();

    // One registration path for all four scopes; see `BenchScope`.
    run_scope(&mut criterion, &runtime, actor_scope());
    run_scope(&mut criterion, &runtime, push_scope());
    run_scope(&mut criterion, &runtime, startup_scope());
    run_scope(&mut criterion, &runtime, pool_scope());

    criterion.final_summary();

    observe_push_waits(&runtime);
    observe_contended_acquisition(&runtime);
}
