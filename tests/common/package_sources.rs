//! Read every Rust source file that belongs to a Cargo package.
//!
//! Target-root discovery answers what the compiler builds from the roots
//! `cargo metadata` reports. Some committed source is not in that set: a child
//! module behind a disabled `#[cfg(feature = ...)]` is never compiled until the
//! feature is enabled, yet it is part of the package and will compile then.
//! Reading every `.rs` file under the package directory checks the text that
//! exists rather than the text one feature set happens to compile.
//!
//! Only directories that cannot hold package source are skipped: the Cargo
//! build tree, version-control metadata, and the tool caches the repository's
//! own gates create. A directory named `target` counts as the build tree only
//! when it does not lie under a `src` directory, because a module may
//! legitimately have that name. Every other directory is walked, including a
//! dot-prefixed one, because a module can be reached through
//! `#[path = ".internal/x.rs"]`. The repository root is itself the `wireframe`
//! package's directory, so the exclusions also keep the walk out of the build
//! tree and the caches.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::fs_utf8::Dir;
use serde_json::Value;

use super::repo_access::{RepoAccessResult, repo_dir, repo_root};

/// A source file's repository-relative path paired with its contents.
pub(crate) type SourceFile = (Utf8PathBuf, String);

/// Read every `.rs` file in the named Cargo package's directory.
///
/// The directory comes from `cargo metadata` rather than a hard-coded path, so
/// moving the crate cannot silently move it out of the scan.
pub(crate) fn sources_in_package(package: &str) -> RepoAccessResult<Vec<SourceFile>> {
    sources_under(package_directory(package)?.as_path())
}

/// Read every `.rs` file below `crate_root`, relative to the repository root.
///
/// An empty `crate_root` names the repository root itself.
pub(crate) fn sources_under(crate_root: &Utf8Path) -> RepoAccessResult<Vec<SourceFile>> {
    let dir = if crate_root.as_str().is_empty() {
        repo_dir()?
    } else {
        repo_dir()?.open_dir(crate_root)?
    };
    let mut sources = Vec::new();
    read_rust_sources(&dir, crate_root, &mut sources)?;
    Ok(sources)
}

/// Resolve the directory of a workspace package through `cargo metadata`.
///
/// # Errors
///
/// Returns an error when `cargo metadata` fails, does not describe the package,
/// or places it outside the repository.
pub(crate) fn package_directory(package: &str) -> RepoAccessResult<Utf8PathBuf> {
    let manifest = package_manifest(package)?;
    let manifest_dir = manifest
        .parent()
        .ok_or_else(|| format!("manifest path {manifest} has no parent directory"))?;
    let relative = manifest_dir.strip_prefix(repo_root()?).map_err(|_| {
        format!("the manifest for `{package}` ({manifest}) is outside the repository")
    })?;
    Ok(relative.to_path_buf())
}

/// The names of the packages the workspace metadata describes.
pub(crate) fn workspace_package_names() -> RepoAccessResult<Vec<String>> {
    Ok(workspace_package_manifests()?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// The name and repository-relative manifest path of every workspace package.
///
/// The path is relative so callers can read the manifest through the same
/// capability directory as the rest of the repository, and so failure messages
/// name a file the reader can open.
pub(crate) fn workspace_package_manifests() -> RepoAccessResult<Vec<(String, Utf8PathBuf)>> {
    let metadata = workspace_metadata()?;
    let repo = repo_root()?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata should expose `packages` as an array")?;
    packages
        .iter()
        .map(|package| {
            let name = package
                .get("name")
                .and_then(Value::as_str)
                .ok_or("a metadata package should have a string name")?;
            let manifest = package
                .get("manifest_path")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("`{name}` should expose a string `manifest_path`"))?;
            let relative = Utf8PathBuf::from(manifest)
                .strip_prefix(&repo)
                .map_err(|_| {
                    format!("the manifest for `{name}` ({manifest}) is outside the repository")
                })?
                .to_path_buf();
            Ok((name.to_owned(), relative))
        })
        .collect()
}

/// Run `cargo metadata` for the workspace and parse its JSON output.
fn workspace_metadata() -> RepoAccessResult<Value> {
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
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Resolve the manifest path of a workspace package through `cargo metadata`.
fn package_manifest(package: &str) -> RepoAccessResult<Utf8PathBuf> {
    let metadata = workspace_metadata()?;
    let manifest = metadata
        .get("packages")
        .and_then(Value::as_array)
        .and_then(|packages| {
            packages
                .iter()
                .find(|candidate| candidate.get("name").and_then(Value::as_str) == Some(package))
        })
        .and_then(|candidate| candidate.get("manifest_path"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata does not describe `{package}`"))?;
    Ok(Utf8PathBuf::from(manifest))
}

/// Directories that never hold package source.
///
/// `.git` is version-control metadata, and the rest are caches the repository's
/// own gates create. A dot-prefixed directory that is not listed here is
/// walked, so a module behind `#[path = ".internal/optional.rs"]` is still
/// read.
const NON_SOURCE_DIRECTORIES: &[&str] = &[
    ".git",
    ".uv-cache",
    ".uv-tools",
    ".hypothesis",
    ".pytest_cache",
];

/// The directory name Cargo uses for its build tree.
const CARGO_BUILD_DIRECTORY: &str = "target";

/// Whether a directory entry lies outside the package's own source.
///
/// A directory named `target` is the Cargo build tree unless it lies under a
/// `src` directory, where a module may legitimately have that name. The
/// repository root is itself the `wireframe` package's directory, so this also
/// keeps the walk out of the root build tree.
fn should_skip(name: &str, path: &Utf8Path) -> bool {
    if name == CARGO_BUILD_DIRECTORY {
        return !path
            .components()
            .any(|component| component.as_str() == "src");
    }
    NON_SOURCE_DIRECTORIES.contains(&name)
}

/// Append every `.rs` file below `dir` to `sources`, with its repository path.
fn read_rust_sources(
    dir: &Dir,
    prefix: &Utf8Path,
    sources: &mut Vec<SourceFile>,
) -> RepoAccessResult {
    for dir_entry in dir.entries()? {
        let entry = dir_entry?;
        let name = entry.file_name()?;
        let path = prefix.join(&name);
        if should_skip(&name, &path) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            read_rust_sources(&entry.open_dir()?, &path, sources)?;
        } else if Utf8Path::new(&name).extension() == Some("rs") {
            sources.push((path, dir.read_to_string(&name)?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod package_source_tests {
    //! Regressions for the walk's exclusions.
    //!
    //! The walk must not descend into `target/`, because the repository root is
    //! the `wireframe` package's directory and its build tree holds thousands of
    //! generated `.rs` files. It must descend into a dot-prefixed directory that
    //! is not a known cache, because a module can live there, and into a
    //! `target` directory under `src`, because a module may have that name. The
    //! fixture test runs the walk and the unsafe-syntax check together, so the
    //! exclusions are proven on real directory entries rather than only on the
    //! predicate.

    use std::{
        env,
        io,
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use camino::{Utf8Path, Utf8PathBuf};
    use cap_std::{ambient_authority, fs_utf8::Dir};
    use rstest::rstest;

    use super::{RepoAccessResult, SourceFile, read_rust_sources, should_skip};
    use crate::unsafe_syntax::violations_in;

    /// Distinguishes fixture directories created by concurrent test runs.
    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    /// A uniquely named directory under the system temporary directory.
    struct Fixture {
        parent: Dir,
        name: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> io::Result<Self> {
            let temporary = Utf8PathBuf::from_path_buf(env::temp_dir()).map_err(|path| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("temporary directory path is not UTF-8: {}", path.display()),
                )
            })?;
            let parent = Dir::open_ambient_dir(&temporary, ambient_authority())?;
            let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = Utf8PathBuf::from(format!(
                "wireframe-package-sources-{}-{sequence}",
                process::id()
            ));
            parent.create_dir(&name)?;
            Ok(Self { parent, name })
        }

        fn open(&self) -> io::Result<Dir> { self.parent.open_dir(&self.name) }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // A failed cleanup must not turn a passing assertion into a panic.
            drop(self.parent.remove_dir_all(&self.name));
        }
    }

    /// A source text that the unsafe-syntax check must report.
    ///
    /// The keyword and the construct are assembled rather than written as one
    /// literal so this helper's own source does not read as prohibited to the
    /// scan it exercises; the fixture file still contains the exact spelling.
    fn unsafe_function(name: &str) -> String { ["unsafe", " fn ", name, "() {}"].concat() }

    /// Build the fixture tree: source, a hidden module, a module named `target`,
    /// and three directories the walk must skip.
    fn build_tree(dir: &Dir) -> io::Result<()> {
        for directory in [
            "src/.internal",
            "src/target",
            "target",
            ".git",
            ".uv-cache",
            "nested/deeper",
        ] {
            dir.create_dir_all(directory)?;
        }
        dir.write("src/lib.rs", "fn safe() {}\n")?;
        dir.write("src/.internal/optional.rs", unsafe_function("hidden"))?;
        dir.write("src/target/mod.rs", "mod target;\n")?;
        dir.write("target/generated.rs", unsafe_function("generated"))?;
        dir.write(".git/config.rs", "fn version_control() {}\n")?;
        dir.write(".uv-cache/cached.rs", "fn cached() {}\n")?;
        dir.write("nested/deeper/module.rs", "fn nested() {}\n")
    }

    #[test]
    fn walks_source_but_skips_build_and_cache_directories() -> RepoAccessResult<()> {
        let fixture = Fixture::new()?;
        let dir = fixture.open()?;
        build_tree(&dir)?;

        let mut sources: Vec<SourceFile> = Vec::new();
        read_rust_sources(&dir, Utf8Path::new(""), &mut sources)?;

        let mut paths: Vec<String> = sources.iter().map(|(path, _)| path.to_string()).collect();
        paths.sort();
        let expected = [
            "nested/deeper/module.rs",
            "src/.internal/optional.rs",
            "src/lib.rs",
            "src/target/mod.rs",
        ];
        if paths != expected {
            return Err(format!(
                "the walk must visit nested and hidden source and skip build and caches; expected \
                 {expected:?}, got {paths:?}"
            )
            .into());
        }

        let violations: Vec<String> = sources
            .iter()
            .flat_map(|(path, source)| violations_in(source, path.as_str()))
            .collect();
        let hidden_module_reported = violations.iter().any(|violation| {
            violation.contains("src/.internal/optional.rs contains an unsafe function")
        });
        if !hidden_module_reported {
            return Err(format!(
                "an unsafe function in a hidden module must be reported: {violations:?}"
            )
            .into());
        }
        let skipped_directories_are_clean = violations.iter().all(|violation| {
            !violation.contains("target/")
                && !violation.contains(".uv-cache/")
                && !violation.contains(".git/")
        });
        if !skipped_directories_are_clean {
            return Err(format!(
                "skipped directories must not reach the unsafe-syntax check: {violations:?}"
            )
            .into());
        }
        Ok(())
    }

    #[rstest]
    #[case::build_tree("target", "target", true)]
    #[case::member_build_tree("target", "crates/wireframe-loom/target", true)]
    #[case::standalone_build_tree("target", "tests/feature_gate/target", true)]
    #[case::module_named_target("target", "src/target", false)]
    #[case::nested_module_named_target("target", "src/parser/target", false)]
    #[case::version_control(".git", ".git", true)]
    #[case::tool_cache(".uv-cache", ".uv-cache", true)]
    #[case::hidden_module_directory(".internal", "src/.internal", false)]
    #[case::source_directory("src", "src", false)]
    #[case::source_file("lib.rs", "src/lib.rs", false)]
    fn skips_only_non_source_directories(
        #[case] name: &str,
        #[case] path: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(should_skip(name, Utf8Path::new(path)), expected);
    }
}
