//! Scenario tests for marked examples in published Markdown documents.

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
