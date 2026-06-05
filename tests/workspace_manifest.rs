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

use repo_access::repo_root;
use rstest::rstest;
use serde_json::Value;
use workspace_manifest_support::{
    WorkspaceManifestResult as TestResult,
    cargo_metadata,
    has_manifest_line,
    root_manifest,
    root_package_id,
    verification_package_id,
};

fn parse_metadata_json(metadata: &str) -> TestResult<Value> { Ok(serde_json::from_str(metadata)?) }

fn contains_json_string_field(json: &str, field: &str, value: &str) -> bool {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    json.contains(&format!("\"{field}\":\"{escaped}\""))
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions provide clearer diagnostics in integration tests"
)]
fn root_manifest_declares_explicit_workspace_section() -> TestResult {
    let manifest = root_manifest()?;
    assert!(
        has_manifest_line(&manifest, "[workspace]"),
        "root Cargo.toml should declare an explicit [workspace] section"
    );
    assert!(
        has_manifest_line(
            &manifest,
            "members = [\".\", \"crates/wireframe-verification\", \"wireframe_testing\"]"
        ),
        "the workspace should explicitly list the root, verification, and testing crates"
    );
    assert!(
        has_manifest_line(&manifest, "default-members = [\".\"]"),
        "15.1.2 should keep the root package as the only default workspace member"
    );
    assert!(
        has_manifest_line(&manifest, "resolver = \"3\""),
        "the hybrid workspace should opt into the edition-2024 resolver"
    );
    Ok(())
}

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

    assert!(
        contains_json_string_field(&metadata, "workspace_root", repo_root_str),
        "workspace_root should be the repository root"
    );
    assert!(
        contains_json_string_field(&metadata, "manifest_path", manifest_path_str),
        "metadata should continue to resolve the root package manifest"
    );
    assert!(
        metadata.contains(&root_package_id),
        "workspace metadata should include the root package"
    );
    let workspace_members = metadata_json
        .get("workspace_members")
        .and_then(Value::as_array)
        .expect("cargo metadata should expose workspace_members as an array");
    assert!(
        workspace_members
            .iter()
            .any(|member| member.as_str() == Some(root_package_id.as_str())),
        "workspace_members should include the root package id"
    );
    assert!(
        workspace_members
            .iter()
            .any(|member| member.as_str() == Some(verification_package_id.as_str())),
        "workspace_members should include the verification crate id"
    );
    assert!(
        workspace_members
            .iter()
            .any(|member| member.as_str() == Some(helper_package_id.as_str())),
        "workspace_members should include the wireframe_testing crate id"
    );
    assert!(
        metadata.contains("wireframe-verification"),
        "15.1.2 should add the verification crate to cargo metadata"
    );
    assert!(
        metadata.contains("wireframe_testing"),
        "workspace metadata should include the test helper crate"
    );
    let workspace_default_members = metadata_json
        .get("workspace_default_members")
        .and_then(Value::as_array)
        .expect("cargo metadata should expose workspace_default_members as an array");
    assert_eq!(
        workspace_default_members.len(),
        1,
        "15.1.2 should keep exactly one default workspace member"
    );
    assert_eq!(
        workspace_default_members.first().and_then(Value::as_str),
        Some(root_package_id.as_str()),
        "15.1.2 should keep the root package as the only default workspace member"
    );
    Ok(())
}
