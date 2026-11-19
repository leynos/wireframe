//! Harness inventory for the BDD wiring check.
//!
//! Each harness names its root module and the registries that declare its
//! scenario and fixture modules. Resolving those registries against a snapshot
//! yields the declarations that the family and orphan rules report against.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;

use super::{
    snapshot::{RepoSnapshot, Violation},
    syntax::{ModuleDecl, module_declarations},
};

/// One BDD harness root and its scenario and fixture registries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Harness {
    /// Name used in diagnostics.
    pub(crate) name: &'static str,
    /// Root module that declares both registries.
    root: &'static str,
    /// Registry that declares scenario modules.
    pub(crate) scenario_registry: &'static str,
    /// Registry that declares fixture modules.
    pub(crate) fixture_registry: &'static str,
}

/// The harnesses that own BDD families.
const HARNESSES: [Harness; 2] = [
    Harness {
        name: "bdd",
        root: "tests/bdd/mod.rs",
        scenario_registry: "tests/scenarios/mod.rs",
        fixture_registry: "tests/fixtures/mod.rs",
    },
    Harness {
        name: "bdd_pool",
        root: "tests/bdd_pool/mod.rs",
        scenario_registry: "tests/bdd_pool/scenarios.rs",
        fixture_registry: "tests/bdd_pool/fixtures.rs",
    },
];

/// Directory holding feature files.
pub(crate) const FEATURE_DIRECTORY: &str = "tests/features";
/// Directory holding scenario modules.
pub(crate) const SCENARIO_DIRECTORY: &str = "tests/scenarios";
/// Directory holding fixture modules.
pub(crate) const FIXTURE_DIRECTORY: &str = "tests/fixtures";

/// Resolved registries for one harness.
pub(crate) struct HarnessInventory {
    /// Harness description.
    pub(crate) harness: Harness,
    /// Scenario modules declared under `tests/scenarios`.
    pub(crate) scenarios: Vec<ModuleDecl>,
    /// Fixture modules declared under `tests/fixtures`.
    pub(crate) fixtures: Vec<ModuleDecl>,
}

/// Resolve every harness's registries.
pub(crate) fn resolve_inventories(
    snapshot: &RepoSnapshot,
    violations: &mut Vec<Violation>,
) -> Vec<HarnessInventory> {
    HARNESSES
        .iter()
        .map(|harness| harness_inventory(snapshot, harness, violations))
        .collect()
}

/// Return the stems of every feature file in a snapshot.
pub(crate) fn feature_stems(snapshot: &RepoSnapshot) -> BTreeSet<String> {
    snapshot
        .paths()
        .filter(|path| path.parent() == Some(Utf8Path::new(FEATURE_DIRECTORY)))
        .filter(|path| path.extension() == Some("feature"))
        .filter_map(|path| path.file_stem().map(ToOwned::to_owned))
        .collect()
}

/// Return the scenario module names registered by each harness.
pub(crate) fn harness_scenario_modules(
    snapshot: &RepoSnapshot,
) -> BTreeMap<&'static str, BTreeSet<String>> {
    HARNESSES
        .iter()
        .map(|harness| {
            let mut ignored = Vec::new();
            let modules = registry_modules(
                snapshot,
                harness.scenario_registry,
                SCENARIO_DIRECTORY,
                &mut ignored,
            );
            (
                harness.name,
                modules.into_iter().map(|module| module.name).collect(),
            )
        })
        .collect()
}

/// Resolve one harness's registries and check its root-to-registry edges.
fn harness_inventory(
    snapshot: &RepoSnapshot,
    harness: &Harness,
    violations: &mut Vec<Violation>,
) -> HarnessInventory {
    check_root_edges(snapshot, harness, violations);
    HarnessInventory {
        harness: *harness,
        scenarios: registry_modules(
            snapshot,
            harness.scenario_registry,
            SCENARIO_DIRECTORY,
            violations,
        ),
        fixtures: registry_modules(
            snapshot,
            harness.fixture_registry,
            FIXTURE_DIRECTORY,
            violations,
        ),
    }
}

/// Require the harness root to declare both of its registries.
fn check_root_edges(snapshot: &RepoSnapshot, harness: &Harness, violations: &mut Vec<Violation>) {
    let root = Utf8Path::new(harness.root);
    let declarations = match module_declarations(snapshot, root) {
        Ok(declarations) => declarations,
        Err(reason) => {
            violations.push(Violation::new(root, reason));
            return;
        }
    };
    for registry in [harness.scenario_registry, harness.fixture_registry] {
        let declared = declarations
            .iter()
            .any(|declaration| declaration.target.as_path() == Utf8Path::new(registry));
        if !declared {
            violations.push(Violation::new(
                root,
                format!("harness `{}` must declare `{registry}`", harness.name),
            ));
        }
    }
}

/// Resolve the in-scope declarations in one registry.
fn registry_modules(
    snapshot: &RepoSnapshot,
    registry: &str,
    scope: &str,
    violations: &mut Vec<Violation>,
) -> Vec<ModuleDecl> {
    let path = Utf8Path::new(registry);
    let declarations = match module_declarations(snapshot, path) {
        Ok(declarations) => declarations,
        Err(reason) => {
            violations.push(Violation::new(path, reason));
            return Vec::new();
        }
    };
    declarations
        .into_iter()
        .filter_map(|declaration| {
            if !declaration.target.starts_with(scope) {
                return None;
            }
            if snapshot.contains(&declaration.target) {
                Some(declaration)
            } else {
                violations.push(Violation::new(
                    &declaration.target,
                    format!(
                        "module declaration `{}` resolves to a missing file",
                        declaration.name
                    ),
                ));
                None
            }
        })
        .collect()
}
