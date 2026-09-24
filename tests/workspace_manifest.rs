//! Regression tests for the formal-verification workspace manifest contract.
//!
//! These checks verify that the repository advertises an explicit hybrid
//! workspace, includes the internal verification and testing crates as
//! workspace members, and still keeps the root package as the only default
//! member.

#[path = "common/workspace_manifest_support.rs"]
mod workspace_manifest_support;

#[path = "common/repo_access.rs"]
mod repo_access;

#[path = "common/fallible_assertions/check.rs"]
mod fallible_check;
#[path = "common/fallible_assertions/check_equal.rs"]
mod fallible_check_equal;

use fallible_check::check;
use fallible_check_equal::check_equal;
use repo_access::{read_repo_file, repo_root};
use rstest::rstest;
use serde_json::Value;
use workspace_manifest_support::{
    WorkspaceManifestResult as TestResult,
    cargo_metadata,
    has_manifest_line,
    has_manifest_table,
    helper_package_id,
    root_manifest,
    root_package_id,
    verification_package_id,
};

fn parse_metadata_json(metadata: &str) -> TestResult<Value> { Ok(serde_json::from_str(metadata)?) }

fn contains_json_string_field(json: &str, field: &str, value: &str) -> bool {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    json.contains(&format!("\"{field}\":\"{escaped}\""))
}

fn check_workspace_members(
    members: &[Value],
    root_package_id: &str,
    verification_package_id: &str,
    helper_package_id: &str,
) -> TestResult {
    check(
        members
            .iter()
            .any(|member| member.as_str() == Some(root_package_id)),
        "workspace_members should include the root package id",
    )?;
    check(
        members
            .iter()
            .any(|member| member.as_str() == Some(verification_package_id)),
        "workspace_members should include the verification crate id",
    )?;
    check(
        members
            .iter()
            .any(|member| member.as_str() == Some(helper_package_id)),
        "workspace_members should include the wireframe_testing crate id",
    )?;
    Ok(())
}

#[rstest]
fn root_manifest_declares_explicit_workspace_section() -> TestResult {
    let manifest = root_manifest()?;
    check(
        has_manifest_table(&manifest, "[workspace]"),
        "root Cargo.toml should declare an explicit [workspace] section",
    )?;
    check(
        has_manifest_line(
            &manifest,
            "[workspace]",
            "members = [\".\", \"crates/wireframe-verification\", \"wireframe_testing\"]",
        ),
        "the workspace should explicitly list the root, verification, and testing crates",
    )?;
    check(
        has_manifest_line(&manifest, "[workspace]", "default-members = [\".\"]"),
        "15.1.2 should keep the root package as the only default workspace member",
    )?;
    check(
        has_manifest_line(&manifest, "[workspace]", "resolver = \"3\""),
        "the hybrid workspace should opt into the edition-2024 resolver",
    )?;
    Ok(())
}

#[rstest]
fn companion_crates_inherit_private_documentation_clippy_policy() -> TestResult {
    let root_manifest = root_manifest()?;
    check(
        has_manifest_table(&root_manifest, "[workspace.lints.clippy]"),
        "the workspace must define its shared Clippy policy",
    )?;
    check(
        has_manifest_line(
            &root_manifest,
            "[workspace.lints.clippy]",
            "missing_docs_in_private_items = \"deny\"",
        ),
        "the shared Clippy policy must deny undocumented private implementation items",
    )?;
    check(
        has_manifest_table(&root_manifest, "[lints]"),
        "the root package must opt into workspace lint inheritance",
    )?;
    check(
        has_manifest_line(&root_manifest, "[lints]", "workspace = true"),
        "the root package must inherit the shared Clippy policy",
    )?;

    for (package_name, manifest_path) in [
        ("wireframe_testing", "wireframe_testing/Cargo.toml"),
        (
            "wireframe-verification",
            "crates/wireframe-verification/Cargo.toml",
        ),
    ] {
        let manifest = read_repo_file(manifest_path)?;
        check(
            has_manifest_table(&manifest, "[lints]"),
            format!("{package_name} must opt into workspace lint inheritance"),
        )?;
        check(
            has_manifest_line(&manifest, "[lints]", "workspace = true"),
            format!("{package_name} must inherit the shared Clippy policy"),
        )?;
    }
    Ok(())
}

#[rstest]
fn cargo_metadata_reports_explicit_members_without_widening_default_members() -> TestResult {
    let repo_root = repo_root()?;
    let repo_root_str = repo_root.as_str();
    let root_package_id = root_package_id()?;
    let helper_package_id = helper_package_id()?;
    let verification_package_id = verification_package_id()?;
    let manifest_path = repo_root.join("Cargo.toml");
    let manifest_path_str = manifest_path.as_str();
    let metadata = cargo_metadata()?;
    let metadata_json = parse_metadata_json(&metadata)?;

    check(
        contains_json_string_field(&metadata, "workspace_root", repo_root_str),
        "workspace_root should be the repository root",
    )?;
    check(
        contains_json_string_field(&metadata, "manifest_path", manifest_path_str),
        "metadata should continue to resolve the root package manifest",
    )?;
    check(
        metadata.contains(&root_package_id),
        "workspace metadata should include the root package",
    )?;
    let workspace_members = metadata_json
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "cargo metadata should expose workspace_members as an array, got {:?}",
                metadata_json.get("workspace_members")
            )
        })?;
    check_workspace_members(
        workspace_members,
        &root_package_id,
        &verification_package_id,
        &helper_package_id,
    )?;
    check(
        metadata.contains("wireframe-verification"),
        "15.1.2 should add the verification crate to cargo metadata",
    )?;
    check(
        metadata.contains("wireframe_testing"),
        "workspace metadata should include the test helper crate",
    )?;
    let workspace_default_members = metadata_json
        .get("workspace_default_members")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "cargo metadata should expose workspace_default_members as an array, got {:?}",
                metadata_json.get("workspace_default_members")
            )
        })?;
    check_equal(
        &workspace_default_members.len(),
        &1_usize,
        "15.1.2 should keep exactly one default workspace member",
    )?;
    check_equal(
        &workspace_default_members.first().and_then(Value::as_str),
        &Some(root_package_id.as_str()),
        "15.1.2 should keep the root package as the only default workspace member",
    )?;
    Ok(())
}
