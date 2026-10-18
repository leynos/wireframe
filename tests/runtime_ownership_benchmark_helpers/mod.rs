//! Unit tests for the runtime-ownership benchmark workload and measurement helpers.
//!
//! Each test discharges one obligation from the `ExecPlan`'s verification plan for
//! issue #639: V-O1 through V-O5 here, with V-O6's allocation controls living in
//! `benches/runtime_ownership_baselines_alloc/` because they need a
//! `#[global_allocator]` that a `tests/` target cannot install. Those
//! obligations are counting contracts over finite partitions, so parameterized
//! `rstest` cases carry them rather than property tests or a formal method; a
//! generator over these integer partitions would restate the same grid, and a
//! proof would restate the assertions.
//!
//! These checks drive real runtimes, allocate, and open loopback connections, so
//! they are unsuitable for loom.
//!
//! # Layout
//!
//! `harness` owns the runtime helpers, the iteration count every case shares,
//! and the workload-default and label contracts. The remaining modules group
//! cases by the obligation they discharge: `startup` (V-O2), `actor` (V-O3),
//! `push` (V-O4), and `pool` (V-O1 and V-O5). Splitting them keeps each file
//! below the workspace's 400-line module limit while leaving every assertion in
//! one place per obligation.

//! # V-O6 lives in the benchmark binary
//!
//! The counter assertions for the allocation obligation live in
//! `verify_allocation_harness` in `benches/runtime_ownership_baselines_alloc/probes.rs`,
//! not here and not in `wireframe_testing`'s unit tests. `CountingAllocator` is
//! a `#[global_allocator]`, and this file is a `tests/` target of the root
//! crate, which links the system allocator: a probe here reported zero for a
//! closure that allocated a 1024-byte buffer, proving the gate cannot open in
//! this binary. Installing an allocator per test target is not possible, so the
//! controls run where the allocator is installed. `wireframe_testing`'s own unit
//! tests can only check that the counting window opens and closes, since the
//! gate is observable without the allocator; the counts themselves are asserted
//! in the bench binary. The nested-`Arc` sensitivity experiment is recorded in
//! `docs/runtime-ownership-baselines.md` for the same reason.

#![cfg(not(loom))]

mod actor;
mod harness;
mod pool;
mod push;
mod startup;
