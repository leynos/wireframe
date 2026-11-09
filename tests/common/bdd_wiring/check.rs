//! Pure evaluation of BDD family naming and registration rules.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use super::{
    bindings::check_scenario_module,
    harness::{
        FEATURE_DIRECTORY,
        FIXTURE_DIRECTORY,
        HarnessInventory,
        SCENARIO_DIRECTORY,
        feature_stems,
        resolve_inventories,
    },
    orphans::check_orphans,
    snapshot::{RepoSnapshot, Violation},
    syntax::ModuleDecl,
};

/// Which registry a family should be declared in.
#[derive(Clone, Copy, Debug)]
enum Column {
    /// Scenario modules.
    Scenarios,
    /// Fixture modules.
    Fixtures,
}

/// Expected files for one BDD family.
struct FamilyPaths {
    /// Feature file.
    feature: Utf8PathBuf,
    /// Scenario module file.
    scenario: Utf8PathBuf,
    /// Flat fixture file.
    flat_fixture: Utf8PathBuf,
    /// Nested fixture file.
    nested_fixture: Utf8PathBuf,
    /// Fixture file selected by the snapshot, when present.
    fixture: Option<Utf8PathBuf>,
}

/// Registrations of one module name across both harnesses.
struct Registration {
    /// Harnesses that declare the name.
    harnesses: BTreeSet<&'static str>,
    /// Matching declarations.
    declarations: Vec<ModuleDecl>,
}

/// Identifies the module a registration is expected to describe.
struct ModuleRef<'a> {
    /// File the registration should resolve to, used in diagnostics.
    path: &'a Utf8Path,
    /// Module kind, such as `scenario` or `fixture`.
    kind: &'a str,
    /// Module name without a file extension.
    module: &'a str,
}

impl Registration {
    /// Require the registration to belong to exactly one harness.
    fn single_harness(
        &self,
        module: &ModuleRef<'_>,
        violations: &mut Vec<Violation>,
    ) -> Option<&'static str> {
        if self.harnesses.is_empty() {
            violations.push(Violation::new(
                module.path,
                format!(
                    "{} module `{}` is not registered in any harness",
                    module.kind, module.module
                ),
            ));
            return None;
        }
        if self.harnesses.len() > 1 {
            violations.push(Violation::new(
                module.path,
                format!(
                    "{} module `{}` is registered in both harnesses",
                    module.kind, module.module
                ),
            ));
            return None;
        }
        self.harnesses.first().copied()
    }
}

/// Evaluate every documented BDD naming and registration rule.
pub(crate) fn evaluate(snapshot: &RepoSnapshot) -> Vec<Violation> {
    let mut violations = Vec::new();
    let inventories = resolve_inventories(snapshot, &mut violations);
    let stems = feature_stems(snapshot);
    for stem in &stems {
        check_family(snapshot, stem, &inventories, &mut violations);
    }
    check_orphans(snapshot, &inventories, &stems, &mut violations);
    violations
}

/// Check every rule for one feature family.
fn check_family(
    snapshot: &RepoSnapshot,
    stem: &str,
    inventories: &[HarnessInventory],
    violations: &mut Vec<Violation>,
) {
    let mut paths = family_paths(stem);
    check_family_files(snapshot, &mut paths, stem, violations);
    let scenario_harness = check_scenario_registration(inventories, &paths, stem, violations);
    let fixture_harness = check_fixture_registration(inventories, &paths, stem, violations);
    if let (Some(scenario), Some(fixture)) = (scenario_harness, fixture_harness)
        && scenario != fixture
    {
        violations.push(Violation::new(
            &paths.scenario,
            format!(
                "scenario and fixture modules for `{stem}` must be registered in the same harness"
            ),
        ));
    }
    if snapshot.contains(&paths.scenario) {
        violations.extend(check_scenario_module(
            snapshot,
            &paths.feature,
            &paths.scenario,
            stem,
        ));
    }
}

/// Build the expected file paths for one family.
fn family_paths(stem: &str) -> FamilyPaths {
    FamilyPaths {
        feature: Utf8PathBuf::from(format!("{FEATURE_DIRECTORY}/{stem}.feature")),
        scenario: Utf8PathBuf::from(format!("{SCENARIO_DIRECTORY}/{stem}_scenarios.rs")),
        flat_fixture: Utf8PathBuf::from(format!("{FIXTURE_DIRECTORY}/{stem}.rs")),
        nested_fixture: Utf8PathBuf::from(format!("{FIXTURE_DIRECTORY}/{stem}/mod.rs")),
        fixture: None,
    }
}

/// Check that the family's scenario and fixture files exist.
fn check_family_files(
    snapshot: &RepoSnapshot,
    paths: &mut FamilyPaths,
    stem: &str,
    violations: &mut Vec<Violation>,
) {
    if !snapshot.contains(&paths.scenario) {
        violations.push(Violation::new(
            &paths.scenario,
            format!("missing scenario module for `{stem}`"),
        ));
    }
    let has_flat = snapshot.contains(&paths.flat_fixture);
    let has_nested = snapshot.contains(&paths.nested_fixture);
    if has_flat && has_nested {
        violations.push(Violation::new(
            &paths.flat_fixture,
            format!(
                "fixture for `{stem}` is ambiguous: `{}` and `{}` both exist",
                paths.flat_fixture, paths.nested_fixture
            ),
        ));
    }
    paths.fixture = if has_flat {
        Some(paths.flat_fixture.clone())
    } else if has_nested {
        Some(paths.nested_fixture.clone())
    } else {
        violations.push(Violation::new(
            &paths.flat_fixture,
            format!(
                "missing fixture module for `{stem}`; add `{}` or `{}`",
                paths.flat_fixture, paths.nested_fixture
            ),
        ));
        None
    };
}

/// Check that the scenario module is registered in exactly one harness.
fn check_scenario_registration(
    inventories: &[HarnessInventory],
    paths: &FamilyPaths,
    stem: &str,
    violations: &mut Vec<Violation>,
) -> Option<&'static str> {
    let module = format!("{stem}_scenarios");
    let registration = registering(inventories, Column::Scenarios, &module);
    let module_ref = ModuleRef {
        path: &paths.scenario,
        kind: "scenario",
        module: &module,
    };
    let harness = registration.single_harness(&module_ref, violations)?;
    for declaration in &registration.declarations {
        if declaration.target != paths.scenario {
            violations.push(Violation::new(
                &paths.scenario,
                format!(
                    "scenario module `{module}` resolves to `{}` but must resolve to `{}`",
                    declaration.target, paths.scenario
                ),
            ));
        }
    }
    Some(harness)
}

/// Check that the fixture module is registered publicly in exactly one harness.
fn check_fixture_registration(
    inventories: &[HarnessInventory],
    paths: &FamilyPaths,
    stem: &str,
    violations: &mut Vec<Violation>,
) -> Option<&'static str> {
    let expected = paths.fixture.as_ref()?;
    let registration = registering(inventories, Column::Fixtures, stem);
    let module_ref = ModuleRef {
        path: expected,
        kind: "fixture",
        module: stem,
    };
    let harness = registration.single_harness(&module_ref, violations)?;
    for declaration in &registration.declarations {
        if declaration.target != *expected {
            violations.push(Violation::new(
                expected,
                format!(
                    "fixture module `{stem}` resolves to `{}` but must resolve to `{expected}`",
                    declaration.target
                ),
            ));
        }
        if !declaration.is_public {
            violations.push(Violation::new(
                expected,
                format!("fixture module `{stem}` must be declared `pub`"),
            ));
        }
    }
    Some(harness)
}

/// Collect a module name's registrations across both harnesses.
fn registering(inventories: &[HarnessInventory], column: Column, module: &str) -> Registration {
    let mut registration = Registration {
        harnesses: BTreeSet::new(),
        declarations: Vec::new(),
    };
    for inventory in inventories {
        let modules = match column {
            Column::Scenarios => &inventory.scenarios,
            Column::Fixtures => &inventory.fixtures,
        };
        for declaration in modules
            .iter()
            .filter(|declaration| declaration.name == module)
        {
            registration.harnesses.insert(inventory.harness.name);
            registration.declarations.push(declaration.clone());
        }
    }
    registration
}
