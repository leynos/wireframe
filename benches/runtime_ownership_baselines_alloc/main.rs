//! Criterion benchmarks recording allocation baselines for the runtime-ownership
//! hot paths.
//!
//! This binary is separate from `runtime_ownership_baselines` because
//! `#[global_allocator]` is per-process: installing the counting allocator would
//! tax every timing sample in the sibling binary. Both binaries drive the same
//! workload helpers in [`wireframe_testing::runtime_ownership_benchmarks`].
//!
//! Each baseline's count is published as a `tracing` event beside a stable
//! benchmark ID. The count is deliberately not part of the ID: Criterion keys
//! its saved results, `--baseline` comparisons and change reports on the ID, and
//! a count that moves by tens between runs would make every run a new benchmark
//! with nothing to compare against.
//!
//! # What the number means
//!
//! The recorded value is a **process-global allocation delta** across the
//! measurement window. It includes the async runtime's own allocations, task
//! setup, closure captures, and anything else that ran in the window — not only
//! the code under test. Compare counts across runs of this same binary; do not
//! read them as a per-operation cost.
//!
//! # The sensitivity experiment
//!
//! Acceptance criterion 5 asks the harness to identify whether removing a
//! nested `Arc` changes the allocation count. That question has two possible
//! answers, and the harness must be able to tell them apart rather than assume
//! either:
//!
//! - cloning a shared `Arc` costs an atomic increment and **no** allocation, so removing such a
//!   clone changes nothing an allocation counter can see; while
//! - constructing a fresh `Arc` per item costs **one allocation per item**, so removing it moves
//!   the count by at least the item count.
//!
//! [`verify_allocation_harness`] runs both shapes before any measurement and
//! fails loudly unless the difference is at least one allocation per item, which
//! can only hold if the counter actually resolves the per-item allocation. The
//! recorded counts are transcribed into `docs/runtime-ownership-baselines.md`.
//!
//! # Contended pool acquisition
//!
//! FIFO contended pool acquisition is counted once per baseline pool size, after
//! `Criterion::final_summary`, and reported as a `tracing` event rather than
//! timed. Sustained contention intermittently fails or stalls the pool acquire
//! path — a pre-existing defect recorded in `docs/runtime-ownership-baselines.md`
//! and belonging to ADR 013 / issue #647. Criterion's timing pass runs far more
//! iterations than an allocation window, so it reaches that interleaving almost
//! every run; that is how this binary failed on its first execution. Each count
//! comes from one bracketed window.
//!
//! # Layout
//!
//! The counting allocator is installed here, at the crate root, because a
//! `#[global_allocator]` applies to the whole binary and must not sit in a
//! module that could be compiled without it. `scopes` holds the labelled
//! workload list, `pool_bracket` owns the pool scope's construction-excluding
//! window, `probes` proves the counter resolves what the baselines depend on,
//! and `reporting` writes the counts out.

mod pool_bracket;
mod probes;
mod reporting;
mod scopes;

use criterion::Criterion;
use tokio::runtime::Runtime;
use wireframe_testing::benchmark_alloc_support::CountingAllocator;

use crate::{
    pool_bracket::{
        verify_measured_phase_is_bracketed,
        verify_zero_iteration_bracket_excludes_construction,
    },
    probes::{expect_bench, verify_allocation_harness},
    reporting::{
        benchmark_allocations,
        benchmark_arc_sensitivity,
        init_observation_subscriber,
        observe_contended_pool_allocations,
    },
};

/// Process allocator for this benchmark binary.
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// Entrypoint for the runtime-ownership allocation baselines.
fn main() {
    init_observation_subscriber();

    // The harness check must precede the runtime and Criterion: both can start
    // threads whose allocations would otherwise land inside a measurement window.
    verify_allocation_harness();

    // The zero-iteration half must precede the runtime and Criterion too, and
    // builds and drops its own current-thread runtime in that window so no
    // worker thread of another runtime can allocate inside it.
    verify_zero_iteration_bracket_excludes_construction();

    let runtime = expect_bench(Runtime::new(), "allocation benchmark runtime failed");
    // The measured half needs the benchmark runtime to build a real pool, so it
    // follows construction rather than preceding it like the guard above. Both
    // run before Criterion, whose own threads would otherwise allocate inside a
    // window.
    verify_measured_phase_is_bracketed(&runtime);

    let mut criterion = Criterion::default().configure_from_args();

    benchmark_allocations(&mut criterion, &runtime);
    benchmark_arc_sensitivity(&mut criterion);

    criterion.final_summary();

    observe_contended_pool_allocations(&runtime);
}
