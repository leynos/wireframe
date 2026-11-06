//! Makefile contract checks for the workspace-wide lint pipeline.
//!
//! The parent test stays a short narrative by delegating each group of
//! related assertions to one helper in this module.

use super::{
    TestResult,
    ensure,
    formal_tooling_support::{MakefileContent, makefile},
};

/// Check that `test`, `test-doc` and `typecheck` still validate the intended
/// workspace scope.
pub(super) fn assert_workspace_validation_scope(makefile: &MakefileContent<'_>) -> TestResult {
    let recipe = |target: &str| {
        makefile
            .target_recipe(target)
            .ok_or_else(|| format!("expected `{target}` target in Makefile"))
    };

    let test_recipe = recipe("test")?;
    let test_doc_recipe = recipe("test-doc")?;
    let typecheck_recipe = recipe("typecheck")?;

    for (target, target_recipe) in [
        ("test", &test_recipe),
        ("test-doc", &test_doc_recipe),
        ("typecheck", &typecheck_recipe),
    ] {
        ensure(
            target_recipe.contains("--workspace"),
            format!("`{target}` should validate the supported workspace members"),
        )?;
    }
    ensure(
        test_recipe.contains("--all-targets --all-features"),
        "`test` should retain all-target and all-feature coverage",
    )?;
    ensure(
        !test_recipe.contains("--exclude") && !test_recipe.contains(" -p "),
        "`test` should not restrict the workspace it validates",
    )?;
    ensure(
        test_doc_recipe.contains("--workspace --doc --all-features"),
        "`test-doc` should run every workspace member's doctests with all features",
    )?;
    ensure(
        !test_doc_recipe.contains("--exclude"),
        "`test-doc` should not exclude a workspace member from doctests",
    )?;
    ensure(
        typecheck_recipe.contains("--all-targets --all-features"),
        "`typecheck` should retain all-target and all-feature coverage",
    )
}

/// Check that `lint` aggregates the three leaves and each leaf is phony.
pub(super) fn assert_lint_aggregate(makefile: &MakefileContent<'_>) -> TestResult {
    let lint_prerequisites = makefile
        .target_prerequisites("lint")
        .ok_or_else(|| "expected `lint` target in Makefile".to_owned())?;
    ensure(
        lint_prerequisites
            == [
                "lint-doc",
                "lint-clippy",
                "lint-whitaker",
                "lint-python-docs",
            ],
        "`lint` should aggregate the Rust lint leaves in order, then the Python docstring check",
    )?;
    for leaf in [
        "lint-doc",
        "lint-clippy",
        "lint-whitaker",
        "lint-python-docs",
        "install-whitaker",
    ] {
        ensure(
            makefile.has_phony_target(leaf),
            format!("`{leaf}` should be declared as a phony Make target"),
        )?;
    }
    Ok(())
}

/// Check the recipe and assignment text of the lint leaves.
///
/// The raw Makefile text is read here rather than passed in: the sibling
/// helpers already receive it, and threading a second borrowed string through
/// the call sites obscures which assertions own which text.
pub(super) fn assert_lint_leaf_flags() -> TestResult {
    let makefile_str = makefile()?;
    let makefile = MakefileContent(&makefile_str);
    let lint_doc_recipe = makefile
        .target_recipe("lint-doc")
        .ok_or_else(|| "expected `lint-doc` target in Makefile".to_owned())?;
    let lint_clippy_recipe = makefile
        .target_recipe("lint-clippy")
        .ok_or_else(|| "expected `lint-clippy` target in Makefile".to_owned())?;
    let lint_whitaker_recipe = makefile
        .target_recipe("lint-whitaker")
        .ok_or_else(|| "expected `lint-whitaker` target in Makefile".to_owned())?;

    ensure(
        lint_doc_recipe.contains("doc $(DOC_FLAGS)"),
        "`lint-doc` should build rustdoc with the configured documentation policy",
    )?;
    ensure(
        lint_doc_recipe.contains("RUSTDOCFLAGS=\"$(RUSTDOC_FLAGS)\""),
        "`lint-doc` should pass rustdoc's own warning-denying flags",
    )?;
    ensure(
        lint_clippy_recipe.contains("clippy $(CLIPPY_FLAGS)"),
        "`lint-clippy` should invoke the configured Clippy policy",
    )?;
    ensure(
        lint_whitaker_recipe.contains("$(WHITAKER) $(WHITAKER_FLAGS)"),
        "`lint-whitaker` should invoke Whitaker with its configured flags",
    )?;

    for assignment in [
        "DOC_FLAGS ?= --workspace --all-features --no-deps",
        "CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings",
        "WHITAKER_FLAGS ?= --all -- --workspace --all-targets --all-features",
    ] {
        ensure(
            makefile_str.lines().any(|line| line == assignment),
            format!("the lint pipeline should assign `{assignment}`"),
        )?;
    }
    Ok(())
}
