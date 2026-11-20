//! Compact synthetic BDD layouts used by the wiring-check tests.
//!
//! The base layout conforms to every rule, and each [`Mutation`] applies one
//! defect so the mutation tests can assert that the checker reports the
//! corresponding violation rather than passing vacuously. This module holds
//! the sources, paths and snapshot builders; [`defects`] holds the builders
//! that break one rule each.

mod defects;

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

/// A feature whose `Rule` groups an unbound second scenario.
const RULE_UNBOUND_FEATURE: &str = r"
Feature: Example
  Scenario: Example runs
    Given an example

  Rule: Grouped behaviour
    Scenario: Grouped step
      Given an example
";

/// A feature whose `Rule` scenario is bound by the scenario module.
const RULE_BOUND_FEATURE: &str = RULE_UNBOUND_FEATURE;

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

/// A scenario module binding the feature's top-level and grouped titles.
const RULE_SCENARIO: &str = r#"
use rstest_bdd_macros::scenario;
use crate::fixtures::example::*;

#[scenario(
    path = "tests/features/example.feature",
    name = "Example runs"
)]
fn example_runs(example_world: ExampleWorld) { let _ = example_world; }

#[scenario(
    path = "tests/features/example.feature",
    name = "Grouped step"
)]
fn grouped_step(example_world: ExampleWorld) { let _ = example_world; }
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

/// A scenario module importing the family fixture by name, rename, and group.
const NAMED_IMPORTS: &str = r#"
use rstest_bdd_macros::scenario;
use crate::fixtures::example::example_world;
use crate::fixtures::example::ExampleWorld as World;
use crate::fixtures::example::{ExampleWorld, example_world as fixture};

#[scenario(
    path = "tests/features/example.feature",
    name = "Example runs"
)]
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

/// Main harness root, referenced by the mutation that removes a declaration.
const MAIN_ROOT_FILE: &str = "tests/bdd/mod.rs";
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
    /// The scenario and fixture modules are registered in different harnesses.
    SplitRegistrations,
    /// A binding points at another feature file.
    WrongFeaturePath,
    /// A scenario title has no binding.
    MissingBinding,
    /// A rule-grouped scenario title has no binding.
    RuleScenarioUnbound,
    /// A binding names a title the feature does not contain.
    UnmatchedBinding,
    /// A title is bound twice.
    DuplicateBinding,
    /// A binding uses an `index` selector.
    IndexSelector,
    /// A binding passes its `name` as a non-literal expression.
    NonLiteralName,
    /// A binding passes its `path` as a non-literal expression.
    NonLiteralPath,
    /// A binding omits its `name` argument.
    MissingName,
    /// The scenario module omits its fixture import.
    MissingImport,
    /// The scenario module imports only the fixture's `self`.
    SelfImport,
    /// The scenario module imports a different family's fixture.
    WrongFixtureImport,
    /// The scenario module imports only the fixture's `self`, renamed.
    SelfImportAliased,
    /// A feature family has both fixture file forms.
    AmbiguousFixture,
    /// A harness root omits a registry declaration.
    MissingRegistryDeclaration,
    /// A scenario `#[path]` resolves to a different existing file.
    ScenarioDeclarationWrongTarget,
    /// A fixture `#[path]` resolves to a different existing file.
    FixtureDeclarationWrongTarget,
    /// A fixture module is declared without `pub`.
    PrivateFixture,
    /// A scenario file has no matching feature.
    OrphanScenario,
    /// A registered scenario module has no matching feature.
    RegisteredOrphanScenario,
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
        .with_file(MAIN_ROOT_FILE, MAIN_ROOT)
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

/// Build the conforming layout where a `Rule` scenario is bound.
pub(crate) fn rule_scenario_snapshot() -> SnapshotBuilder {
    base_snapshot()
        .with_file(FEATURE_FILE, RULE_BOUND_FEATURE)
        .with_file(SCENARIO_FILE, RULE_SCENARIO)
}

/// Build the conforming layout with named fixture imports.
pub(crate) fn named_import_snapshot() -> SnapshotBuilder {
    base_snapshot().with_file(SCENARIO_FILE, NAMED_IMPORTS)
}

/// A single `use` form that can satisfy the family fixture import.
///
/// Each form takes a different branch of the `use`-tree walk, so the
/// acceptance cases built from this enum fail independently when one branch
/// stops recognizing a fixture import.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ImportForm {
    /// Import the fixture by name.
    Name,
    /// Import the fixture under a different name.
    Rename,
    /// Import the fixture as one item of a group.
    Group,
}

impl ImportForm {
    /// The `use` item that imports the family fixture in this form.
    fn import(self) -> &'static str {
        match self {
            Self::Name => "use crate::fixtures::example::example_world;",
            Self::Rename => "use crate::fixtures::example::ExampleWorld as World;",
            Self::Group => {
                "use crate::fixtures::example::{ExampleWorld, example_world as fixture};"
            }
        }
    }
}

/// Build the conforming layout importing the fixture in one `use` form only.
pub(crate) fn import_form_snapshot(form: ImportForm) -> SnapshotBuilder {
    base_snapshot().with_file(
        SCENARIO_FILE,
        EXAMPLE_SCENARIO.replace("use crate::fixtures::example::*;", form.import()),
    )
}

/// Apply one synthetic defect to the conforming layout.
pub(crate) fn mutated_snapshot(mutation: Mutation) -> SnapshotBuilder {
    defects::apply(base_snapshot(), mutation)
}
