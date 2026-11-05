//! Step definitions for published-document example scenarios.

use rstest_bdd_macros::{given, then};

use crate::fixtures::tested_examples::{TestResult, TestedExamplesWorld};

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
