//! Executable contracts for the documents a new user reads first.
//!
//! The shared loader requires a marker on every fence, and Rust fences run as
//! doctests through `src/tested_documentation.rs`. This suite holds the README,
//! the users' guide, and the current migration guide to more than that: which
//! fences exist, that each language has a validator, that no Rust fence is
//! ignored or hides a line from its reader, that shell and manifest fences match
//! this repository, and that the links between the documents resolve.

#![forbid(unsafe_code)]

#[path = "documentation_examples_tests/links.rs"]
mod links;
#[path = "common/repo_access.rs"]
mod repo_access;
#[path = "documentation_examples_tests/shell_commands.rs"]
mod shell_commands;
#[path = "common/tested_examples.rs"]
mod tested_examples;

use std::collections::BTreeSet;

use repo_access::read_repo_file;
use rstest::rstest;
use tested_examples::{TestedExample, load_documents};

/// The documents a new user reads first, which this suite holds to its contracts.
const DOCUMENTS: &[&str] = &[
    "README.md",
    "docs/users-guide.md",
    "docs/v0-4-0-migration-guide.md",
];

/// The release these documents describe, which is the one this tree builds.
const DOCUMENTED_RELEASE: &str = env!("CARGO_PKG_VERSION");

const CARGO_MANIFEST: &str = include_str!("../Cargo.toml");

/// Every published example, so that adding or removing one is a deliberate act.
const EXPECTED_EXAMPLE_IDS: &[&str] = &[
    "guide-allocator",
    "guide-client-call-streaming",
    "guide-client-connect",
    "guide-client-correlation",
    "guide-client-error-mapping",
    "guide-client-hooks",
    "guide-client-lifecycle",
    "guide-client-pool",
    "guide-client-preamble",
    "guide-client-send-streaming",
    "guide-client-tracing",
    "guide-client-typed-with",
    "guide-connection-actor",
    "guide-const-client",
    "guide-const-config",
    "guide-const-push-builder",
    "guide-custom-envelope",
    "guide-decode-eof",
    "guide-dependencies",
    "guide-first-server",
    "guide-fragmentation-config",
    "guide-fragmenter",
    "guide-frame-codec",
    "guide-lifecycle-hooks",
    "guide-memory-budgets",
    "guide-message-assembler",
    "guide-message-assembly-state",
    "guide-message-series",
    "guide-middleware-from-fn",
    "guide-middleware-transform",
    "guide-middleware-versions",
    "guide-prepared-app",
    "guide-prometheus-dependency",
    "guide-prometheus-recorder",
    "guide-protocol-hooks",
    "guide-push-queues",
    "guide-recovery-hook",
    "guide-request-body",
    "guide-request-journey",
    "guide-response-channel",
    "guide-response-route",
    "guide-response-route-stream",
    "guide-response-stream",
    "guide-routes",
    "guide-run-client-example",
    "guide-send-response",
    "guide-server-configuration",
    "guide-server-preamble",
    "guide-server-push",
    "guide-session-registry",
    "guide-shared-state",
    "guide-testing-dependencies",
    "guide-testing-fixtures",
    "guide-testing-observability",
    "guide-testing-pair",
    "guide-testing-partial-frames",
    "guide-testing-reassembly",
    "guide-testing-slow-io",
    "guide-tracing-subscriber",
    "migration-assembly-error-match",
    "migration-before-send-order",
    "migration-client-error-match",
    "migration-copied-bytes-after",
    "migration-copied-bytes-before",
    "migration-dependencies",
    "migration-extractors-after",
    "migration-extractors-before",
    "migration-fragmentation-config",
    "migration-message-assembler-after",
    "migration-message-assembler-before",
    "migration-observability-dependencies",
    "migration-pool-closed",
    "migration-preamble-timeout",
    "migration-prepared-app",
    "migration-prepared-test-drivers",
    "migration-protocol-error",
    "migration-protocol-hooks-after",
    "migration-protocol-hooks-before",
    "migration-reassembler-new",
    "migration-server-from-app",
    "readme-dependencies",
    "readme-first-server",
    "readme-new-project",
    "readme-run-echo-client",
    "readme-run-echo-server",
    "readme-run-first-server",
];

/// Rustdoc attributes a documented Rust fence may carry.
///
/// `ignore` is absent on purpose: an ignored fence is an untested one.
const ACCEPTED_RUST_INFO: &[&str] = &["rust", "rust,no_run", "rust,compile_fail"];

/// Read one of [`DOCUMENTS`].
fn read_document(path: &str) -> String {
    read_repo_file(path).expect("a documented file should be readable")
}

/// Load every example in [`DOCUMENTS`].
fn load_documented_examples() -> Vec<TestedExample> {
    let contents: Vec<_> = DOCUMENTS
        .iter()
        .map(|path| (*path, read_document(path)))
        .collect();
    let documents: Vec<_> = contents
        .iter()
        .map(|(path, text)| (*path, text.as_str()))
        .collect();
    load_documents(&documents).expect("documents should load")
}

/// Return the info string of an example's fence, including rustdoc attributes.
fn fence_info(example: &TestedExample) -> String {
    read_document(&example.document)
        .lines()
        .nth(example.start_line - 1)
        .expect("an example starts inside its document")
        .trim_start_matches(['`', '~'])
        .trim()
        .to_owned()
}

/// Return the first line rustdoc would compile but hide from its rendering.
///
/// GitHub shows such a line verbatim, so a reader copying the example would not
/// get what was tested.
fn hidden_rustdoc_line(body: &str) -> Option<&str> {
    body.lines().find(|line| {
        let code = line.trim_start();
        code == "#" || code.starts_with("# ")
    })
}

fn parsed_manifest() -> toml::Table {
    CARGO_MANIFEST
        .parse()
        .expect("Cargo.toml should parse as TOML")
}

#[test]
fn every_documented_fence_has_a_known_unique_identifier() {
    let examples = load_documented_examples();
    let actual: BTreeSet<_> = examples.iter().map(|example| example.id.as_str()).collect();
    let expected: BTreeSet<_> = EXPECTED_EXAMPLE_IDS.iter().copied().collect();
    assert_eq!(actual, expected, "documented example registry drifted");
}

#[test]
fn rust_fences_are_doctests_that_hide_nothing() {
    for example in load_documented_examples()
        .iter()
        .filter(|example| example.language == "rust")
    {
        let info = fence_info(example);
        assert!(
            ACCEPTED_RUST_INFO.contains(&info.as_str()),
            "{} in {} uses unsupported fence info '{info}'",
            example.id,
            example.document
        );
        let hidden = hidden_rustdoc_line(&example.source);
        assert!(
            hidden.is_none(),
            "{} in {} hides {hidden:?}; readers copying it would not get what was tested",
            example.id,
            example.document
        );
    }
}

#[rstest]
#[case::hidden_item("# fn main() {}\nlet x = 1;\n", Some("# fn main() {}"))]
#[case::hidden_indented("    # let y = 2;\n", Some("    # let y = 2;"))]
#[case::bare_hash("#\n", Some("#"))]
#[case::attribute("#[derive(Debug)]\nstruct A;\n", None)]
#[case::inner_attribute("#![allow(unused)]\n", None)]
#[case::comment("// # not hidden\nlet x = 1;\n", None)]
fn hidden_lines_are_detected(#[case] body: &str, #[case] expected: Option<&str>) {
    assert_eq!(hidden_rustdoc_line(body), expected);
}

/// Check one `[dependencies]`-style table from a documented manifest.
fn assert_dependencies_match_release(id: &str, dependencies: &toml::Table, features: &toml::Table) {
    let expected = format!("={DOCUMENTED_RELEASE}");
    for crate_name in ["wireframe", "wireframe_testing"] {
        let declared = dependencies.get(crate_name).map(|dependency| {
            dependency
                .as_str()
                .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
        });
        if let Some(requirement) = declared {
            assert_eq!(
                requirement,
                Some(expected.as_str()),
                "{id} pins {crate_name} to the wrong release"
            );
        }
    }
    let requested = dependencies
        .get("wireframe")
        .and_then(|wireframe| wireframe.get("features"))
        .and_then(toml::Value::as_array);
    for name in requested
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
    {
        assert!(
            features.contains_key(name),
            "{id} names unknown feature '{name}'"
        );
    }
}

#[test]
fn dependency_examples_name_the_documented_release_and_real_features() {
    let manifest = parsed_manifest();
    let features = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml should declare features");

    let examples = load_documented_examples();
    for example in examples.iter().filter(|example| example.language == "toml") {
        let parsed: toml::Table = example
            .source
            .parse()
            .expect("example should be valid TOML");
        let sections = ["dependencies", "dev-dependencies"]
            .iter()
            .filter_map(|section| parsed.get(*section).and_then(toml::Value::as_table));
        for dependencies in sections {
            assert_dependencies_match_release(&example.id, dependencies, features);
        }
    }
}

/// Every fence language needs an owner, so a new one cannot go unvalidated.
///
/// `rust` runs through rustdoc, `toml` and `bash` are checked by this suite,
/// `mermaid` by `make nixie`, and `text` is reserved for illustrations such as
/// sample output and code from an earlier release.
#[test]
fn every_fence_language_has_a_validator() {
    for example in load_documented_examples() {
        assert!(
            matches!(
                example.language.as_str(),
                "rust" | "toml" | "bash" | "mermaid" | "text"
            ),
            "{} in {} uses unvalidated language '{}'",
            example.id,
            example.document,
            example.language
        );
    }
}

#[test]
fn the_readme_links_the_badge_and_the_home_page_in_the_manifest() {
    let manifest = parsed_manifest();
    let homepage = manifest
        .get("package")
        .and_then(|package| package.get("homepage"))
        .and_then(toml::Value::as_str)
        .expect("Cargo.toml should declare a homepage");
    assert_eq!(homepage, "https://df12.studio/wireframe");

    let readme = read_document("README.md");
    assert!(readme.contains(&format!("Website: <{homepage}>")));
    assert!(
        readme.contains(&format!("Wireframe v{DOCUMENTED_RELEASE}")),
        "the README should describe the release this tree builds"
    );
    assert!(readme.contains("https://img.shields.io/crates/v/wireframe"));
    assert!(readme.contains("https://crates.io/crates/wireframe"));
}
