//! Discover Cargo target roots from `cargo metadata`.
//!
//! A contract about target roots has to inspect every root Cargo compiles, not
//! only the ones a hard-coded list happens to name. `cargo metadata` knows
//! about automatic and explicitly configured targets alike, so its output is
//! the inventory. The standalone `tests/feature_gate` package declares its own
//! `[workspace]`, so a second metadata run with that manifest is needed to see
//! it: the repository's own metadata does not describe it.

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use super::repo_access::{RepoAccessResult, read_repo_file, repo_root};

/// A single Cargo target root discovered from `cargo metadata`.
#[derive(Debug)]
pub(crate) struct TargetRoot {
    /// Absolute path to the target's source file.
    pub(crate) src_path: Utf8PathBuf,
    /// Cargo target kinds, for example `lib` or `test,bench`.
    pub(crate) kind: String,
    /// Name of the package that owns the target.
    pub(crate) package: String,
}

/// The `cargo metadata` invocation that describes the workspace.
///
/// `--locked` makes the query fail rather than rewrite a stale lockfile, so
/// discovery cannot change the working tree it is inspecting.
const WORKSPACE_METADATA: &[&str] = &["metadata", "--no-deps", "--format-version", "1", "--locked"];

/// The invocation that additionally describes the standalone gate package.
const FEATURE_GATE_METADATA: &[&str] = &[
    "metadata",
    "--no-deps",
    "--format-version",
    "1",
    "--locked",
    "--manifest-path",
    "tests/feature_gate/Cargo.toml",
];

/// The process boundary discovery runs through.
///
/// Substituting it lets a test pin the exact queries discovery issues without
/// starting Cargo, and keeps the read-only contract visible: discovery may run
/// metadata queries and nothing else.
type MetadataRunner<'a> = &'a dyn Fn(&[&str]) -> RepoAccessResult<Value>;

/// Discover every target root in the workspace and in the standalone gate package.
///
/// `tests/feature_gate` declares an empty `[workspace]` so it cannot inherit the
/// shared lint tables, which also means the repository's own `cargo metadata`
/// does not describe it. It is compiled by `scripts/check-feature-gate.sh`, so it
/// has to be inspected explicitly rather than assumed absent.
pub(crate) fn discover_target_roots() -> RepoAccessResult<Vec<TargetRoot>> {
    discover_target_roots_with(&cargo_metadata)
}

/// Discover target roots through `run`, which must only query metadata.
pub(crate) fn discover_target_roots_with(
    run: MetadataRunner<'_>,
) -> RepoAccessResult<Vec<TargetRoot>> {
    let mut roots = metadata_targets(&run(WORKSPACE_METADATA)?)?;
    roots.extend(metadata_targets(&run(FEATURE_GATE_METADATA)?)?);
    Ok(roots)
}

/// Describe a target root for failure messages.
pub(crate) fn describe(root: &TargetRoot, repo: &Utf8Path) -> String {
    format!(
        "{} ({} of {})",
        relative_path(root, repo),
        root.kind,
        root.package
    )
}

/// Read a target root's source through the repository capability directory.
pub(crate) fn read_target_source(root: &TargetRoot, repo: &Utf8Path) -> RepoAccessResult<String> {
    let relative = root
        .src_path
        .strip_prefix(repo)
        .map_err(|_| format!("target root {} is outside the repository", root.src_path))?;
    read_repo_file(relative)
}

/// Run `cargo metadata` in the repository and parse its JSON output.
fn cargo_metadata(args: &[&str]) -> RepoAccessResult<Value> {
    run_cargo_metadata(repo_root()?.as_std_path(), args)
}

/// Run `cargo metadata` in `directory` and parse its JSON output.
///
/// The directory is a parameter so a test can point the production runner at an
/// isolated package and prove the query leaves that package untouched.
fn run_cargo_metadata(directory: &std::path::Path, args: &[&str]) -> RepoAccessResult<Value> {
    let output = std::process::Command::new("cargo")
        .args(args)
        .current_dir(directory)
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
fn metadata_targets(metadata: &Value) -> RepoAccessResult<Vec<TargetRoot>> {
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
            roots.push(target_root(target, package_name)?);
        }
    }
    Ok(roots)
}

/// Build a target root from one entry of a package's `targets` array.
fn target_root(target: &Value, package_name: &str) -> RepoAccessResult<TargetRoot> {
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
    Ok(TargetRoot {
        src_path: Utf8PathBuf::from(src_path),
        kind,
        package: package_name.to_owned(),
    })
}

/// Path of a target root relative to the repository.
fn relative_path<'a>(root: &'a TargetRoot, repo: &'a Utf8Path) -> &'a Utf8Path {
    root.src_path.strip_prefix(repo).unwrap_or(&root.src_path)
}

#[cfg(test)]
mod cargo_target_tests {
    //! Regressions for discovery and for the paths reported in failure messages.
    //!
    //! Discovery is exercised through a substituted process boundary, so the
    //! test pins the two metadata queries it issues and proves that both
    //! documents contribute roots. A root outside the repository keeps its
    //! absolute path so the message points somewhere useful instead of dropping
    //! a prefix that is not there.

    use std::{
        cell::RefCell,
        env,
        io,
        process::{self, Command},
        sync::atomic::{AtomicU64, Ordering},
    };

    use camino::{Utf8Path, Utf8PathBuf};
    use cap_std::{ambient_authority, fs_utf8::Dir};
    use serde_json::json;

    use super::{
        RepoAccessResult,
        TargetRoot,
        WORKSPACE_METADATA,
        describe,
        discover_target_roots_with,
        run_cargo_metadata,
    };

    /// Distinguishes scratch directories created by concurrent test runs.
    static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    /// A package with no dependencies, so its lockfile needs no network access.
    const PROBE_MANIFEST: &str = concat!(
        "[package]\nname = \"metadata-probe\"\nversion = \"0.1.0\"\n",
        "edition = \"2021\"\n\n[workspace]\n",
    );

    /// A uniquely named directory under the system temporary directory.
    struct Scratch {
        temporary: Utf8PathBuf,
        parent: Dir,
        name: Utf8PathBuf,
    }

    impl Scratch {
        fn new() -> io::Result<Self> {
            let temporary = Utf8PathBuf::from_path_buf(env::temp_dir()).map_err(|path| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("temporary directory path is not UTF-8: {}", path.display()),
                )
            })?;
            let parent = Dir::open_ambient_dir(&temporary, ambient_authority())?;
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = Utf8PathBuf::from(format!(
                "wireframe-cargo-targets-{}-{sequence}",
                process::id()
            ));
            parent.create_dir(&name)?;
            Ok(Self {
                temporary,
                parent,
                name,
            })
        }

        fn path(&self) -> Utf8PathBuf { self.temporary.join(&self.name) }

        fn open(&self) -> io::Result<Dir> { self.parent.open_dir(&self.name) }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // A failed cleanup must not turn a passing assertion into a panic.
            drop(self.parent.remove_dir_all(&self.name));
        }
    }

    /// Write the probe package's lockfile so the `--locked` query has one.
    fn generate_lockfile(root: &Utf8Path) -> RepoAccessResult {
        let output = Command::new("cargo")
            .arg("generate-lockfile")
            .current_dir(root)
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "`cargo generate-lockfile` failed in {root}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }

    /// Every file path below `dir`, relative to it, in sorted order.
    fn file_paths(dir: &Dir) -> io::Result<Vec<String>> {
        let mut paths = Vec::new();
        collect_paths(dir, "", &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    fn collect_paths(dir: &Dir, prefix: &str, paths: &mut Vec<String>) -> io::Result<()> {
        for entry in dir.entries()? {
            let child = entry?;
            let name = child.file_name()?;
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if child.file_type()?.is_dir() {
                collect_paths(&child.open_dir()?, &path, paths)?;
            } else {
                paths.push(path);
            }
        }
        Ok(())
    }

    fn root(src_path: &str, kind: &str) -> TargetRoot {
        TargetRoot {
            src_path: Utf8PathBuf::from(src_path),
            kind: kind.to_owned(),
            package: "wireframe".to_owned(),
        }
    }

    /// A metadata document with one package and one target.
    fn metadata(package: &str, src_path: &str, kind: &str) -> serde_json::Value {
        json!({
            "packages": [{
                "name": package,
                "targets": [{ "src_path": src_path, "kind": [kind] }],
            }],
        })
    }

    #[test]
    fn discovery_queries_the_workspace_and_the_standalone_package() {
        let calls = RefCell::new(Vec::new());
        let run = |args: &[&str]| {
            calls.borrow_mut().push(args.join(" "));
            let package = if args.contains(&"--manifest-path") {
                "wireframe-feature-gate"
            } else {
                "wireframe"
            };
            Ok(metadata(package, "/repo/src/lib.rs", "lib"))
        };

        let roots = discover_target_roots_with(&run).expect("fixture metadata should parse");

        assert_eq!(
            *calls.borrow(),
            [
                "metadata --no-deps --format-version 1 --locked",
                concat!(
                    "metadata --no-deps --format-version 1 --locked --manifest-path ",
                    "tests/feature_gate/Cargo.toml",
                ),
            ],
            "discovery must issue exactly the two read-only metadata queries"
        );
        assert_eq!(
            roots
                .iter()
                .map(|root| root.package.as_str())
                .collect::<Vec<_>>(),
            ["wireframe", "wireframe-feature-gate"],
            "roots from both metadata documents must be combined"
        );
    }

    #[test]
    fn metadata_queries_leave_an_isolated_package_unchanged() -> RepoAccessResult {
        let scratch = Scratch::new()?;
        let dir = scratch.open()?;
        dir.create_dir("src")?;
        dir.write("Cargo.toml", PROBE_MANIFEST)?;
        dir.write("src/lib.rs", "pub fn probe() {}\n")?;
        let root = scratch.path();
        generate_lockfile(&root)?;
        let before = file_paths(&dir)?;

        let metadata = run_cargo_metadata(root.as_std_path(), WORKSPACE_METADATA)?;

        let packages = metadata
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        if packages != 1 {
            return Err(format!(
                "the probe workspace should describe its one package; found {packages}"
            )
            .into());
        }
        let after = file_paths(&dir)?;
        if after != before {
            return Err(format!(
                "a metadata query must not add, remove or rewrite files in the package it \
                 inspects; before {before:?}, after {after:?}"
            )
            .into());
        }
        Ok(())
    }

    #[test]
    fn strips_the_repository_prefix() {
        let repo = Utf8Path::new("/repo");
        let described = describe(&root("/repo/src/lib.rs", "lib"), repo);
        assert_eq!(described, "src/lib.rs (lib of wireframe)");
    }

    #[test]
    fn keeps_a_path_outside_the_repository() {
        let repo = Utf8Path::new("/repo");
        let described = describe(&root("/elsewhere/src/lib.rs", "lib"), repo);
        assert_eq!(described, "/elsewhere/src/lib.rs (lib of wireframe)");
    }
}
