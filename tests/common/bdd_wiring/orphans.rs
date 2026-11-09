//! Orphan detection for scenario and fixture modules.
//!
//! A module that has no matching feature cannot belong to a family, so it is
//! either a leftover from a removed feature or a registration mistake. Both
//! on-disk files and registry declarations are reported.

use std::collections::BTreeSet;

use camino::Utf8Path;

use super::{
    harness::{FEATURE_DIRECTORY, HarnessInventory, SCENARIO_DIRECTORY},
    snapshot::{RepoSnapshot, Violation},
};

/// Report scenario and fixture modules that have no matching feature.
pub(crate) fn check_orphans(
    snapshot: &RepoSnapshot,
    inventories: &[HarnessInventory],
    stems: &BTreeSet<String>,
    violations: &mut Vec<Violation>,
) {
    for path in snapshot.paths() {
        if path.parent() != Some(Utf8Path::new(SCENARIO_DIRECTORY)) {
            continue;
        }
        let Some(file_stem) = path.file_stem() else {
            continue;
        };
        let Some(stem) = file_stem.strip_suffix("_scenarios") else {
            continue;
        };
        if !stems.contains(stem) {
            violations.push(Violation::new(
                path,
                format!("orphan scenario module: no `{FEATURE_DIRECTORY}/{stem}.feature`"),
            ));
        }
    }
    for inventory in inventories {
        check_registered_orphans(inventory, stems, violations);
    }
}

/// Report registered modules that have no matching feature.
fn check_registered_orphans(
    inventory: &HarnessInventory,
    stems: &BTreeSet<String>,
    violations: &mut Vec<Violation>,
) {
    for declaration in &inventory.scenarios {
        if let Some(stem) = declaration.name.strip_suffix("_scenarios")
            && !stems.contains(stem)
        {
            violations.push(Violation::new(
                &declaration.target,
                format!(
                    "registered scenario module `{}` has no matching feature",
                    declaration.name
                ),
            ));
        }
    }
    for declaration in inventory
        .fixtures
        .iter()
        .filter(|declaration| declaration.is_public)
    {
        if !stems.contains(&declaration.name) {
            violations.push(Violation::new(
                &declaration.target,
                format!(
                    "public fixture module `{}` has no matching feature",
                    declaration.name
                ),
            ));
        }
    }
}
