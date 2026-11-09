//! Scenario tests for marked examples in published Markdown documents.

use rstest_bdd_macros::scenario;

use crate::fixtures::tested_examples::{TestedExamplesWorld, tested_examples_world};

/// A document whose fenced example names no language.
const UNLANGUAGED_DOCUMENT: &str = concat!(
    "<!-- tested-example: staged-001 -->\n",
    "```\n",
    "fn main() {}\n",
    "```\n",
);

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
    name = "A malformed example is rejected by the shared loader"
)]
fn malformed_examples_are_rejected(tested_examples_world: TestedExamplesWorld) {
    let mut world = tested_examples_world;
    world.stage_document(UNLANGUAGED_DOCUMENT);
    world
        .validate_staged_document()
        .expect("validating a staged document should not itself fail");
    world
        .validation_failed_with("staged.md:2: fenced example has no language identifier")
        .expect("a fence without a language should be rejected");
}
