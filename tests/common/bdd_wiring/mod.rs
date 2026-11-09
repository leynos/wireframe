//! Support for the BDD naming and registration wiring check.
//!
//! The check separates collecting repository files from evaluating the
//! documented naming contract. [`RepoSnapshot`] carries repository-relative
//! paths and their contents, [`evaluate`] applies the pure rules, and
//! [`SnapshotBuilder`](snapshot::SnapshotBuilder) builds compact synthetic
//! layouts for tests.

mod bindings;
mod check;
mod features;
mod harness;
mod orphans;
mod snapshot;
mod syntax;
mod synthetic;

pub(crate) use check::evaluate;
pub(crate) use harness::{feature_stems, harness_scenario_modules};
pub(crate) use snapshot::{RepoSnapshot, Violation};
pub(crate) use synthetic::{Mutation, base_snapshot, mutated_snapshot, nested_fixture_snapshot};
