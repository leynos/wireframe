//! Step definitions for published-document example scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::tested_examples::{TestResult, TestedExamplesWorld};

/// A document whose fenced example names no language, so the loader rejects it.
const UNLANGUAGED_DOCUMENT: &str = concat!(
    "<!-- tested-example: staged-001 -->\n",
    "```\n",
    "fn main() {}\n",
    "```\n",
);

#[given("the published user-facing examples are loaded")]
fn given_published_user_facing_examples_are_loaded(
    tested_examples_world: &mut TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.load()
}

#[then("every published example has a stable identifier and source")]
fn then_every_published_example_has_a_stable_identifier_and_source(
    tested_examples_world: &TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.verify_examples_are_addressable()
}

#[given("a document whose example fence carries no language")]
fn given_a_document_whose_example_fence_carries_no_language(
    tested_examples_world: &mut TestedExamplesWorld,
) {
    tested_examples_world.stage_document(UNLANGUAGED_DOCUMENT);
}

#[when("the shared loader validates the staged document")]
fn when_the_shared_loader_validates_the_staged_document(
    tested_examples_world: &mut TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.validate_staged_document()
}

#[then("validation fails and names the fence line")]
fn then_validation_fails_and_names_the_fence_line(
    tested_examples_world: &TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world
        .validation_failed_with("staged.md:2: fenced example has no language identifier")
}
