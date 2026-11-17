//! Regression tests for the BDD family naming and registration contract.
//!
//! The live test runs the checker against this repository so drift in
//! `tests/features`, `tests/scenarios`, or `tests/fixtures` fails `make test`.
//! The synthetic cases prove each rule reports its violation instead of
//! passing vacuously.

#![forbid(unsafe_code)]

#[path = "common/fallible_assertions/check.rs"]
mod fallible_check;

#[path = "common/repo_access.rs"]
mod repo_access;

#[path = "common/bdd_wiring/mod.rs"]
mod bdd_wiring;

use std::collections::{BTreeMap, BTreeSet};

use bdd_wiring::{
    Mutation,
    RepoSnapshot,
    Violation,
    alternate_heading_snapshot,
    base_snapshot,
    doc_string_snapshot,
    evaluate,
    feature_stems,
    harness_scenario_modules,
    mutated_snapshot,
    nested_fixture_snapshot,
};
use fallible_check::check;
use rstest::rstest;

/// Result type for wiring-check tests.
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Render violations one per line for failure output.
fn render(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Require one violation with the expected path and reason fragment.
fn expect_violation(
    violations: &[Violation],
    expected_path: &str,
    expected_fragment: &str,
) -> Result<(), String> {
    let matched = violations.iter().any(|violation| {
        violation.path.as_str() == expected_path && violation.reason.contains(expected_fragment)
    });
    check(
        matched,
        format!(
            "expected a violation at `{expected_path}` containing `{expected_fragment}`; got:\n{}",
            render(violations)
        ),
    )
}

#[test]
fn live_repository_families_are_wired() -> TestResult {
    let snapshot = RepoSnapshot::from_repository()?;
    assert_no_violations(&snapshot, "the live repository")?;
    let stems = feature_stems(&snapshot);
    check(!stems.is_empty(), "expected at least one feature file")?;
    let harnesses = harness_scenario_modules(&snapshot);
    assert_harnesses_register_families(&harnesses)?;
    assert_pool_families_are_registered(&harnesses)
}

/// Require every BDD harness to register at least one family.
fn assert_harnesses_register_families(
    harnesses: &BTreeMap<&'static str, BTreeSet<String>>,
) -> TestResult {
    for harness in ["bdd", "bdd_pool"] {
        let registered = harnesses.get(harness).map_or(0, BTreeSet::len);
        check(
            registered > 0,
            format!("harness `{harness}` should register at least one family"),
        )?;
    }
    Ok(())
}

/// Require the pool harness to register each pool family.
fn assert_pool_families_are_registered(
    harnesses: &BTreeMap<&'static str, BTreeSet<String>>,
) -> TestResult {
    let pool = harnesses
        .get("bdd_pool")
        .ok_or("the pool harness inventory is missing")?;
    for family in ["client_pool", "client_pool_handle", "client_pool_rotation"] {
        let module = format!("{family}_scenarios");
        check(
            pool.contains(&module),
            format!("the pool harness should register `{module}`"),
        )?;
    }
    Ok(())
}

/// Require a snapshot to evaluate without violations.
fn assert_no_violations(snapshot: &RepoSnapshot, description: &str) -> TestResult {
    let violations = evaluate(snapshot);
    check(
        violations.is_empty(),
        format!(
            "{description} should have no violations; got:\n{}",
            render(&violations)
        ),
    )?;
    Ok(())
}

#[test]
fn conforming_synthetic_layout_has_no_violations() -> TestResult {
    assert_no_violations(&base_snapshot().build(), "the conforming layout")
}

#[test]
fn nested_fixture_layout_is_accepted() -> TestResult {
    assert_no_violations(
        &nested_fixture_snapshot().build(),
        "the nested fixture layout",
    )
}

#[test]
fn alternate_scenario_headings_are_accepted() -> TestResult {
    assert_no_violations(
        &alternate_heading_snapshot().build(),
        "the alternative heading layout",
    )
}

#[test]
fn doc_string_text_is_not_a_scenario_title() -> TestResult {
    assert_no_violations(
        &doc_string_snapshot().build(),
        "a feature whose doc string contains heading-like text",
    )
}

#[rstest]
#[case::missing_scenario(
    Mutation::MissingScenario,
    "tests/scenarios/example_scenarios.rs",
    "missing scenario module"
)]
#[case::missing_fixture(
    Mutation::MissingFixture,
    "tests/fixtures/example.rs",
    "missing fixture module"
)]
#[case::unregistered_scenario(
    Mutation::UnregisteredScenario,
    "tests/scenarios/example_scenarios.rs",
    "not registered in any harness"
)]
#[case::registered_twice(
    Mutation::RegisteredTwice,
    "tests/scenarios/example_scenarios.rs",
    "registered in both harnesses"
)]
#[case::wrong_feature_path(
    Mutation::WrongFeaturePath,
    "tests/scenarios/example_scenarios.rs",
    "must reference"
)]
#[case::missing_binding(
    Mutation::MissingBinding,
    "tests/scenarios/example_scenarios.rs",
    "has no binding"
)]
#[case::unmatched_binding(
    Mutation::UnmatchedBinding,
    "tests/scenarios/example_scenarios.rs",
    "does not match a scenario title"
)]
#[case::duplicate_binding(
    Mutation::DuplicateBinding,
    "tests/scenarios/example_scenarios.rs",
    "bound more than once"
)]
#[case::index_selector(
    Mutation::IndexSelector,
    "tests/scenarios/example_scenarios.rs",
    "`index` selectors are not supported"
)]
#[case::missing_import(
    Mutation::MissingImport,
    "tests/scenarios/example_scenarios.rs",
    "must import"
)]
#[case::orphan_scenario(
    Mutation::OrphanScenario,
    "tests/scenarios/orphan_scenarios.rs",
    "orphan scenario module"
)]
#[case::public_fixture_without_feature(
    Mutation::PublicFixtureWithoutFeature,
    "tests/fixtures/orphan.rs",
    "public fixture module"
)]
#[case::unresolvable_path(
    Mutation::UnresolvablePath,
    "tests/scenarios/missing_scenarios.rs",
    "resolves to a missing file"
)]
#[case::doc_comment_only(
    Mutation::DocCommentOnly,
    "tests/scenarios/example_scenarios.rs",
    "has no binding"
)]
#[case::example_heading_unbound(
    Mutation::ExampleHeadingUnbound,
    "tests/scenarios/example_scenarios.rs",
    "scenario title `Unbound example` has no binding"
)]
#[case::scenario_template_unbound(
    Mutation::ScenarioTemplateUnbound,
    "tests/scenarios/example_scenarios.rs",
    "scenario title `Unbound template` has no binding"
)]
#[case::malformed_feature(
    Mutation::MalformedFeature,
    "tests/features/example.feature",
    "cannot parse feature file"
)]
fn synthetic_violations_are_reported(
    #[case] mutation: Mutation,
    #[case] expected_path: &str,
    #[case] expected_fragment: &str,
) -> TestResult {
    let snapshot = mutated_snapshot(mutation).build();
    let violations = evaluate(&snapshot);
    expect_violation(&violations, expected_path, expected_fragment)?;
    Ok(())
}
