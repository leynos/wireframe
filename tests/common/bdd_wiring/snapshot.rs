//! Repository-relative snapshots of the BDD test sources.
//!
//! Collecting files is separated from evaluating the naming contract so the
//! same pure rules can run against the live repository and against compact
//! synthetic layouts in tests.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt::{self, Display, Formatter},
};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use cap_std::fs_utf8::Dir;

use crate::repo_access::{read_repo_file, repo_dir};

/// Directories whose `.rs` and `.feature` files form a BDD snapshot.
pub(crate) const SNAPSHOT_DIRS: [&str; 5] = [
    "tests/features",
    "tests/scenarios",
    "tests/fixtures",
    "tests/bdd",
    "tests/bdd_pool",
];

/// Shared result type for snapshot helpers.
pub(crate) type SnapshotResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

/// A violation of the documented BDD naming or registration contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// Repository-relative path of the offending file.
    pub(crate) path: Utf8PathBuf,
    /// Reason naming the expected file, declaration, or binding.
    pub(crate) reason: String,
}

impl Violation {
    /// Create a violation for `path`.
    #[must_use]
    pub(crate) fn new(path: impl Into<Utf8PathBuf>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl Display for Violation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.reason)
    }
}

/// Text contents of every BDD file in a repository layout.
#[derive(Debug, Clone, Default)]
pub(crate) struct RepoSnapshot {
    /// Repository-relative path mapped to file contents, ordered lexically.
    files: BTreeMap<Utf8PathBuf, String>,
}

impl RepoSnapshot {
    /// Collect the live repository's BDD sources.
    ///
    /// # Errors
    ///
    /// Returns an error when a snapshot directory cannot be read or a file is
    /// not valid UTF-8.
    pub(crate) fn from_repository() -> SnapshotResult<Self> {
        let repo = repo_dir()?;
        let mut files = BTreeMap::new();
        for directory in SNAPSHOT_DIRS {
            collect_directory(&repo, Utf8Path::new(directory), &mut files)?;
        }
        Ok(Self { files })
    }

    /// Build a snapshot from explicit repository-relative files.
    #[must_use]
    pub(crate) fn from_files<I>(files: I) -> Self
    where
        I: IntoIterator<Item = (Utf8PathBuf, String)>,
    {
        Self {
            files: files.into_iter().collect(),
        }
    }

    /// Return the text of a repository-relative file.
    pub(crate) fn get(&self, path: impl AsRef<Utf8Path>) -> Option<&str> {
        self.files.get(path.as_ref()).map(String::as_str)
    }

    /// Report whether a repository-relative file exists.
    pub(crate) fn contains(&self, path: impl AsRef<Utf8Path>) -> bool {
        self.files.contains_key(path.as_ref())
    }

    /// Iterate over every file path in lexical order.
    pub(crate) fn paths(&self) -> impl Iterator<Item = &Utf8PathBuf> { self.files.keys() }
}

/// Builder for synthetic snapshots used by wiring-check tests.
#[derive(Debug, Clone, Default)]
pub(crate) struct SnapshotBuilder {
    /// Repository-relative path mapped to file contents.
    files: BTreeMap<Utf8PathBuf, String>,
}

impl SnapshotBuilder {
    /// Start an empty snapshot.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            files: BTreeMap::new(),
        }
    }

    /// Add or replace one repository-relative file.
    #[must_use]
    pub(crate) fn with_file(
        mut self,
        path: impl Into<Utf8PathBuf>,
        contents: impl Into<String>,
    ) -> Self {
        self.files.insert(path.into(), contents.into());
        self
    }

    /// Remove one repository-relative file, if present.
    #[must_use]
    pub(crate) fn without_file(mut self, path: impl AsRef<Utf8Path>) -> Self {
        let _removed = self.files.remove(path.as_ref());
        self
    }

    /// Finish the snapshot.
    pub(crate) fn build(self) -> RepoSnapshot { RepoSnapshot::from_files(self.files) }
}

/// Recursively collect snapshot files from one repository directory.
fn collect_directory(
    repo: &Dir,
    directory: &Utf8Path,
    files: &mut BTreeMap<Utf8PathBuf, String>,
) -> SnapshotResult {
    let entries = repo
        .read_dir(directory)
        .map_err(|error| read_error(directory, &error))?;
    for entry in entries {
        let dir_entry = entry.map_err(|error| read_error(directory, &error))?;
        let name = dir_entry
            .file_name()
            .map_err(|error| read_error(directory, &error))?;
        let path = directory.join(name);
        let file_type = dir_entry
            .file_type()
            .map_err(|error| read_error(&path, &error))?;
        if file_type.is_dir() {
            collect_directory(repo, &path, files)?;
        } else if is_snapshot_file(&path) {
            let contents = read_repo_file(&path)?;
            files.insert(path, contents);
        }
    }
    Ok(())
}

/// Report whether a path is a Rust source or Gherkin feature file.
fn is_snapshot_file(path: &Utf8Path) -> bool { matches!(path.extension(), Some("rs" | "feature")) }

/// Build a contextual error for a failed read.
fn read_error(path: &Utf8Path, error: &dyn Display) -> Box<dyn Error + Send + Sync> {
    format!("failed to read `{path}`: {error}").into()
}

/// Normalize `.` and `..` components in a repository-relative path.
///
/// Snapshot paths are always repository-relative, so root and prefix
/// components do not occur and are ignored.
pub(crate) fn normalize_path(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::Normal(part) => normalized.push(part),
            Utf8Component::ParentDir => {
                let _removed = normalized.pop();
            }
            Utf8Component::CurDir | Utf8Component::RootDir | Utf8Component::Prefix(_) => {}
        }
    }
    normalized
}
