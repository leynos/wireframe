//! Shared helpers for workspace-manifest integration and behavioural tests.

use std::{env, process::Command};

use camino::Utf8PathBuf;
use cap_std::{ambient_authority, fs_utf8::Dir};

pub(crate) type WorkspaceManifestResult<T = ()> =
    Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) fn repo_root() -> WorkspaceManifestResult<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(env::current_dir()?).map_err(|path| {
        format!(
            "repository root path is not valid UTF-8: {}",
            path.display()
        )
        .into()
    })
}

pub(crate) fn repo_dir() -> WorkspaceManifestResult<Dir> {
    Ok(Dir::open_ambient_dir(repo_root()?, ambient_authority())?)
}

pub(crate) fn root_manifest() -> WorkspaceManifestResult<String> {
    Ok(repo_dir()?.read_to_string("Cargo.toml")?)
}

pub(crate) fn run_cargo(args: &[&str]) -> WorkspaceManifestResult<String> {
    let subcommand = args.first().copied().unwrap_or("cargo");
    let output = Command::new("cargo")
        .args(args)
        .current_dir(repo_root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`cargo {subcommand}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub(crate) fn cargo_metadata() -> WorkspaceManifestResult<String> {
    run_cargo(&["metadata", "--no-deps", "--format-version", "1"])
}

pub(crate) fn root_package_id() -> WorkspaceManifestResult<String> {
    run_cargo(&["pkgid", "--", "wireframe"]).map(|stdout| stdout.trim().to_owned())
}
