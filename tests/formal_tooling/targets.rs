//! Regression cases for formal-verification Make targets and version syntax.

use super::*;

/// A `make` target named by the formal-verification contract.
///
/// The case tables below name targets and the text their recipes owe. Holding
/// those in domain types keeps a string from meaning "target" in one case and
/// "expected recipe text" in the next.
#[derive(Debug, Clone, Copy)]
struct MakeTarget {
    /// Target name exactly as the Makefile declares it.
    name: &'static str,
}

impl MakeTarget {
    /// Name the target under test.
    const fn new(name: &'static str) -> Self { Self { name } }
}

impl AsRef<str> for MakeTarget {
    fn as_ref(&self) -> &str { self.name }
}

impl core::fmt::Display for MakeTarget {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name)
    }
}

/// Text a Make recipe is contractually expected to contain.
#[derive(Debug, Clone, Copy)]
struct RecipeText {
    /// Fragment the recipe must contain.
    text: &'static str,
}

impl RecipeText {
    /// Record the expected recipe fragment.
    const fn new(text: &'static str) -> Self { Self { text } }
}

impl AsRef<str> for RecipeText {
    fn as_ref(&self) -> &str { self.text }
}

/// Roadmap note quoted when a formal stub target declines to run.
#[derive(Debug, Clone, Copy)]
struct RoadmapNote {
    /// Roadmap reference reported by the stub.
    note: &'static str,
}

impl RoadmapNote {
    /// Record the roadmap note.
    const fn new(note: &'static str) -> Self { Self { note } }
}

impl AsRef<str> for RoadmapNote {
    fn as_ref(&self) -> &str { self.note }
}

#[rstest]
#[case::install_kani(MakeTarget::new("install-kani"))]
#[case::check_kani_version(MakeTarget::new("check-kani-version"))]
#[case::install_verus(MakeTarget::new("install-verus"))]
#[case::run_verus(MakeTarget::new("run-verus"))]
fn makefile_targets_do_not_embed_bespoke_installer_logic(#[case] target: MakeTarget) -> TestResult {
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
#[case::test_verification(MakeTarget::new("test-verification"))]
#[case::kani(MakeTarget::new("kani"))]
#[case::kani_full(MakeTarget::new("kani-full"))]
#[case::verus(MakeTarget::new("verus"))]
#[case::formal_pr(MakeTarget::new("formal-pr"))]
#[case::formal_nightly(MakeTarget::new("formal-nightly"))]
fn formal_execution_targets_are_declared(#[case] target: MakeTarget) -> TestResult {
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
#[case::test_verification(
    MakeTarget::new("test-verification"),
    RecipeText::new("test -p $(VERIFICATION_CRATE)")
)]
#[case::kani(MakeTarget::new("kani"), RecipeText::new("$(FORMAL_STUB) kani"))]
#[case::kani_full(
    MakeTarget::new("kani-full"),
    RecipeText::new("$(FORMAL_STUB) kani-full")
)]
#[case::verus(MakeTarget::new("verus"), RecipeText::new("$(FORMAL_STUB) verus"))]
fn direct_recipe_targets_have_expected_content(
    #[case] target: MakeTarget,
    #[case] expected_content: RecipeText,
) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let recipe = makefile
        .target_recipe(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;
    let expected_content = expected_content.as_ref();

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
#[case::formal_pr(
    MakeTarget::new("formal-pr"),
    &[MakeTarget::new("test-verification"), MakeTarget::new("kani"), MakeTarget::new("verus")]
)]
#[case::formal_nightly(
    MakeTarget::new("formal-nightly"),
    &[
        MakeTarget::new("test-verification"),
        MakeTarget::new("kani-full"),
        MakeTarget::new("verus")
    ]
)]
fn aggregate_targets_declare_expected_prerequisites(
    #[case] target: MakeTarget,
    #[case] expected_prerequisites: &[MakeTarget],
) -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let prerequisites = makefile
        .target_prerequisites(target)
        .ok_or_else(|| format!("expected `{target}` target in Makefile"))?;
    let expected_prerequisites = expected_prerequisites
        .iter()
        .map(|prerequisite| prerequisite.as_ref().to_owned())
        .collect::<Vec<_>>();

    ensure(
        prerequisites == expected_prerequisites,
        format!("`{target}` should have the expected prerequisites"),
    )
}

#[rstest]
#[case::kani(
    MakeTarget::new("kani"),
    RoadmapNote::new("roadmap 15.3.1 adds src/frame Kani smoke harnesses")
)]
#[case::kani_full(
    MakeTarget::new("kani-full"),
    RoadmapNote::new("roadmap 15.3.x adds the full Kani harness set")
)]
#[case::verus(
    MakeTarget::new("verus"),
    RoadmapNote::new("roadmap 15.5.2 adds verus/wireframe_proofs.rs")
)]
fn stub_targets_skip_and_exit_zero(
    #[case] target: MakeTarget,
    #[case] roadmap_message: RoadmapNote,
) -> TestResult {
    let (status, _stdout, stderr) = run_make(target, false)?;
    let roadmap_message = roadmap_message.as_ref();

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
#[case::kani(
    MakeTarget::new("kani"),
    RoadmapNote::new("roadmap 15.3.1 adds src/frame Kani smoke harnesses")
)]
#[case::kani_full(
    MakeTarget::new("kani-full"),
    RoadmapNote::new("roadmap 15.3.x adds the full Kani harness set")
)]
#[case::verus(
    MakeTarget::new("verus"),
    RoadmapNote::new("roadmap 15.5.2 adds verus/wireframe_proofs.rs")
)]
fn stub_targets_fail_under_formal_strict(
    #[case] target: MakeTarget,
    #[case] roadmap_message: RoadmapNote,
) -> TestResult {
    let (status, _stdout, stderr) = run_make(target, true)?;
    let expected_skip = format!(
        "FORMAL-SKIP: {target} not yet implemented — {}",
        roadmap_message.as_ref()
    );

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
#[case::test_verification(
    MakeTarget::new("test-verification"),
    RecipeText::new("wireframe-verification")
)]
#[case::formal_pr(MakeTarget::new("formal-pr"), RecipeText::new("formal-stub.sh"))]
#[case::formal_nightly(MakeTarget::new("formal-nightly"), RecipeText::new("formal-stub.sh"))]
fn aggregate_targets_dry_run_zero(
    #[case] target: MakeTarget,
    #[case] expected_content: RecipeText,
) -> TestResult {
    let output = run_make_dry_run(target)?;
    let expected_content = expected_content.as_ref();

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
        .target_prerequisites(MakeTarget::new("formal"))
        .ok_or_else(|| "expected `formal` target in Makefile".to_owned())?;

    ensure(
        makefile.has_phony_target(MakeTarget::new("formal")),
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
