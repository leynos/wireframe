//! Regression tests for the formal-verification tooling contract.
//!
//! These checks verify that Wireframe declares pinned Kani, Verus, and
//! `rust-prover-tools` metadata and exposes concise Makefile targets that
//! delegate to `prover-tools` rather than carrying bespoke installer logic.

#[path = "common/formal_tooling_support.rs"]
mod formal_tooling_support;

#[path = "common/repo_access.rs"]
mod repo_access;

use std::process::Command;

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
    verus_checksums,
    verus_linux_archive_name,
    verus_version,
};
use proptest::prelude::*;
use repo_access::repo_root;
use rstest::rstest;

fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

fn run_make_dry_run(target: impl AsRef<str>) -> TestResult<String> {
    let target = target.as_ref();
    let output = Command::new("make")
        .args(["--dry-run", target])
        .current_dir(repo_root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`make --dry-run {target}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
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

#[rstest]
#[case::install_kani("install-kani")]
#[case::check_kani_version("check-kani-version")]
#[case::install_verus("install-verus")]
#[case::run_verus("run-verus")]
fn makefile_targets_do_not_embed_bespoke_installer_logic(#[case] target: &str) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let recipe = makefile
        .target_recipe(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;

    for forbidden in [
        "cargo install",
        "cargo kani setup",
        "curl",
        "unzip",
        "sha256sum",
        "shasum",
        "rustup toolchain install",
    ] {
        ensure(
            !recipe.contains(forbidden),
            format!("`{target}` should not contain bespoke installer command `{forbidden}`"),
        )?;
    }
    Ok(())
}

proptest! {
    #[test]
    fn three_part_numeric_versions_accept_exactly_three_numeric_parts(
        major in "[0-9]+",
        minor in "[0-9]+",
        patch in "[0-9]+",
    ) {
        let version = format!("{major}.{minor}.{patch}");

        prop_assert!(is_three_part_numeric_version(version));
    }

    #[test]
    fn three_part_numeric_versions_reject_non_matching_strings(
        candidate in "\\PC*",
    ) {
        let expected = candidate
            .split('.')
            .collect::<Vec<_>>()
            .as_slice()
            .iter()
            .copied()
            .all(|part| !part.is_empty() && part.chars().all(|character| character.is_ascii_digit()))
            && candidate.split('.').count() == 3;

        prop_assert_eq!(is_three_part_numeric_version(&candidate), expected);
    }

    #[test]
    fn sha256_hex_accepts_sixty_four_ascii_hex_digits(
        digest in "[0-9A-Fa-f]{64}",
    ) {
        prop_assert!(is_sha256_hex(&digest));
    }

    #[test]
    fn sha256_hex_rejects_non_matching_strings(
        candidate in "\\PC*",
    ) {
        let expected = candidate.len() == 64
            && candidate.chars().all(|character| character.is_ascii_hexdigit());

        prop_assert_eq!(is_sha256_hex(&candidate), expected);
    }
}
