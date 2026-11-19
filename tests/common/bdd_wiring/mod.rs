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
mod path_properties;
mod snapshot;
mod syntax;
mod synthetic;

pub(crate) use check::evaluate;
pub(crate) use harness::{feature_stems, harness_scenario_modules};
pub(crate) use snapshot::{RepoSnapshot, Violation};
pub(crate) use synthetic::{
    ImportForm,
    Mutation,
    alternate_heading_snapshot,
    base_snapshot,
    doc_string_snapshot,
    import_form_snapshot,
    mutated_snapshot,
    named_import_snapshot,
    nested_fixture_snapshot,
    rule_scenario_snapshot,
};
