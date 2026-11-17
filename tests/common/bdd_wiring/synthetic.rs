//! Compact synthetic BDD layouts used by the wiring-check tests.
//!
//! The base layout conforms to every rule. Each [`Mutation`] applies one
//! defect so the mutation tests can assert that the checker reports the
//! corresponding violation rather than passing vacuously.

use super::snapshot::SnapshotBuilder;

/// Harness root that declares the main scenario and fixture registries.
const MAIN_ROOT: &str = r#"
#[path = "../scenarios/mod.rs"]
mod scenarios;

#[path = "../fixtures/mod.rs"]
mod fixtures;
"#;

/// Pool harness root; both registries are siblings of the root file.
const POOL_ROOT: &str = "mod fixtures;\nmod scenarios;\n";

/// A feature with a single scenario title.
const EXAMPLE_FEATURE: &str = r"
Feature: Example
  Scenario: Example runs
    Given an example
";

/// A feature using the alternative `Example:` and `Scenario Template:` headings.
const ALTERNATE_FEATURE: &str = r"
Feature: Example
  Example: Example runs
    Given an example

  Scenario Template: Example is templated
    Given an example

    Examples:
      | value |
      | one   |
";

/// A feature whose `Example:` heading has no binding.
const EXAMPLE_HEADING_FEATURE: &str = r"
Feature: Example
  Scenario: Example runs
    Given an example

  Example: Unbound example
    Given an example
";

/// A feature whose `Scenario Template:` heading has no binding.
const TEMPLATE_HEADING_FEATURE: &str = r"
Feature: Example
  Scenario: Example runs
    Given an example

  Scenario Template: Unbound template
    Given an example

    Examples:
      | value |
      | one   |
";

/// A feature whose doc string contains text that looks like a heading.
const DOC_STRING_FEATURE: &str = r#"
Feature: Example
  Scenario: Example runs
    Given an example
      """
      Scenario: Not a real scenario
      """
"#;

/// A feature with an unterminated doc string.
const MALFORMED_FEATURE: &str = r#"
Feature: Example
  Scenario: Example runs
    Given an example
      """
"#;

/// A scenario module that conforms to every wiring rule.
const EXAMPLE_SCENARIO: &str = r#"
use rstest_bdd_macros::scenario;
use crate::fixtures::example::*;

#[scenario(
    path = "tests/features/example.feature",
    name = "Example runs"
)]
fn example_runs(example_world: ExampleWorld) { let _ = example_world; }
"#;

/// A scenario module binding both alternative headings.
const ALTERNATE_SCENARIO: &str = r#"
use rstest_bdd_macros::scenario;
use crate::fixtures::example::*;

#[scenario(
    path = "tests/features/example.feature",
    name = "Example runs"
)]
fn example_runs(example_world: ExampleWorld) { let _ = example_world; }

#[scenario(
    path = "tests/features/example.feature",
    name = "Example is templated"
)]
fn example_is_templated(example_world: ExampleWorld) { let _ = example_world; }
"#;

/// A scenario module with imports but no bindings.
const IMPORTS_ONLY: &str = r"
use rstest_bdd_macros::scenario;
use crate::fixtures::example::*;
";

/// A scenario module that binds the title only inside a doc comment.
const DOC_COMMENT_ONLY: &str = r#"
use crate::fixtures::example::*;

/// #[scenario(path = "tests/features/example.feature", name = "Example runs")]
fn example_runs(example_world: ExampleWorld) { let _ = example_world; }
"#;

/// A scenario module without the fixture import.
const MISSING_IMPORT: &str = r#"
use rstest_bdd_macros::scenario;

#[scenario(
    path = "tests/features/example.feature",
    name = "Example runs"
)]
fn example_runs(example_world: ExampleWorld) { let _ = example_world; }
"#;

/// Example world fixture.
const EXAMPLE_FIXTURE: &str = "//! Example world.\npub struct ExampleWorld;\n";

/// Paths shared by the synthetic layouts.
const FEATURE_FILE: &str = "tests/features/example.feature";
const SCENARIO_FILE: &str = "tests/scenarios/example_scenarios.rs";
const SCENARIO_REGISTRY: &str = "tests/scenarios/mod.rs";
const FIXTURE_FILE: &str = "tests/fixtures/example.rs";
const FIXTURE_REGISTRY: &str = "tests/fixtures/mod.rs";
const POOL_SCENARIOS: &str = "tests/bdd_pool/scenarios.rs";
const POOL_FIXTURES: &str = "tests/bdd_pool/fixtures.rs";

/// A single synthetic defect applied to the conforming layout.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Mutation {
    /// The family's scenario module is absent.
    MissingScenario,
    /// The family's fixture module is absent.
    MissingFixture,
    /// The scenario module exists but is not declared in a registry.
    UnregisteredScenario,
    /// Both harnesses register the family.
    RegisteredTwice,
    /// A binding points at another feature file.
    WrongFeaturePath,
    /// A scenario title has no binding.
    MissingBinding,
    /// A binding names a title the feature does not contain.
    UnmatchedBinding,
    /// A title is bound twice.
    DuplicateBinding,
    /// A binding uses an `index` selector.
    IndexSelector,
    /// The scenario module omits its fixture import.
    MissingImport,
    /// A scenario file has no matching feature.
    OrphanScenario,
    /// A public fixture module has no matching feature.
    PublicFixtureWithoutFeature,
    /// A `#[path]` declaration resolves to a missing file.
    UnresolvablePath,
    /// The `#[scenario]` attribute appears only in a doc comment.
    DocCommentOnly,
    /// An `Example:` heading has no binding.
    ExampleHeadingUnbound,
    /// A `Scenario Template:` heading has no binding.
    ScenarioTemplateUnbound,
    /// The feature file is not valid Gherkin.
    MalformedFeature,
}

/// Build the conforming synthetic layout shared by the mutation cases.
pub(crate) fn base_snapshot() -> SnapshotBuilder {
    SnapshotBuilder::new()
        .with_file("tests/bdd/mod.rs", MAIN_ROOT)
        .with_file(SCENARIO_REGISTRY, "mod example_scenarios;\n")
        .with_file(FIXTURE_REGISTRY, "pub mod example;\n")
        .with_file(FEATURE_FILE, EXAMPLE_FEATURE)
        .with_file(SCENARIO_FILE, EXAMPLE_SCENARIO)
        .with_file(FIXTURE_FILE, EXAMPLE_FIXTURE)
        .with_file("tests/bdd_pool/mod.rs", POOL_ROOT)
        .with_file(POOL_SCENARIOS, "//! Scenario wiring.\n")
        .with_file(POOL_FIXTURES, "//! Fixture wiring.\n")
}

/// Build the conforming layout with the `<stem>/mod.rs` fixture form.
pub(crate) fn nested_fixture_snapshot() -> SnapshotBuilder {
    base_snapshot()
        .without_file(FIXTURE_FILE)
        .with_file("tests/fixtures/example/mod.rs", EXAMPLE_FIXTURE)
}

/// Build the conforming layout with the alternative Gherkin headings.
pub(crate) fn alternate_heading_snapshot() -> SnapshotBuilder {
    base_snapshot()
        .with_file(FEATURE_FILE, ALTERNATE_FEATURE)
        .with_file(SCENARIO_FILE, ALTERNATE_SCENARIO)
}

/// Build the conforming layout with heading-like text inside a doc string.
pub(crate) fn doc_string_snapshot() -> SnapshotBuilder {
    base_snapshot().with_file(FEATURE_FILE, DOC_STRING_FEATURE)
}

/// Apply one synthetic defect to the conforming layout.
pub(crate) fn mutated_snapshot(mutation: Mutation) -> SnapshotBuilder {
    let builder = base_snapshot();
    match mutation {
        Mutation::MissingScenario => missing_scenario(builder),
        Mutation::MissingFixture => missing_fixture(builder),
        Mutation::UnregisteredScenario => builder.with_file(SCENARIO_REGISTRY, ""),
        Mutation::RegisteredTwice => registered_twice(builder),
        Mutation::WrongFeaturePath => wrong_feature_path(builder),
        Mutation::MissingBinding => builder.with_file(SCENARIO_FILE, IMPORTS_ONLY),
        Mutation::UnmatchedBinding => unmatched_binding(builder),
        Mutation::DuplicateBinding => duplicate_binding(builder),
        Mutation::IndexSelector => index_selector(builder),
        Mutation::MissingImport => builder.with_file(SCENARIO_FILE, MISSING_IMPORT),
        Mutation::OrphanScenario => orphan_scenario(builder),
        Mutation::PublicFixtureWithoutFeature => public_fixture_without_feature(builder),
        Mutation::UnresolvablePath => unresolvable_path(builder),
        Mutation::DocCommentOnly => builder.with_file(SCENARIO_FILE, DOC_COMMENT_ONLY),
        Mutation::ExampleHeadingUnbound => builder.with_file(FEATURE_FILE, EXAMPLE_HEADING_FEATURE),
        Mutation::ScenarioTemplateUnbound => {
            builder.with_file(FEATURE_FILE, TEMPLATE_HEADING_FEATURE)
        }
        Mutation::MalformedFeature => builder.with_file(FEATURE_FILE, MALFORMED_FEATURE),
    }
}

/// Remove the scenario module and its registration.
fn missing_scenario(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .without_file(SCENARIO_FILE)
        .with_file(SCENARIO_REGISTRY, "")
}

/// Remove the fixture module and its registration.
fn missing_fixture(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .without_file(FIXTURE_FILE)
        .with_file(FIXTURE_REGISTRY, "")
}

/// Register the family in both harnesses.
fn registered_twice(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .with_file(
            POOL_SCENARIOS,
            "#[path = \"../scenarios/example_scenarios.rs\"]\nmod example_scenarios;\n",
        )
        .with_file(
            POOL_FIXTURES,
            "#[path = \"../fixtures/example.rs\"]\npub mod example;\n",
        )
}

/// Point the binding at a feature file that does not exist.
fn wrong_feature_path(builder: SnapshotBuilder) -> SnapshotBuilder {
    let source = EXAMPLE_SCENARIO.replace(FEATURE_FILE, "tests/features/other.feature");
    builder.with_file(SCENARIO_FILE, source)
}

/// Bind a name the feature does not contain.
fn unmatched_binding(builder: SnapshotBuilder) -> SnapshotBuilder {
    let source = EXAMPLE_SCENARIO.replace("Example runs", "Unknown scenario");
    builder.with_file(SCENARIO_FILE, source)
}

/// Bind the same title twice.
fn duplicate_binding(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(
        SCENARIO_FILE,
        format!("{EXAMPLE_SCENARIO}\n{EXAMPLE_SCENARIO}"),
    )
}

/// Select a scenario by index instead of by name.
fn index_selector(builder: SnapshotBuilder) -> SnapshotBuilder {
    let source = EXAMPLE_SCENARIO.replace("name = \"Example runs\"", "index = 0");
    builder.with_file(SCENARIO_FILE, source)
}

/// Add a scenario module that has no matching feature.
fn orphan_scenario(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(
        "tests/scenarios/orphan_scenarios.rs",
        "//! Orphan scenario module.\n",
    )
}

/// Register a public fixture module that has no matching feature.
fn public_fixture_without_feature(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .with_file(FIXTURE_REGISTRY, "pub mod example;\npub mod orphan;\n")
        .with_file("tests/fixtures/orphan.rs", "//! Orphan fixture module.\n")
}

/// Declare a module whose `#[path]` target does not exist.
fn unresolvable_path(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(
        SCENARIO_REGISTRY,
        "mod example_scenarios;\n#[path = \"missing_scenarios.rs\"]\nmod missing_scenarios;\n",
    )
}
