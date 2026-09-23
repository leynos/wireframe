//! Regression cases for formal-verification Make targets and version syntax.

use super::*;

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

#[rstest]
#[case::test_verification("test-verification")]
#[case::kani("kani")]
#[case::kani_full("kani-full")]
#[case::verus("verus")]
#[case::formal_pr("formal-pr")]
#[case::formal_nightly("formal-nightly")]
fn formal_execution_targets_are_declared(#[case] target: &str) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);

    ensure(
        makefile.has_phony_target(target),
        format!("`{target}` should be declared as a phony Make target"),
    )?;
    ensure(
        makefile.target_prerequisites(target).is_some(),
        format!("`{target}` should have a Make rule"),
    )
}

#[rstest]
#[case::test_verification("test-verification", "test -p $(VERIFICATION_CRATE)")]
#[case::kani("kani", "$(FORMAL_STUB) kani")]
#[case::kani_full("kani-full", "$(FORMAL_STUB) kani-full")]
#[case::verus("verus", "$(FORMAL_STUB) verus")]
fn direct_recipe_targets_have_expected_content(
    #[case] target: &str,
    #[case] expected_content: &str,
) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let recipe = makefile
        .target_recipe(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;

    ensure(
        recipe.contains(expected_content),
        format!("`{target}` should contain `{expected_content}`"),
    )
}

#[rstest]
fn test_verification_executes_the_configured_cargo_command() -> TestResult {
    let cargo_wrapper = TemporaryCargoWrapper::new()?;
    let output = Command::new("make")
        .arg("test-verification")
        .arg(format!("CARGO={}", cargo_wrapper.path.as_str()))
        .current_dir(repo_access::repo_root()?)
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let wrapper_output = stdout
        .lines()
        .filter(|line| line.starts_with("WRAPPER_"))
        .collect::<Vec<_>>();

    ensure(
        output.status.success(),
        "`make test-verification` should execute the configured Cargo command",
    )?;
    ensure(
        wrapper_output
            == [
                "WRAPPER_RUSTFLAGS=-D warnings",
                "WRAPPER_ARG=test",
                "WRAPPER_ARG=-p",
                "WRAPPER_ARG=wireframe-verification",
            ],
        "`make test-verification` should invoke Cargo with `RUSTFLAGS=-D warnings` and `test -p \
         wireframe-verification`",
    )
}

#[rstest]
#[case::formal_pr("formal-pr", &["test-verification", "kani", "verus"])]
#[case::formal_nightly(
    "formal-nightly",
    &["test-verification", "kani-full", "verus"]
)]
fn aggregate_targets_declare_expected_prerequisites(
    #[case] target: &str,
    #[case] expected_prerequisites: &[&str],
) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let prerequisites = makefile
        .target_prerequisites(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;
    let expected_prerequisites = expected_prerequisites
        .iter()
        .map(|prerequisite| (*prerequisite).to_owned())
        .collect::<Vec<_>>();

    ensure(
        prerequisites == expected_prerequisites,
        format!("`{target}` should have the expected prerequisites"),
    )
}

#[rstest]
#[case::kani("kani", "roadmap 15.3.1 adds src/frame Kani smoke harnesses")]
#[case::kani_full("kani-full", "roadmap 15.3.x adds the full Kani harness set")]
#[case::verus("verus", "roadmap 15.5.2 adds verus/wireframe_proofs.rs")]
fn stub_targets_skip_and_exit_zero(
    #[case] target: &str,
    #[case] roadmap_message: &str,
) -> TestResult {
    let (status, _stdout, stderr) = run_make(target, false)?;

    ensure(
        status.success(),
        format!("`make {target}` should exit zero"),
    )?;
    ensure(
        stderr == format!("FORMAL-SKIP: {target} not yet implemented — {roadmap_message}\n"),
        format!("`make {target}` should report its complete formal skip"),
    )
}

#[rstest]
#[case::kani("kani", "roadmap 15.3.1 adds src/frame Kani smoke harnesses")]
#[case::kani_full("kani-full", "roadmap 15.3.x adds the full Kani harness set")]
#[case::verus("verus", "roadmap 15.5.2 adds verus/wireframe_proofs.rs")]
fn stub_targets_fail_under_formal_strict(
    #[case] target: &str,
    #[case] roadmap_message: &str,
) -> TestResult {
    let (status, _stdout, stderr) = run_make(target, true)?;
    let expected_skip = format!("FORMAL-SKIP: {target} not yet implemented — {roadmap_message}");

    ensure(
        !status.success(),
        format!("`FORMAL_STRICT=1 make {target}` should fail"),
    )?;
    // GNU Make appends its failure diagnostic after the stub's fixed line.
    ensure(
        stderr.lines().next() == Some(&expected_skip),
        format!("`FORMAL_STRICT=1 make {target}` should first report its complete formal skip"),
    )
}

#[rstest]
#[case::test_verification("test-verification", "wireframe-verification")]
#[case::formal_pr("formal-pr", "formal-stub.sh")]
#[case::formal_nightly("formal-nightly", "formal-stub.sh")]
fn aggregate_targets_dry_run_zero(
    #[case] target: &str,
    #[case] expected_content: &str,
) -> TestResult {
    let output = run_make_dry_run(target)?;

    ensure(
        output.contains(expected_content),
        format!("`make --dry-run {target}` should emit `{expected_content}`"),
    )
}

#[rstest]
fn formal_alias_delegates_to_formal_pr() -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let prerequisites = makefile
        .target_prerequisites("formal")
        .ok_or_else(|| "expected `formal` target in Makefile".to_owned())?;

    ensure(
        makefile.has_phony_target("formal"),
        "`formal` should be declared as a phony Make target",
    )?;
    ensure(
        prerequisites == ["formal-pr"],
        "`formal` should delegate to `formal-pr`",
    )
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
