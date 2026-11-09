//! Regressions for discovery and for the paths reported in failure messages.
//!
//! Discovery is exercised through a substituted process boundary, so the
//! test pins the two metadata queries it issues and proves that both
//! documents contribute roots. A root outside the repository keeps its
//! absolute path so the message points somewhere useful instead of dropping
//! a prefix that is not there.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    env,
    io,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::{ambient_authority, fs_utf8::Dir};
use rstest::rstest;
use serde_json::json;

use super::{
    RepoAccessResult,
    TargetRoot,
    WORKSPACE_METADATA,
    describe,
    discover_target_roots_with,
    metadata_targets,
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

/// Every file below `dir`, keyed by relative path, with its bytes.
///
/// Paths alone would miss a rewrite that keeps the file in place, so the
/// snapshot records content as well as names.
fn file_contents(dir: &Dir) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    collect_files(dir, Utf8Path::new(""), &mut files)?;
    Ok(files)
}

/// Collect the files below `dir` into `files`, prefixed by `prefix`.
fn collect_files(
    dir: &Dir,
    prefix: &Utf8Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> io::Result<()> {
    for entry in dir.entries()? {
        let child = entry?;
        let name = child.file_name()?;
        let path = prefix.join(&name);
        if child.file_type()?.is_dir() {
            collect_files(&child.open_dir()?, &path, files)?;
        } else {
            // `dir` is the directory that holds `name`, so the read stays
            // relative to the level the entry came from.
            let contents = dir.read(&name)?;
            files.insert(path.into_string(), contents);
        }
    }
    Ok(())
}

/// A metadata document describing one package with a single library target.
fn metadata(package: &str, src_path: &Utf8Path) -> serde_json::Value {
    json!({
        "packages": [{
            "name": package,
            "targets": [{ "src_path": src_path.as_str(), "kind": ["lib"] }],
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
        Ok(metadata(package, Utf8Path::new("/repo/src/lib.rs")))
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
fn a_failing_workspace_query_fails_discovery() {
    let run = |args: &[&str]| {
        if args.contains(&"--manifest-path") {
            Ok(metadata(
                "wireframe-feature-gate",
                Utf8Path::new("/repo/src/lib.rs"),
            ))
        } else {
            Err("the workspace query failed".into())
        }
    };

    let error = discover_target_roots_with(&run).expect_err("a failing workspace query must fail");

    assert!(
        error.to_string().contains("the workspace query failed"),
        "the failure must name the query that failed: {error}"
    );
}

#[test]
fn a_failing_standalone_query_fails_discovery() {
    let run = |args: &[&str]| {
        if args.contains(&"--manifest-path") {
            Err("the standalone query failed".into())
        } else {
            Ok(metadata("wireframe", Utf8Path::new("/repo/src/lib.rs")))
        }
    };

    let error = discover_target_roots_with(&run).expect_err("a failing standalone query must fail");

    assert!(
        error.to_string().contains("the standalone query failed"),
        "the failure must name the query that failed: {error}"
    );
}

#[rstest]
#[case::missing_packages(json!({}), "`packages`")]
#[case::packages_not_an_array(json!({"packages": {}}), "`packages`")]
#[case::package_without_a_name(json!({"packages": [{"targets": []}]}), "name")]
#[case::package_without_targets(json!({"packages": [{"name": "wireframe"}]}), "`targets`")]
#[case::package_name_not_a_string(
    json!({"packages": [{"name": 7, "targets": []}]}),
    "name"
)]
#[case::package_targets_not_an_array(
    json!({"packages": [{"name": "wireframe", "targets": {}}]}),
    "`targets`"
)]
#[case::target_without_a_source_path(
    json!({"packages": [{"name": "wireframe", "targets": [{"kind": ["lib"]}]}]}),
    "src_path"
)]
#[case::target_without_a_kind(
    json!({"packages": [{"name": "wireframe", "targets": [{"src_path": "/repo/src/lib.rs"}]}]}),
    "kind"
)]
#[case::source_path_not_a_string(
    json!({"packages": [{"name": "wireframe", "targets": [{"src_path": 1, "kind": ["lib"]}]}]}),
    "src_path"
)]
#[case::kind_not_an_array(
    json!({"packages": [{
        "name": "wireframe",
        "targets": [{"src_path": "/repo/src/lib.rs", "kind": "lib"}],
    }]}),
    "`kind`"
)]
#[case::kind_entry_not_a_string(
    json!({"packages": [{
        "name": "wireframe",
        "targets": [{"src_path": "/repo/src/lib.rs", "kind": ["lib", 1]}],
    }]}),
    "`kind`"
)]
fn malformed_metadata_is_rejected(#[case] document: serde_json::Value, #[case] expected: &str) {
    let error = metadata_targets(&document).expect_err("malformed metadata must be rejected");

    assert!(
        error.to_string().contains(expected),
        "the error must name {expected:?}: {error}"
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
    let before = file_contents(&dir)?;
    for required in ["Cargo.toml", "Cargo.lock", "src/lib.rs"] {
        if !before.contains_key(required) {
            return Err(format!(
                "the snapshot should cover {required}, which the query could rewrite; captured \
                 {:?}",
                before.keys().collect::<Vec<_>>()
            )
            .into());
        }
    }

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
    let after = file_contents(&dir)?;
    if after != before {
        return Err(format!(
            "a metadata query must not add, remove or rewrite files in the package it inspects; \
             before {:?}, after {:?}",
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>()
        )
        .into());
    }
    Ok(())
}

/// A rewrite that keeps the path must still change the snapshot.
#[test]
fn a_content_snapshot_detects_a_rewrite() -> RepoAccessResult {
    let scratch = Scratch::new()?;
    let dir = scratch.open()?;
    dir.write("Cargo.toml", PROBE_MANIFEST)?;
    let before = file_contents(&dir)?;

    dir.write("Cargo.toml", PROBE_MANIFEST.replace("0.1.0", "0.2.0"))?;

    let after = file_contents(&dir)?;
    if after == before {
        return Err("rewriting a file in place must change the content snapshot".into());
    }
    Ok(())
}

#[rstest]
#[case::inside_the_repository("/repo/src/lib.rs", "src/lib.rs (lib of wireframe)")]
#[case::outside_the_repository("/elsewhere/src/lib.rs", "/elsewhere/src/lib.rs (lib of wireframe)")]
fn describes_a_root_relative_to_the_repository(#[case] src_path: &str, #[case] expected: &str) {
    let root = TargetRoot {
        src_path: Utf8PathBuf::from(src_path),
        kind: "lib".to_owned(),
        package: "wireframe".to_owned(),
    };
    assert_eq!(describe(&root, Utf8Path::new("/repo")), expected);
}
