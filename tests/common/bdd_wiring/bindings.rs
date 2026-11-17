//! Evaluation of scenario-title bindings and fixture imports.

use camino::Utf8Path;
use syn::{Item, UseTree};

use super::{
    features::{ScenarioBinding, feature_titles, scenario_bindings},
    snapshot::{RepoSnapshot, Violation},
};

/// Check one family's scenario module for bindings and its fixture import.
pub(crate) fn check_scenario_module(
    snapshot: &RepoSnapshot,
    feature_path: &Utf8Path,
    scenario_path: &Utf8Path,
    stem: &str,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let Some(contents) = snapshot.get(scenario_path) else {
        return violations;
    };
    let file = match syn::parse_file(contents) {
        Ok(file) => file,
        Err(error) => {
            violations.push(Violation::new(
                scenario_path,
                format!("cannot parse scenario module: {error}"),
            ));
            return violations;
        }
    };
    if !imports_fixture(&file, stem) {
        violations.push(Violation::new(
            scenario_path,
            format!("scenario module must import `crate::fixtures::{stem}`"),
        ));
    }
    if let Some(feature_contents) = snapshot.get(feature_path) {
        let source = BindingSource {
            contents: feature_contents,
            feature_path,
            scenario_path,
        };
        check_feature_bindings(&source, &file, &mut violations);
    }
    violations
}

/// Feature source and the paths of the modules that bind it.
struct BindingSource<'a> {
    /// Feature file contents.
    contents: &'a str,
    /// Family feature file.
    feature_path: &'a Utf8Path,
    /// Scenario module that must bind the feature's titles.
    scenario_path: &'a Utf8Path,
}

/// Compare a scenario module's bindings with its feature's titles.
fn check_feature_bindings(
    source: &BindingSource<'_>,
    file: &syn::File,
    violations: &mut Vec<Violation>,
) {
    let titles = match feature_titles(source.contents) {
        Ok(titles) => titles,
        Err(reason) => {
            violations.push(Violation::new(source.feature_path, reason));
            return;
        }
    };
    let context = BindingContext {
        titles: &titles,
        feature_path: source.feature_path,
        scenario_path: source.scenario_path,
    };
    context.check(file, &mut Vec::new(), violations);
}

/// Context shared by binding checks for one scenario module.
struct BindingContext<'a> {
    /// Feature titles that bindings must cover.
    titles: &'a [String],
    /// Family feature file that bindings must reference.
    feature_path: &'a Utf8Path,
    /// Scenario module holding the bindings.
    scenario_path: &'a Utf8Path,
}

impl BindingContext<'_> {
    /// Compare every binding with the feature titles.
    fn check(
        &self,
        file: &syn::File,
        bound_names: &mut Vec<String>,
        violations: &mut Vec<Violation>,
    ) {
        for binding in &scenario_bindings(file) {
            self.check_binding(binding, bound_names, violations);
        }
        self.check_title_coverage(bound_names, violations);
    }

    /// Check one binding's path and name against the feature titles.
    fn check_binding(
        &self,
        binding: &ScenarioBinding,
        bound_names: &mut Vec<String>,
        violations: &mut Vec<Violation>,
    ) {
        if let Some(problem) = &binding.problem {
            violations.push(Violation::new(self.scenario_path, problem.clone()));
            return;
        }
        if let Some(path) = &binding.path
            && Utf8Path::new(path) != self.feature_path
        {
            violations.push(Violation::new(
                self.scenario_path,
                format!(
                    "binding path `{path}` must reference `{}`",
                    self.feature_path
                ),
            ));
        }
        let Some(name) = &binding.name else {
            return;
        };
        let trimmed = name.trim();
        let matched = self.titles.iter().any(|title| title == trimmed);
        if !matched {
            violations.push(Violation::new(
                self.scenario_path,
                format!(
                    "binding `{trimmed}` does not match a scenario title in `{}`",
                    self.feature_path
                ),
            ));
        }
        bound_names.push(trimmed.to_owned());
    }

    /// Require exactly one binding for each feature title.
    fn check_title_coverage(&self, bound_names: &[String], violations: &mut Vec<Violation>) {
        for title in self.titles {
            let count = bound_names
                .iter()
                .filter(|name| name.as_str() == title.as_str())
                .count();
            if count == 0 {
                violations.push(Violation::new(
                    self.scenario_path,
                    format!("scenario title `{title}` has no binding"),
                ));
            } else if count > 1 {
                violations.push(Violation::new(
                    self.scenario_path,
                    format!("scenario title `{title}` is bound more than once"),
                ));
            }
        }
    }
}

/// Report whether a scenario module imports its family's fixture module.
fn imports_fixture(file: &syn::File, stem: &str) -> bool {
    file.items.iter().any(|item| match item {
        Item::Use(use_item) => use_tree_imports(&use_item.tree, &mut Vec::new(), stem),
        _ => false,
    })
}

/// Walk a `use` tree looking for `crate::fixtures::<stem>`.
fn use_tree_imports(tree: &UseTree, path: &mut Vec<String>, stem: &str) -> bool {
    match tree {
        UseTree::Path(segment) => {
            path.push(segment.ident.to_string());
            let found = use_tree_imports(&segment.tree, path, stem);
            let _popped = path.pop();
            found
        }
        UseTree::Name(name) => is_fixture_path(path, stem) && name.ident != "self",
        UseTree::Rename(rename) => is_fixture_path(path, stem) && rename.ident != "self",
        UseTree::Glob(_) => is_fixture_path(path, stem),
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| use_tree_imports(item, path, stem)),
    }
}

/// Report whether the accumulated tree path names the family's fixture.
fn is_fixture_path(path: &[String], stem: &str) -> bool {
    let [krate, fixtures, family] = path else {
        return false;
    };
    if krate.as_str() != "crate" || fixtures.as_str() != "fixtures" {
        return false;
    }
    family.as_str() == stem
}
