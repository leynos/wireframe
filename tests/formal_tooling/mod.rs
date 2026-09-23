//! Regression tests for the formal-verification tooling contract.
//!
//! These checks verify that Wireframe declares pinned Kani, Verus, and
//! `rust-prover-tools` metadata and exposes concise Makefile targets that
//! delegate to `prover-tools` rather than carrying bespoke installer logic.

use std::{
    env,
    io::Write,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

use camino::Utf8PathBuf;
use cap_std::{
    ambient_authority,
    fs::{OpenOptions, Permissions, PermissionsExt},
    fs_utf8::Dir,
};

#[path = "../common/formal_tooling_support.rs"]
mod formal_tooling_support;

#[path = "../common/repo_access.rs"]
mod repo_access;

use formal_tooling_support::{
    ChecksumsContent,
    FormalToolingResult as TestResult,
    MakefileContent,
    ProverToolsRef,
    is_sha256_hex,
    is_three_part_numeric_version,
    kani_version,
    makefile,
    prover_tools_ref_metadata,
    read_trimmed_repo_file,
    run_make,
    run_make_dry_run,
    verus_checksums,
    verus_linux_archive_name,
    verus_version,
};
use proptest::prelude::*;
use rstest::rstest;

static CARGO_WRAPPER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const CARGO_WRAPPER_SCRIPT: &str = r#"#!/bin/sh
printf 'WRAPPER_RUSTFLAGS=%s\n' "${RUSTFLAGS-}"
for argument in "$@"; do
    printf 'WRAPPER_ARG=%s\n' "$argument"
done
"#;

struct TemporaryCargoWrapper {
    directory: Dir,
    filename: Utf8PathBuf,
    path: Utf8PathBuf,
}

impl TemporaryCargoWrapper {
    fn new() -> std::io::Result<Self> {
        let temporary_directory = Utf8PathBuf::from_path_buf(env::temp_dir()).map_err(|path| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("temporary directory path is not UTF-8: {}", path.display()),
            )
        })?;
        let directory = Dir::open_ambient_dir(&temporary_directory, ambient_authority())?;
        let sequence = CARGO_WRAPPER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let filename = Utf8PathBuf::from(format!(
            "wireframe-formal-cargo-wrapper-{}-{sequence}",
            process::id()
        ));
        let path = temporary_directory.join(&filename);
        let wrapper = Self {
            directory,
            filename,
            path,
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut wrapper_file = wrapper.directory.open_with(&wrapper.filename, &options)?;

        wrapper_file.write_all(CARGO_WRAPPER_SCRIPT.as_bytes())?;
        wrapper
            .directory
            .set_permissions(&wrapper.filename, Permissions::from_mode(0o700))?;

        Ok(wrapper)
    }
}

impl Drop for TemporaryCargoWrapper {
    fn drop(&mut self) { let _ = self.directory.remove_file(&self.filename); }
}

fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

#[rstest]
#[case::kani_version("Kani version", formal_tooling_support::KANI_VERSION_PATH)]
#[case::verus_version("Verus version", formal_tooling_support::VERUS_VERSION_PATH)]
#[case::verus_checksums("Verus checksums", formal_tooling_support::VERUS_CHECKSUMS_PATH)]
#[case::prover_tools_ref("rust-prover-tools ref", formal_tooling_support::PROVER_TOOLS_REF_PATH)]
fn formal_tooling_metadata_files_are_present(
    #[case] label: &str,
    #[case] path: &str,
) -> TestResult {
    let contents = read_trimmed_repo_file(path)?;

    ensure(
        !contents.is_empty(),
        format!("{label} metadata should not be empty"),
    )
}

#[rstest]
fn kani_version_pin_uses_three_part_numeric_version() -> TestResult {
    let version = kani_version()?;

    ensure(
        is_three_part_numeric_version(&version),
        format!("Kani version should use MAJOR.MINOR.PATCH, got `{version}`"),
    )
}

#[rstest]
fn verus_checksum_manifest_names_configured_linux_archive() -> TestResult {
    let version = verus_version()?;
    let archive_name = verus_linux_archive_name(&version);
    let checksums_str = verus_checksums()?;
    let checksums = ChecksumsContent(&checksums_str);

    ensure(
        checksums.contains_archive(&archive_name),
        format!("Verus checksums should include a SHA-256 entry for `{archive_name}`"),
    )
}

#[rstest]
fn rust_prover_tools_pin_includes_repository_context_and_ref() -> TestResult {
    let metadata_str = prover_tools_ref_metadata()?;
    let metadata = ProverToolsRef(&metadata_str);

    ensure(
        metadata
            .as_str()
            .contains("repository: https://github.com/leynos/rust-prover-tools.git"),
        "rust-prover-tools metadata should name the upstream repository",
    )?;
    ensure(
        metadata.as_str().contains("branch: main"),
        "rust-prover-tools metadata should name the source branch",
    )?;
    ensure(
        metadata.ref_value().is_some_and(|value| value.len() == 40),
        "rust-prover-tools metadata should expose a 40-character commit ref",
    )?;
    ensure(
        metadata
            .as_str()
            .contains("git ls-remote https://github.com/leynos/rust-prover-tools.git"),
        "rust-prover-tools metadata should include a verification command",
    )
}

#[rstest]
#[case::install_kani("install-kani", "kani install")]
#[case::check_kani_version("check-kani-version", "kani check-version")]
#[case::install_verus("install-verus", "verus install")]
#[case::run_verus("run-verus", "verus run")]
fn makefile_declares_prover_tools_targets(
    #[case] target: &str,
    #[case] prover_subcommand: &str,
) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let recipe = makefile
        .target_recipe(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;

    ensure(
        makefile.has_phony_target(target),
        format!("`{target}` should be declared as a phony Make target"),
    )?;
    ensure(
        recipe.contains("$(PROVER_TOOLS)"),
        format!("`{target}` should delegate through the pinned prover-tools entry point"),
    )?;
    ensure(
        recipe.contains(prover_subcommand),
        format!("`{target}` should call `{prover_subcommand}`"),
    )?;
    if target == "run-verus" {
        ensure(
            recipe.contains("--proof-file \"$(VERUS_PROOF_FILE)\""),
            "`run-verus` should pass the configured proof file",
        )?;
    }
    Ok(())
}

#[rstest]
fn run_verus_target_passes_configured_proof_file() -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let recipe = makefile
        .target_recipe("run-verus")
        .ok_or_else(|| "expected `run-verus` target in Makefile".to_owned())?;

    ensure(
        recipe.contains("--proof-file \"$(VERUS_PROOF_FILE)\""),
        "`run-verus` should pass `--proof-file \"$(VERUS_PROOF_FILE)\"`",
    )
}

#[rstest]
fn workspace_validation_targets_retain_required_cargo_scope() -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let test_recipe = makefile
        .target_recipe("test")
        .ok_or_else(|| "expected `test` target in Makefile".to_owned())?;
    let test_doc_recipe = makefile
        .target_recipe("test-doc")
        .ok_or_else(|| "expected `test-doc` target in Makefile".to_owned())?;
    let typecheck_recipe = makefile
        .target_recipe("typecheck")
        .ok_or_else(|| "expected `typecheck` target in Makefile".to_owned())?;
    let lint_recipe = makefile
        .target_recipe("lint")
        .ok_or_else(|| "expected `lint` target in Makefile".to_owned())?;

    for (target, recipe) in [
        ("test", &test_recipe),
        ("test-doc", &test_doc_recipe),
        ("typecheck", &typecheck_recipe),
    ] {
        ensure(
            recipe.contains("--workspace"),
            format!("`{target}` should validate the supported workspace members"),
        )?;
    }
    ensure(
        test_recipe.contains("--all-targets --all-features"),
        "`test` should retain all-target and all-feature coverage",
    )?;
    ensure(
        test_doc_recipe.contains("--exclude wireframe_testing --doc --all-features"),
        "`test-doc` should retain the tracked wireframe_testing doctest exclusion",
    )?;
    ensure(
        typecheck_recipe.contains("--all-targets --all-features"),
        "`typecheck` should retain all-target and all-feature coverage",
    )?;
    ensure(
        lint_recipe.contains("doc --workspace --no-deps"),
        "`lint` should retain workspace rustdoc coverage",
    )?;
    ensure(
        lint_recipe.contains("clippy $(CLIPPY_FLAGS)"),
        "`lint` should invoke the configured Clippy policy",
    )?;
    ensure(
        makefile_str
            .contains("CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings"),
        "the shared Clippy policy should retain workspace-wide deny-warnings coverage",
    )
}

#[rstest]
#[case::install_kani("install-kani", "prover-tools kani install")]
#[case::check_kani_version("check-kani-version", "prover-tools kani check-version")]
#[case::install_verus("install-verus", "prover-tools verus install")]
#[case::run_verus("run-verus", "prover-tools verus run")]
fn make_targets_dry_run_to_expected_prover_tools_command(
    #[case] target: &str,
    #[case] expected_command: &str,
) -> TestResult {
    let output = run_make_dry_run(target)?;

    ensure(
        output.contains(expected_command),
        format!("`make --dry-run {target}` should emit `{expected_command}`"),
    )
}

mod targets;
