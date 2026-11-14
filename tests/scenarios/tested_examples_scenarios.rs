//! Scenario tests for marked examples in published Markdown documents.
//!
//! Each scenario exercises the shared loader over the shipped documents read
//! from disk: one acceptance run over the untouched inventory, and one
//! rejection run after a real guide loses a marker.

use rstest_bdd_macros::scenario;

use crate::fixtures::tested_examples::{TestedExamplesWorld, tested_examples_world};

#[scenario(
    path = "tests/features/tested_examples.feature",
    name = "Published examples are addressable by stable identifiers"
)]
fn published_examples_are_addressable(tested_examples_world: TestedExamplesWorld) {
    let mut world = tested_examples_world;
    world
        .load()
        .expect("published user-facing examples should load");
    world
        .verify_examples_are_addressable()
        .expect("published examples should be addressable");
}

#[scenario(
    path = "tests/features/tested_examples.feature",
    name = "The shipped documentation passes the loader gate"
)]
fn shipped_documentation_passes_the_loader_gate(tested_examples_world: TestedExamplesWorld) {
    let mut world = tested_examples_world;
    world
        .prepare_shipped_documents()
        .expect("the shipped documentation should be readable from disk");
    world
        .validate_prepared_documents()
        .expect("validating the prepared documents should not itself fail");
    world
        .validation_succeeded()
        .expect("the shipped documentation should pass the loader gate");
}

#[scenario(
    path = "tests/features/tested_examples.feature",
    name = "A shipped document that loses a marker fails the loader gate"
)]
fn shipped_document_missing_a_marker_fails_the_loader_gate(
    tested_examples_world: TestedExamplesWorld,
) {
    let mut world = tested_examples_world;
    world
        .prepare_shipped_documents_missing_marker()
        .expect("the shipped documentation should be readable from disk");
    world
        .validate_prepared_documents()
        .expect("validating the prepared documents should not itself fail");
    world
        .validation_failed_with_all(&["docs/users-guide.md:", "missing its marker"])
        .expect("a guide without one marker should fail the loader gate");
}
