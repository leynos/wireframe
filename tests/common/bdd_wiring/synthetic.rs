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
}

/// Build the conforming synthetic layout shared by the mutation cases.
pub(crate) fn base_snapshot() -> SnapshotBuilder {
    SnapshotBuilder::new()
        .with_file("tests/bdd/mod.rs", MAIN_ROOT)
        .with_file("tests/scenarios/mod.rs", "mod example_scenarios;\n")
        .with_file("tests/fixtures/mod.rs", "pub mod example;\n")
        .with_file("tests/features/example.feature", EXAMPLE_FEATURE)
        .with_file("tests/scenarios/example_scenarios.rs", EXAMPLE_SCENARIO)
        .with_file("tests/fixtures/example.rs", EXAMPLE_FIXTURE)
        .with_file("tests/bdd_pool/mod.rs", POOL_ROOT)
        .with_file("tests/bdd_pool/scenarios.rs", "//! Scenario wiring.\n")
        .with_file("tests/bdd_pool/fixtures.rs", "//! Fixture wiring.\n")
}

/// Build the conforming layout with the `<stem>/mod.rs` fixture form.
pub(crate) fn nested_fixture_snapshot() -> SnapshotBuilder {
    base_snapshot()
        .without_file("tests/fixtures/example.rs")
        .with_file("tests/fixtures/example/mod.rs", EXAMPLE_FIXTURE)
}

/// Apply one synthetic defect to the conforming layout.
pub(crate) fn mutated_snapshot(mutation: Mutation) -> SnapshotBuilder {
    let builder = base_snapshot();
    match mutation {
        Mutation::MissingScenario => builder
            .without_file("tests/scenarios/example_scenarios.rs")
            .with_file("tests/scenarios/mod.rs", ""),
        Mutation::MissingFixture => builder
            .without_file("tests/fixtures/example.rs")
            .with_file("tests/fixtures/mod.rs", ""),
        Mutation::UnregisteredScenario => builder.with_file("tests/scenarios/mod.rs", ""),
        Mutation::RegisteredTwice => builder
            .with_file(
                "tests/bdd_pool/scenarios.rs",
                "#[path = \"../scenarios/example_scenarios.rs\"]\nmod example_scenarios;\n",
            )
            .with_file(
                "tests/bdd_pool/fixtures.rs",
                "#[path = \"../fixtures/example.rs\"]\npub mod example;\n",
            ),
        Mutation::WrongFeaturePath => builder.with_file(
            "tests/scenarios/example_scenarios.rs",
            EXAMPLE_SCENARIO.replace(
                "tests/features/example.feature",
                "tests/features/other.feature",
            ),
        ),
        Mutation::MissingBinding => {
            builder.with_file("tests/scenarios/example_scenarios.rs", IMPORTS_ONLY)
        }
        Mutation::UnmatchedBinding => builder.with_file(
            "tests/scenarios/example_scenarios.rs",
            EXAMPLE_SCENARIO.replace("Example runs", "Unknown scenario"),
        ),
        Mutation::DuplicateBinding => builder.with_file(
            "tests/scenarios/example_scenarios.rs",
            format!("{EXAMPLE_SCENARIO}\n{EXAMPLE_SCENARIO}"),
        ),
        Mutation::IndexSelector => builder.with_file(
            "tests/scenarios/example_scenarios.rs",
            EXAMPLE_SCENARIO.replace("name = \"Example runs\"", "index = 0"),
        ),
        Mutation::MissingImport => {
            builder.with_file("tests/scenarios/example_scenarios.rs", MISSING_IMPORT)
        }
        Mutation::OrphanScenario => builder.with_file(
            "tests/scenarios/orphan_scenarios.rs",
            "//! Orphan scenario module.\n",
        ),
        Mutation::PublicFixtureWithoutFeature => builder
            .with_file(
                "tests/fixtures/mod.rs",
                "pub mod example;\npub mod orphan;\n",
            )
            .with_file("tests/fixtures/orphan.rs", "//! Orphan fixture module.\n"),
        Mutation::UnresolvablePath => builder.with_file(
            "tests/scenarios/mod.rs",
            "mod example_scenarios;\n#[path = \"missing_scenarios.rs\"]\nmod missing_scenarios;\n",
        ),
        Mutation::DocCommentOnly => {
            builder.with_file("tests/scenarios/example_scenarios.rs", DOC_COMMENT_ONLY)
        }
    }
}
