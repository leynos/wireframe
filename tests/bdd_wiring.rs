//! Regression tests for the BDD family naming and registration contract.
//!
//! The live test runs the checker against this repository so drift in
//! `tests/features`, `tests/scenarios`, or `tests/fixtures` fails `make test`.
//! Each mutation case breaks one documented rule and asserts the violation
//! that rule must produce, so the rule cannot silently stop firing.

#![forbid(unsafe_code)]

#[path = "common/fallible_assertions/check.rs"]
mod fallible_check;

#[path = "common/repo_access.rs"]
mod repo_access;

#[path = "common/bdd_wiring/mod.rs"]
mod bdd_wiring;

use std::collections::{BTreeMap, BTreeSet};

use bdd_wiring::{
    ImportForm,
    Mutation,
    RepoSnapshot,
    Violation,
    alternate_heading_snapshot,
    base_snapshot,
    doc_string_snapshot,
    evaluate,
    feature_stems,
    harness_scenario_modules,
    import_form_snapshot,
    mutated_snapshot,
    named_import_snapshot,
    nested_fixture_snapshot,
    rule_scenario_snapshot,
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

/// A violation a case expects, naming its path and a fragment of its reason.
///
/// The two values always travel together — a case cannot ask for a reason
/// fragment without also saying where to look — so they form one value rather
/// than two loose arguments.
#[derive(Clone, Copy)]
struct Expectation {
    /// Repository-relative path the violation must name.
    path: &'static str,
    /// Text the violation's reason must contain.
    fragment: &'static str,
}

impl Expectation {
    /// Describe an expected violation.
    const fn new(path: &'static str, fragment: &'static str) -> Self { Self { path, fragment } }
}

/// Require one violation matching the expectation.
fn expect_violation(violations: &[Violation], expected: Expectation) -> Result<(), String> {
    let matched = violations.iter().any(|violation| {
        violation.path.as_str() == expected.path && violation.reason.contains(expected.fragment)
    });
    check(
        matched,
        format!(
            "expected a violation at `{}` containing `{}`; got:\n{}",
            expected.path,
            expected.fragment,
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

#[test]
fn rule_grouped_scenario_is_bound() -> TestResult {
    assert_no_violations(
        &rule_scenario_snapshot().build(),
        "a feature whose Rule scenario is bound",
    )
}

#[test]
fn named_fixture_imports_are_accepted() -> TestResult {
    assert_no_violations(
        &named_import_snapshot().build(),
        "a scenario module importing its fixture by name, rename, and group",
    )
}

/// Each single `use` form is accepted on its own, so one form cannot mask
/// another form's import from being recognized.
#[rstest]
#[case::name_import(ImportForm::Name)]
#[case::rename_import(ImportForm::Rename)]
#[case::group_import(ImportForm::Group)]
fn each_fixture_import_form_is_accepted(#[case] form: ImportForm) -> TestResult {
    assert_no_violations(
        &import_form_snapshot(form).build(),
        "a scenario module importing its fixture in one `use` form",
    )
}

#[rstest]
#[case::missing_scenario(
    Mutation::MissingScenario,
    Expectation::new("tests/scenarios/example_scenarios.rs", "missing scenario module")
)]
#[case::missing_fixture(
    Mutation::MissingFixture,
    Expectation::new("tests/fixtures/example.rs", "missing fixture module")
)]
#[case::unregistered_scenario(
    Mutation::UnregisteredScenario,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "not registered in any harness"
    )
)]
#[case::registered_twice(
    Mutation::RegisteredTwice,
    Expectation::new("tests/scenarios/example_scenarios.rs", "registered in both harnesses")
)]
#[case::wrong_feature_path(
    Mutation::WrongFeaturePath,
    Expectation::new("tests/scenarios/example_scenarios.rs", "must reference")
)]
#[case::missing_binding(
    Mutation::MissingBinding,
    Expectation::new("tests/scenarios/example_scenarios.rs", "has no binding")
)]
#[case::unmatched_binding(
    Mutation::UnmatchedBinding,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "does not match a scenario title"
    )
)]
#[case::duplicate_binding(
    Mutation::DuplicateBinding,
    Expectation::new("tests/scenarios/example_scenarios.rs", "bound more than once")
)]
#[case::index_selector(
    Mutation::IndexSelector,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "`index` selectors are not supported"
    )
)]
#[case::missing_import(
    Mutation::MissingImport,
    Expectation::new("tests/scenarios/example_scenarios.rs", "must import")
)]
#[case::orphan_scenario(
    Mutation::OrphanScenario,
    Expectation::new("tests/scenarios/orphan_scenarios.rs", "orphan scenario module")
)]
#[case::public_fixture_without_feature(
    Mutation::PublicFixtureWithoutFeature,
    Expectation::new("tests/fixtures/orphan.rs", "public fixture module")
)]
#[case::unresolvable_path(
    Mutation::UnresolvablePath,
    Expectation::new("tests/scenarios/missing_scenarios.rs", "resolves to a missing file")
)]
#[case::doc_comment_only(
    Mutation::DocCommentOnly,
    Expectation::new("tests/scenarios/example_scenarios.rs", "has no binding")
)]
#[case::example_heading_unbound(
    Mutation::ExampleHeadingUnbound,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "scenario title `Unbound example` has no binding"
    )
)]
#[case::scenario_template_unbound(
    Mutation::ScenarioTemplateUnbound,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "scenario title `Unbound template` has no binding"
    )
)]
#[case::malformed_feature(
    Mutation::MalformedFeature,
    Expectation::new("tests/features/example.feature", "cannot parse feature file")
)]
#[case::split_registrations(
    Mutation::SplitRegistrations,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "must be registered in the same harness"
    )
)]
#[case::rule_scenario_unbound(
    Mutation::RuleScenarioUnbound,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "scenario title `Grouped step` has no binding"
    )
)]
#[case::non_literal_name(
    Mutation::NonLiteralName,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "`name` must be a string literal"
    )
)]
#[case::non_literal_path(
    Mutation::NonLiteralPath,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "`path` must be a string literal"
    )
)]
#[case::missing_name(
    Mutation::MissingName,
    Expectation::new("tests/scenarios/example_scenarios.rs", "must provide a `name`")
)]
#[case::self_import(
    Mutation::SelfImport,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "must import `crate::fixtures::example`"
    )
)]
#[case::ambiguous_fixture(
    Mutation::AmbiguousFixture,
    Expectation::new("tests/fixtures/example.rs", "is ambiguous")
)]
#[case::missing_registry_declaration(
    Mutation::MissingRegistryDeclaration,
    Expectation::new("tests/bdd/mod.rs", "must declare")
)]
#[case::scenario_declaration_wrong_target(
    Mutation::ScenarioDeclarationWrongTarget,
    Expectation::new("tests/scenarios/example_scenarios.rs", "but must resolve to")
)]
#[case::fixture_declaration_wrong_target(
    Mutation::FixtureDeclarationWrongTarget,
    Expectation::new("tests/fixtures/example.rs", "but must resolve to")
)]
#[case::private_fixture(
    Mutation::PrivateFixture,
    Expectation::new("tests/fixtures/example.rs", "must be declared `pub`")
)]
#[case::registered_orphan_scenario(
    Mutation::RegisteredOrphanScenario,
    Expectation::new(
        "tests/scenarios/orphan_scenarios.rs",
        "registered scenario module `orphan_scenarios` has no matching feature"
    )
)]
#[case::wrong_fixture_import(
    Mutation::WrongFixtureImport,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "must import `crate::fixtures::example`"
    )
)]
#[case::self_import_aliased(
    Mutation::SelfImportAliased,
    Expectation::new(
        "tests/scenarios/example_scenarios.rs",
        "must import `crate::fixtures::example`"
    )
)]
fn synthetic_violations_are_reported(
    #[case] mutation: Mutation,
    #[case] expected: Expectation,
) -> TestResult {
    let snapshot = mutated_snapshot(mutation).build();
    let violations = evaluate(&snapshot);
    expect_violation(&violations, expected)?;
    Ok(())
}
