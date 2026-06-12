//! Shared helpers for workspace-manifest integration and behavioural tests.

use std::process::Command;

use crate::repo_access::{read_repo_file, repo_root};

pub(crate) type WorkspaceManifestResult<T = ()> =
    Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) fn root_manifest() -> WorkspaceManifestResult<String> { read_repo_file("Cargo.toml") }

pub(crate) fn run_cargo(args: &[&str]) -> WorkspaceManifestResult<String> {
    let command_args = if args.is_empty() {
        "<no-subcommand>".to_owned()
    } else {
        args.join(" ")
    };
    let output = Command::new("cargo")
        .args(args)
        .current_dir(repo_root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`cargo {command_args}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub(crate) fn cargo_metadata() -> WorkspaceManifestResult<String> {
    run_cargo(&["metadata", "--no-deps", "--format-version", "1"])
}

pub(crate) fn cargo_package_id(package_name: &str) -> WorkspaceManifestResult<String> {
    run_cargo(&["pkgid", "--", package_name]).map(|stdout| stdout.trim().to_owned())
}

pub(crate) fn root_package_id() -> WorkspaceManifestResult<String> { cargo_package_id("wireframe") }

pub(crate) fn helper_package_id() -> WorkspaceManifestResult<String> {
    cargo_package_id("wireframe_testing")
}
pub(crate) fn verification_package_id() -> WorkspaceManifestResult<String> {
    cargo_package_id("wireframe-verification")
}

pub(crate) fn has_manifest_line(manifest: &str, expected: &str) -> bool {
    manifest.lines().any(|line| line.trim() == expected)
}
