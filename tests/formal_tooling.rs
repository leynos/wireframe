//! Regression tests for the formal-verification tooling contract.
//!
//! These checks verify that Wireframe declares pinned Kani, Verus, and
//! `rust-prover-tools` metadata and exposes concise Makefile targets that
//! delegate to `prover-tools` rather than carrying bespoke installer logic.

#[path = "common/formal_tooling_support.rs"]
mod formal_tooling_support;

use formal_tooling_support::{
    FormalToolingResult as TestResult,
    checksums_contain_archive,
    has_phony_target,
    is_three_part_numeric_version,
    kani_version,
    makefile,
    prover_tools_ref_metadata,
    prover_tools_ref_value,
    read_trimmed_repo_file,
    target_recipe,
    verus_checksums,
    verus_linux_archive_name,
    verus_version,
};
use rstest::rstest;

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
    let checksums = verus_checksums()?;

    ensure(
        checksums_contain_archive(&checksums, &archive_name),
        format!("Verus checksums should include a SHA-256 entry for `{archive_name}`"),
    )
}

#[rstest]
fn rust_prover_tools_pin_includes_repository_context_and_ref() -> TestResult {
    let metadata = prover_tools_ref_metadata()?;
    let ref_value = prover_tools_ref_value(&metadata);

    ensure(
        metadata.contains("repository: https://github.com/leynos/rust-prover-tools.git"),
        "rust-prover-tools metadata should name the upstream repository",
    )?;
    ensure(
        metadata.contains("branch: main"),
        "rust-prover-tools metadata should name the source branch",
    )?;
    ensure(
        ref_value.is_some_and(|value| value.len() == 40),
        "rust-prover-tools metadata should expose a 40-character commit ref",
    )?;
    ensure(
        metadata.contains("git ls-remote https://github.com/leynos/rust-prover-tools.git"),
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
    let makefile = makefile()?;
    let recipe = target_recipe(&makefile, target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;

    ensure(
        has_phony_target(&makefile, target),
        format!("`{target}` should be declared as a phony Make target"),
    )?;
    ensure(
        recipe.contains("$(PROVER_TOOLS)"),
        format!("`{target}` should delegate through the pinned prover-tools entry point"),
    )?;
    ensure(
        recipe.contains(prover_subcommand),
        format!("`{target}` should call `{prover_subcommand}`"),
    )
}

#[rstest]
#[case::install_kani("install-kani")]
#[case::check_kani_version("check-kani-version")]
#[case::install_verus("install-verus")]
#[case::run_verus("run-verus")]
fn makefile_targets_do_not_embed_bespoke_installer_logic(#[case] target: &str) -> TestResult {
    let makefile = makefile()?;
    let recipe = target_recipe(&makefile, target)
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
