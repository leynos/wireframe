//! One-defect variants of the conforming synthetic layout.
//!
//! `apply` starts from the conforming layout and breaks exactly one documented
//! rule, so each mutation test proves a specific rule fires instead of passing
//! vacuously. Helpers stay separate from the layout sources in the parent
//! module so both remain easy to scan.

use super::{
    super::snapshot::SnapshotBuilder,
    DOC_COMMENT_ONLY,
    EXAMPLE_FIXTURE,
    EXAMPLE_HEADING_FEATURE,
    EXAMPLE_SCENARIO,
    FEATURE_FILE,
    FIXTURE_FILE,
    FIXTURE_REGISTRY,
    IMPORTS_ONLY,
    MAIN_ROOT_FILE,
    MALFORMED_FEATURE,
    MISSING_IMPORT,
    Mutation,
    POOL_FIXTURES,
    POOL_SCENARIOS,
    RULE_UNBOUND_FEATURE,
    SCENARIO_FILE,
    SCENARIO_REGISTRY,
    TEMPLATE_HEADING_FEATURE,
};

/// Apply one synthetic defect to the conforming layout.
pub(super) fn apply(builder: SnapshotBuilder, mutation: Mutation) -> SnapshotBuilder {
    match mutation {
        Mutation::MissingScenario => missing_scenario(builder),
        Mutation::MissingFixture => missing_fixture(builder),
        Mutation::UnregisteredScenario => builder.with_file(SCENARIO_REGISTRY, ""),
        Mutation::RegisteredTwice => registered_twice(builder),
        Mutation::SplitRegistrations => split_registrations(builder),
        Mutation::WrongFeaturePath => wrong_feature_path(builder),
        Mutation::MissingBinding => builder.with_file(SCENARIO_FILE, IMPORTS_ONLY),
        Mutation::RuleScenarioUnbound => builder.with_file(FEATURE_FILE, RULE_UNBOUND_FEATURE),
        Mutation::UnmatchedBinding => replace_scenario(builder, "Example runs", "Unknown scenario"),
        Mutation::DuplicateBinding => duplicate_binding(builder),
        Mutation::IndexSelector => {
            replace_scenario(builder, "name = \"Example runs\"", "index = 0")
        }
        Mutation::NonLiteralName => {
            replace_scenario(builder, "name = \"Example runs\"", "name = UNSET_TITLE")
        }
        Mutation::NonLiteralPath => replace_scenario(
            builder,
            "path = \"tests/features/example.feature\"",
            "path = UNSET_FEATURE",
        ),
        Mutation::MissingName => replace_scenario(builder, ",\n    name = \"Example runs\"", ""),
        Mutation::MissingImport => builder.with_file(SCENARIO_FILE, MISSING_IMPORT),
        Mutation::SelfImport => replace_scenario(
            builder,
            "use crate::fixtures::example::*;",
            "use crate::fixtures::example::{self};",
        ),
        Mutation::AmbiguousFixture => ambiguous_fixture(builder),
        Mutation::MissingRegistryDeclaration => builder.with_file(
            MAIN_ROOT_FILE,
            "#[path = \"../scenarios/mod.rs\"]\nmod scenarios;\n",
        ),
        Mutation::ScenarioDeclarationWrongTarget => scenario_declaration_wrong_target(builder),
        Mutation::FixtureDeclarationWrongTarget => fixture_declaration_wrong_target(builder),
        Mutation::PrivateFixture => builder.with_file(FIXTURE_REGISTRY, "mod example;\n"),
        Mutation::OrphanScenario => orphan_scenario(builder),
        Mutation::PublicFixtureWithoutFeature => public_fixture_without_feature(builder),
        Mutation::UnresolvablePath => builder.with_file(
            SCENARIO_REGISTRY,
            "mod example_scenarios;\n#[path = \"missing_scenarios.rs\"]\nmod missing_scenarios;\n",
        ),
        Mutation::DocCommentOnly => builder.with_file(SCENARIO_FILE, DOC_COMMENT_ONLY),
        Mutation::ExampleHeadingUnbound => builder.with_file(FEATURE_FILE, EXAMPLE_HEADING_FEATURE),
        Mutation::ScenarioTemplateUnbound => {
            builder.with_file(FEATURE_FILE, TEMPLATE_HEADING_FEATURE)
        }
        Mutation::MalformedFeature => builder.with_file(FEATURE_FILE, MALFORMED_FEATURE),
    }
}

/// Rewrite the conforming scenario module with one textual substitution.
fn replace_scenario(builder: SnapshotBuilder, pattern: &str, replacement: &str) -> SnapshotBuilder {
    builder.with_file(
        SCENARIO_FILE,
        EXAMPLE_SCENARIO.replace(pattern, replacement),
    )
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

/// Register the scenario in one harness and the fixture in the other.
fn split_registrations(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(FIXTURE_REGISTRY, "").with_file(
        POOL_FIXTURES,
        "#[path = \"../fixtures/example.rs\"]\npub mod example;\n",
    )
}

/// Point the binding at a feature file that does not exist.
fn wrong_feature_path(builder: SnapshotBuilder) -> SnapshotBuilder {
    let source = EXAMPLE_SCENARIO.replace(FEATURE_FILE, "tests/features/other.feature");
    builder.with_file(SCENARIO_FILE, source)
}

/// Bind the same title twice by repeating the scenario module.
fn duplicate_binding(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(
        SCENARIO_FILE,
        format!("{EXAMPLE_SCENARIO}\n{EXAMPLE_SCENARIO}"),
    )
}

/// Provide both the flat and nested fixture file forms.
fn ambiguous_fixture(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder.with_file(
        "tests/fixtures/example/mod.rs",
        format!("//! Nested fixture.\n{EXAMPLE_FIXTURE}"),
    )
}

/// Point the scenario registry entry at another existing file.
fn scenario_declaration_wrong_target(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .with_file(
            "tests/scenarios/other_scenarios.rs",
            "//! Other scenarios.\n",
        )
        .with_file(
            SCENARIO_REGISTRY,
            "#[path = \"other_scenarios.rs\"]\nmod example_scenarios;\n",
        )
}

/// Point the fixture registry entry at another existing file.
fn fixture_declaration_wrong_target(builder: SnapshotBuilder) -> SnapshotBuilder {
    builder
        .with_file("tests/fixtures/other.rs", "//! Other fixture.\n")
        .with_file(
            FIXTURE_REGISTRY,
            "#[path = \"other.rs\"]\npub mod example;\n",
        )
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
