//! Shared repository access helpers for integration and behavioural tests.

use std::env;

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::{ambient_authority, fs_utf8::Dir};

/// Result type for repository-access test helpers.
///
/// The boxed error keeps integration-test helpers lightweight while preserving
/// `Send` and `Sync` bounds for callers that run under parallel test harnesses.
pub(crate) type RepoAccessResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Resolve the repository root from the current working directory.
///
/// # Errors
///
/// Returns an error when the current directory cannot be read or its path is
/// not valid UTF-8.
pub(crate) fn repo_root() -> RepoAccessResult<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(env::current_dir()?).map_err(|path| {
        format!(
            "repository root path is not valid UTF-8: {}",
            path.display()
        )
        .into()
    })
}

/// Open the repository root as a capabilities-oriented directory.
///
/// # Errors
///
/// Returns an error when the repository root cannot be resolved or opened.
pub(crate) fn repo_dir() -> RepoAccessResult<Dir> {
    Ok(Dir::open_ambient_dir(repo_root()?, ambient_authority())?)
}

/// Read a UTF-8 file relative to the repository root.
///
/// # Errors
///
/// Returns an error when the repository directory cannot be opened, the file
/// cannot be read, or its contents are not valid UTF-8.
pub(crate) fn read_repo_file(path: impl AsRef<Utf8Path>) -> RepoAccessResult<String> {
    Ok(repo_dir()?.read_to_string(path)?)
}
