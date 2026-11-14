//! Step definitions for published-document example scenarios.

use rstest_bdd_macros::{given, then, when};

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

#[given("the shipped documentation is read from disk")]
fn given_the_shipped_documentation_is_read_from_disk(
    tested_examples_world: &mut TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.prepare_shipped_documents()
}

#[given("a shipped guide has lost one example marker")]
fn given_a_shipped_guide_has_lost_one_example_marker(
    tested_examples_world: &mut TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.prepare_shipped_documents_missing_marker()
}

#[when("the shared loader validates the shipped documentation")]
fn when_the_shared_loader_validates_the_shipped_documentation(
    tested_examples_world: &mut TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world.validate_prepared_documents()
}

#[then("validation succeeds")]
fn then_validation_succeeds(tested_examples_world: &TestedExamplesWorld) -> TestResult<()> {
    tested_examples_world.validation_succeeded()
}

#[then("validation fails and names the document and the missing marker")]
fn then_validation_fails_and_names_the_document_and_the_missing_marker(
    tested_examples_world: &TestedExamplesWorld,
) -> TestResult<()> {
    tested_examples_world
        .validation_failed_with_all(&["docs/users-guide.md:", "missing its marker"])
}
