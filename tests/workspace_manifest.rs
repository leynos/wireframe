//! Regression tests for the staged hybrid-workspace manifest contract.
//!
//! These checks verify that the repository advertises an explicit hybrid
//! workspace while keeping the root package as the only default member during
//! roadmap item 10.1.1.

use std::{env, process::Command};

use camino::Utf8PathBuf;
use cap_std::{ambient_authority, fs_utf8::Dir};
use rstest::rstest;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn repo_root() -> TestResult<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(env::current_dir()?).map_err(|path| {
        format!(
            "repository root path is not valid UTF-8: {}",
            path.display()
        )
        .into()
    })
}

fn repo_dir() -> TestResult<Dir> { Ok(Dir::open_ambient_dir(repo_root()?, ambient_authority())?) }

fn root_manifest() -> TestResult<String> { Ok(repo_dir()?.read_to_string("Cargo.toml")?) }

fn cargo_metadata() -> TestResult<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(repo_root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

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
        manifest.contains("[workspace]"),
        "root Cargo.toml should declare an explicit [workspace] section"
    );
    assert!(
        manifest.contains("members = [\".\"]"),
        "10.1.1 should stage the workspace with only the root package as a member"
    );
    assert!(
        manifest.contains("default-members = [\".\"]"),
        "10.1.1 should keep the root package as the only default workspace member"
    );
    assert!(
        manifest.contains("resolver = \"3\""),
        "the hybrid workspace should opt into the edition-2024 resolver"
    );
    Ok(())
}

#[rstest]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions provide clearer diagnostics in integration tests"
)]
fn cargo_metadata_reports_root_as_only_workspace_member_and_default_member() -> TestResult {
    let repo_root = repo_root()?;
    let repo_root_str = repo_root.as_str();
    let package_id = format!("path+file://{repo_root_str}#wireframe@0.3.0");
    let manifest_path = repo_root.join("Cargo.toml");
    let manifest_path_str = manifest_path.as_str();
    let metadata = cargo_metadata()?;

    assert!(
        contains_json_string_field(&metadata, "workspace_root", repo_root_str),
        "workspace_root should be the repository root"
    );
    assert!(
        contains_json_string_field(&metadata, "manifest_path", manifest_path_str),
        "metadata should continue to resolve the root package manifest"
    );
    assert!(
        metadata.contains(&package_id),
        "workspace metadata should include the root package"
    );
    assert!(
        metadata.contains(&format!("\"workspace_default_members\":[\"{package_id}\"]")),
        "10.1.1 should keep the root package as the only default workspace member"
    );
    assert!(
        !metadata.contains("wireframe-verification"),
        "10.1.1 should not add the verification crate before roadmap item 10.1.2"
    );
    assert!(
        metadata.contains("wireframe_testing#0.3.0"),
        "cargo metadata should continue to report the in-repo helper crate in workspace_members"
    );
    Ok(())
}
