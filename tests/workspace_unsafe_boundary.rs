//! Contract tests for the workspace's `unsafe_code` boundary.
//!
//! The workspace sets `unsafe_code = "deny"` as a floor, and every safe Cargo
//! target root raises the level to `forbid` in its own source. Cargo cannot let
//! a member override an inherited lint entry, so the prohibition has to be
//! source-level; a source-level rule is only worth having if it cannot be
//! forgotten, which is what these tests check.
//!
//! They discover every target root Cargo knows about — automatic and explicitly
//! configured, workspace member and standalone package — and fail if one does not
//! forbid unsafe Rust, if workspace membership changes without an inventory
//! decision, or if unsafe syntax appears anywhere but the single audited
//! boundary crate.
//!
//! A textual contract cannot prove that the attribute actually rejects unsafe
//! code. `make unsafe-boundary` compiles deliberate unsafe probes on the pinned
//! toolchain for that half, including the control that `forbid` overrules an
//! `#[expect(unsafe_code)]`. See `docs/rust-boundary-verification.md` for why
//! the two halves are separate and what neither of them establishes.

#![forbid(unsafe_code)]

#[path = "common/inner_attributes.rs"]
mod inner_attributes;
#[path = "common/repo_access.rs"]
mod repo_access;

use camino::{Utf8Path, Utf8PathBuf};
use inner_attributes::header_inner_attributes;
use repo_access::{read_repo_file, repo_root};
use serde_json::Value;

/// The source-level prohibition every safe target root must carry.
const PROHIBITION: &str = "#![forbid(unsafe_code)]";

/// The package permitted to contain unsafe Rust.
///
/// Adding a name here is an architectural decision, not a fix for a failing
/// gate: the crate must own a genuine unsafe obligation that cannot be written
/// safely, and `docs/rust-boundary-verification.md` must record it.
const AUDITED_PACKAGES: &[&str] = &["wireframe-bench-alloc"];

/// The exact workspace membership this repository has reviewed.
const EXPECTED_MEMBERS: &[&str] = &[
    "wireframe",
    "wireframe-bench-alloc",
    "wireframe-bench-counting",
    "wireframe-loom",
    "wireframe-verification",
    "wireframe_testing",
];

/// Constructs that only a boundary crate may contain.
///
/// The complete tokens are written out because this file is exempt from its own
/// scan; see [`CONTRACT_TEST_PATH`].
const UNSAFE_CONSTRUCTS: &[&str] = &[
    "unsafe impl",
    "unsafe fn",
    "unsafe trait",
    "unsafe extern",
    "unsafe {",
];

/// Path of this contract test, which is exempt from its own text scan.
///
/// The test necessarily names what it looks for, so scanning it would report
/// the scanner rather than a violation. Its own compilation still carries
/// `#![forbid(unsafe_code)]`, which is the check that matters for its source.
const CONTRACT_TEST_PATH: &str = "tests/workspace_unsafe_boundary.rs";

/// A single Cargo target root discovered from `cargo metadata`.
#[derive(Debug)]
struct TargetRoot {
    /// Absolute path to the target's source file.
    src_path: Utf8PathBuf,
    /// Cargo target kinds, for example `lib` or `test,bench`.
    kind: String,
    /// Name of the package that owns the target.
    package: String,
}

/// Result type shared by these contract checks.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Run `cargo metadata` and parse its JSON output.
fn cargo_metadata(args: &[&str]) -> TestResult<Value> {
    let output = std::process::Command::new("cargo")
        .args(args)
        .current_dir(repo_root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`cargo {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Extract the target roots a `cargo metadata` document describes.
fn metadata_targets(metadata: &Value) -> TestResult<Vec<TargetRoot>> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata should expose `packages` as an array")?;
    let mut roots = Vec::new();
    for package in packages {
        let package_name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("a metadata package should have a string name")?;
        let targets = package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or("a metadata package should expose `targets` as an array")?;
        for target in targets {
            let src_path = target
                .get("src_path")
                .and_then(Value::as_str)
                .ok_or("a metadata target should have a string `src_path`")?;
            let kinds = target
                .get("kind")
                .and_then(Value::as_array)
                .ok_or("a metadata target should expose `kind` as an array")?;
            let kind = kinds
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            roots.push(TargetRoot {
                src_path: Utf8PathBuf::from(src_path),
                kind,
                package: package_name.to_owned(),
            });
        }
    }
    Ok(roots)
}

/// Discover every target root in the workspace and in the standalone gate package.
///
/// `tests/feature_gate` declares an empty `[workspace]` so it cannot inherit the
/// shared lint tables, which also means the repository's own `cargo metadata` does
/// not describe it. It is compiled by `scripts/check-feature-gate.sh`, so it has
/// to be inspected explicitly rather than assumed absent.
fn discover_target_roots() -> TestResult<Vec<TargetRoot>> {
    let mut roots = metadata_targets(&cargo_metadata(&[
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
    ])?)?;
    roots.extend(metadata_targets(&cargo_metadata(&[
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        "tests/feature_gate/Cargo.toml",
    ])?)?);
    Ok(roots)
}

/// Path of a target root relative to the repository.
fn relative_path<'a>(root: &'a TargetRoot, repo: &'a Utf8Path) -> &'a Utf8Path {
    root.src_path.strip_prefix(repo).unwrap_or(&root.src_path)
}

/// Whether a root is this contract test.
fn is_contract_test(root: &TargetRoot, repo: &Utf8Path) -> bool {
    relative_path(root, repo).as_str() == CONTRACT_TEST_PATH
}

/// Describe a target root for failure messages.
fn describe(root: &TargetRoot, repo: &Utf8Path) -> String {
    format!(
        "{} ({} of {})",
        relative_path(root, repo),
        root.kind,
        root.package
    )
}

/// Read a target root's source through the repository capability directory.
fn read_target_source(root: &TargetRoot, repo: &Utf8Path) -> TestResult<String> {
    let relative = root
        .src_path
        .strip_prefix(repo)
        .map_err(|_| format!("target root {} is outside the repository", root.src_path))?;
    read_repo_file(relative)
}

/// Detect an `unsafe_code` expectation in either attribute form.
///
/// `rustfmt` moves a long `#[expect(..., reason = ...)]` onto several lines, so
/// the argument can appear on a line of its own rather than beside the
/// attribute name.
fn mentions_unsafe_expectation(source: &str) -> bool {
    source.contains("expect(unsafe_code")
        || source.lines().any(|line| line.trim() == "unsafe_code,")
}

/// Assert the workspace lint table declares the `unsafe_code` floor.
#[test]
fn workspace_manifest_declares_the_unsafe_code_floor() -> TestResult {
    let manifest = read_repo_file("Cargo.toml")?;
    let mut in_table = false;
    let mut floor = None;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_table = line == "[workspace.lints.rust]";
            continue;
        }
        if in_table && let Some(value) = line.strip_prefix("unsafe_code") {
            floor = Some(value.trim().to_owned());
        }
    }
    match floor.as_deref() {
        Some("= \"deny\"") => Ok(()),
        other => Err(format!(
            "[workspace.lints.rust] should set `unsafe_code = \"deny\"` as the floor every target \
             root raises to `forbid`; found {other:?}"
        )
        .into()),
    }
}

/// Assert every target root outside the audited package forbids unsafe Rust.
#[test]
fn every_safe_target_root_forbids_unsafe_code() -> TestResult {
    let repo = repo_root()?;
    let mut violations = Vec::new();
    for root in discover_target_roots()? {
        if AUDITED_PACKAGES.contains(&root.package.as_str()) {
            continue;
        }
        let source = read_target_source(&root, &repo)?;
        if !header_inner_attributes(&source)
            .iter()
            .any(|a| a == PROHIBITION)
        {
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

/// Violations found in one target root's source.
///
/// An `unsafe_code` allowance is a violation in any root. Outside the audited
/// package, a scoped expectation and an unsafe construct are violations too.
fn root_violations(root: &TargetRoot, repo: &Utf8Path, source: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let has_allowance =
        source.contains("#![allow(unsafe_code") || source.contains("#[allow(unsafe_code");
    if has_allowance {
        violations.push(format!(
            "{} uses an `unsafe_code` allowance; the workspace permits only a documented boundary \
             crate",
            describe(root, repo)
        ));
    }
    if AUDITED_PACKAGES.contains(&root.package.as_str()) {
        return violations;
    }
    if mentions_unsafe_expectation(source) {
        violations.push(format!(
            "{} carries an `unsafe_code` expectation outside an audited package",
            describe(root, repo)
        ));
    }
    violations.extend(
        UNSAFE_CONSTRUCTS
            .iter()
            .filter(|construct| source.contains(**construct))
            .map(|construct| construct_message(root, repo, construct)),
    );
    violations
}

/// Describe an unsafe construct found in a safe package.
fn construct_message(root: &TargetRoot, repo: &Utf8Path, construct: &str) -> String {
    format!(
        "{} contains `{construct}` but belongs to the safe package `{}`",
        describe(root, repo),
        root.package
    )
}

/// Assert the audited package is exactly the reviewed inventory, and only it
/// contains unsafe syntax.
#[test]
fn unsafe_syntax_is_confined_to_the_audited_package() -> TestResult {
    let repo = repo_root()?;
    let mut violations = Vec::new();
    for root in discover_target_roots()? {
        if is_contract_test(&root, &repo) {
            continue;
        }
        let source = read_target_source(&root, &repo)?;
        violations.extend(root_violations(&root, &repo, &source));
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

/// Assert the audited crate holds exactly one `GlobalAlloc` implementation.
///
/// Duplicated implementations are what made the original census hard to read:
/// the same four delegating bodies existed twice, in two different files, and
/// only one of them was reachable when the other failed to compile.
#[test]
fn the_audited_package_defines_the_allocator_once() -> TestResult {
    let repo = repo_root()?;
    let mut definitions = Vec::new();
    for root in discover_target_roots()? {
        if is_contract_test(&root, &repo) {
            continue;
        }
        let source = read_target_source(&root, &repo)?;
        if source.contains("impl GlobalAlloc for") {
            definitions.push(describe(&root, &repo));
        }
    }
    if definitions.len() == 1 && !definitions.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the workspace should define the counting global allocator exactly once, in an audited \
         boundary crate; found {definitions:?}"
    )
    .into())
}

/// Assert workspace membership has not grown without review.
#[test]
fn workspace_membership_matches_the_reviewed_inventory() -> TestResult {
    let metadata = cargo_metadata(&["metadata", "--no-deps", "--format-version", "1"])?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata should expose `packages` as an array")?;
    let mut actual = packages
        .iter()
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    actual.sort_unstable();

    let mut expected = EXPECTED_MEMBERS.to_vec();
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
