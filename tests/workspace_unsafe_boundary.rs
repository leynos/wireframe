//! Contract tests for the workspace-wide `unsafe_code` prohibition.
//!
//! `[workspace.lints.rust]` sets `unsafe_code = "forbid"`, and every Cargo
//! target root states the same prohibition in its own source. The manifest table
//! reaches only packages that opt in with `[lints] workspace = true`, and the
//! standalone `tests/feature_gate` package cannot inherit it at all, so the
//! source-level rule is what makes the prohibition impossible to lose by
//! omission.
//!
//! These tests discover every target root Cargo knows about — automatic and
//! explicitly configured, workspace member and standalone package — and fail if
//! one does not forbid unsafe Rust, if a member stops inheriting the workspace
//! lint tables, if workspace membership changes without an inventory decision,
//! or if unsafe syntax appears in any package source.
//!
//! A textual contract cannot prove that the attribute actually rejects unsafe
//! code. `make unsafe-boundary` compiles deliberate unsafe probes on the pinned
//! toolchain for that half, including the control that `forbid` overrules an
//! `#[expect(unsafe_code)]`. See `docs/rust-boundary-verification.md` for why
//! the two halves are separate and what neither of them establishes.

#![forbid(unsafe_code)]

#[path = "common/cargo_targets.rs"]
mod cargo_targets;
#[path = "common/inner_attributes.rs"]
mod inner_attributes;
#[path = "common/package_sources.rs"]
mod package_sources;
#[path = "common/repo_access.rs"]
mod repo_access;
#[path = "common/unsafe_syntax.rs"]
mod unsafe_syntax;

use camino::Utf8Path;
use cargo_targets::{describe, discover_target_roots, read_target_source};
use inner_attributes::has_header_attribute;
use package_sources::{
    sources_in_package,
    sources_under,
    workspace_package_manifests,
    workspace_package_names,
};
use repo_access::{read_repo_file, repo_root};
use unsafe_syntax::violations_in;

/// The source-level prohibition every target root must carry.
const PROHIBITION: &str = "#![forbid(unsafe_code)]";

/// Directory of the standalone package `cargo metadata` does not describe.
///
/// `tests/feature_gate` declares its own `[workspace]`, so the repository's
/// metadata omits it and its sources have to be found by path.
const FEATURE_GATE_DIRECTORY: &str = "tests/feature_gate";

/// Name of the standalone package, which must stay outside the workspace.
const FEATURE_GATE_PACKAGE: &str = "wireframe-feature-gate";

/// The exact workspace membership this repository has reviewed.
const EXPECTED_MEMBERS: &[&str] = &[
    "wireframe",
    "wireframe-loom",
    "wireframe-verification",
    "wireframe_testing",
];

/// Paths exempt from the text scan because they name what it looks for.
///
/// Scanning them would report the scanner rather than a violation. Each is a
/// test helper that compiles under `#![forbid(unsafe_code)]`, which is the
/// check that matters for its own source.
const EXEMPT_PATHS: &[&str] = &[
    "tests/common/unsafe_syntax.rs",
    "tests/common/unsafe_syntax/tests.rs",
    "tests/workspace_unsafe_boundary.rs",
];

/// Result type shared by these contract checks.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Read and parse one Cargo manifest from the repository.
fn read_manifest(path: &Utf8Path) -> TestResult<toml::Value> {
    let text = read_repo_file(path)?;
    Ok(toml::from_str(&text)?)
}

/// Return the `unsafe_code` level a manifest's `[lints.rust]` table declares.
fn declared_unsafe_level(manifest: &toml::Value) -> Option<&str> {
    manifest
        .get("lints")?
        .get("rust")?
        .get("unsafe_code")?
        .as_str()
}

/// Assert the workspace lint table forbids unsafe code.
#[test]
fn workspace_manifest_forbids_unsafe_code() -> TestResult {
    let manifest = read_manifest(Utf8Path::new("Cargo.toml"))?;
    let level = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("lints"))
        .and_then(|lints| lints.get("rust"))
        .and_then(|rust| rust.get("unsafe_code"))
        .and_then(toml::Value::as_str);
    match level {
        Some("forbid") => Ok(()),
        other => Err(format!(
            "[workspace.lints.rust] should set `unsafe_code = \"forbid\"`; found {other:?}"
        )
        .into()),
    }
}

/// Assert every workspace member inherits the workspace lint tables.
///
/// Cargo applies `[workspace.lints]` only to a package that opts in with
/// `[lints] workspace = true`. A member that drops the opt-in silently loses
/// every workspace lint, including this prohibition, so the opt-in is part of
/// the contract rather than an implementation detail.
#[test]
fn workspace_members_inherit_the_lint_tables() -> TestResult {
    let mut violations = Vec::new();
    for (package, manifest_path) in workspace_package_manifests()? {
        let manifest = read_manifest(&manifest_path)?;
        let Some(lints) = manifest.get("lints").and_then(toml::Value::as_table) else {
            violations.push(format!("{package} ({manifest_path}) has no [lints] table"));
            continue;
        };
        let inherits = lints.get("workspace").and_then(toml::Value::as_bool) == Some(true);
        if !inherits {
            violations.push(format!(
                "{package} ({manifest_path}) does not set `[lints] workspace = true`"
            ));
        }
        let local_tables = lints
            .keys()
            .filter(|key| key.as_str() != "workspace")
            .collect::<Vec<_>>();
        if !local_tables.is_empty() {
            violations.push(format!(
                "{package} ({manifest_path}) declares package-local lint tables {local_tables:?}; \
                 Cargo rejects them beside the inheritance opt-in"
            ));
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} workspace lint-inheritance violation(s):\n  {}",
        violations.len(),
        violations.join("\n  ")
    )
    .into())
}

/// Assert the standalone package copies the prohibition it cannot inherit.
///
/// Its own `[workspace]` table stops Cargo from finding the repository manifest,
/// so `[lints] workspace = true` cannot resolve there. The table is hand-copied
/// instead, and copying is exactly the kind of work that drifts.
#[test]
fn the_feature_gate_package_declares_the_prohibition() -> TestResult {
    let manifest = read_manifest(
        Utf8Path::new(FEATURE_GATE_DIRECTORY)
            .join("Cargo.toml")
            .as_path(),
    )?;
    match declared_unsafe_level(&manifest) {
        Some("forbid") => Ok(()),
        other => Err(format!(
            "{FEATURE_GATE_DIRECTORY}/Cargo.toml should set `unsafe_code = \"forbid\"` in its \
             copied `[lints.rust]` table; found {other:?}"
        )
        .into()),
    }
}

/// Assert the standalone package has not silently joined the workspace.
///
/// Joining would make its hand-copied tables disappear and replace the
/// standalone compile that `make test-feature-gate` depends on.
#[test]
fn the_feature_gate_package_stays_standalone() -> TestResult {
    let members = workspace_package_names()?;
    if members.iter().any(|member| member == FEATURE_GATE_PACKAGE) {
        return Err(format!(
            "{FEATURE_GATE_PACKAGE} must not be a workspace member: it is compiled through \
             {FEATURE_GATE_DIRECTORY}/Cargo.toml by `make test-feature-gate`"
        )
        .into());
    }
    Ok(())
}

/// Assert every target root forbids unsafe Rust.
#[test]
fn every_target_root_forbids_unsafe_code() -> TestResult {
    let repo = repo_root()?;
    let roots = discover_target_roots()?;
    if roots.is_empty() {
        return Err("target discovery returned no roots; this check would pass vacuously".into());
    }
    let mut violations = Vec::new();
    for root in roots {
        let source = read_target_source(&root, &repo)?;
        if !has_header_attribute(&source, PROHIBITION) {
            violations.push(describe(&root, &repo));
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    violations.sort();
    Err(format!(
        "{} target root(s) do not carry `{PROHIBITION}` in their header:\n  {}",
        violations.len(),
        violations.join("\n  ")
    )
    .into())
}

/// Assert discovery reaches the package the workspace metadata omits.
///
/// A workspace-only metadata run would leave the standalone package's roots
/// unchecked while the target-root contract still passed, so the second query
/// is pinned here rather than left implicit in the combined result.
#[test]
fn target_discovery_includes_the_standalone_package() -> TestResult {
    let packages: Vec<String> = discover_target_roots()?
        .into_iter()
        .map(|root| root.package)
        .collect();
    if packages
        .iter()
        .any(|package| package == FEATURE_GATE_PACKAGE)
    {
        return Ok(());
    }
    Err(format!(
        "target discovery reported no {FEATURE_GATE_PACKAGE} root; discovered packages: \
         {packages:?}"
    )
    .into())
}

/// Read every source file of every package, including the standalone one.
///
/// Target roots are not enough: a child module behind a disabled feature is
/// committed source that the compiler never sees until the feature is enabled.
/// Reading each package's `.rs` files checks the text that exists rather than
/// the text one feature set compiles. The repository root is the `wireframe`
/// package's directory, so its walk descends into `crates/` and
/// `tests/feature_gate`; the standalone package is added by path so its
/// hand-copied manifest and probe binary cannot fall outside the scan.
fn all_package_sources() -> TestResult<Vec<(String, String)>> {
    let mut sources = std::collections::BTreeMap::new();
    for package in workspace_package_names()? {
        for (path, source) in sources_in_package(&package)? {
            sources.insert(path.to_string(), source);
        }
    }
    for (path, source) in sources_under(Utf8Path::new(FEATURE_GATE_DIRECTORY))? {
        sources.insert(path.to_string(), source);
    }
    Ok(sources.into_iter().collect())
}

/// Assert no package contains unsafe syntax or an unsafe-code override.
#[test]
fn unsafe_syntax_is_absent_from_every_package() -> TestResult {
    let sources = all_package_sources()?;
    if sources.is_empty() {
        return Err(
            "package source discovery returned no files; this check would pass vacuously".into(),
        );
    }
    let mut violations = Vec::new();
    for (path, source) in sources {
        if EXEMPT_PATHS.contains(&path.as_str()) {
            continue;
        }
        violations.extend(violations_in(&source, &path));
    }
    if violations.is_empty() {
        return Ok(());
    }
    violations.sort();
    Err(format!(
        "{} unsafe-code boundary violation(s):\n  {}",
        violations.len(),
        violations.join("\n  ")
    )
    .into())
}

/// Assert workspace membership has not grown without review.
#[test]
fn workspace_membership_matches_the_reviewed_inventory() -> TestResult {
    let mut actual = workspace_package_names()?;
    actual.sort_unstable();

    let mut expected = EXPECTED_MEMBERS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    expected.sort_unstable();

    if actual == expected {
        return Ok(());
    }
    Err(format!(
        "workspace membership changed without an inventory decision.\nexpected: \
         {expected:?}\nactual:   {actual:?}\nAdd the member to `EXPECTED_MEMBERS` and record why \
         in docs/rust-boundary-verification.md, or remove it."
    )
    .into())
}
