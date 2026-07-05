//! Contract checks for the mutation-testing caller workflow.
//!
//! The executable logic lives in the `leynos/shared-actions` reusable
//! workflow, which carries its own unit and integration tests; this
//! repository's caller is declarative configuration. These checks pin
//! the contract the caller must uphold so regressions (for example,
//! repointing the pin at a branch, dropping the OIDC token scope, or
//! losing the issue-571 configuration) fail `make test` instead of
//! surfacing in a scheduled run.

const WORKFLOW: &str = include_str!("../.github/workflows/mutation-testing.yml");

/// Return the trimmed `uses:` line referencing the shared workflow.
fn shared_workflow_reference() -> &'static str {
    let reference = WORKFLOW
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("uses: leynos/shared-actions/"));
    match reference {
        Some(line) => line,
        None => panic!("the caller must reference the shared-actions reusable workflow"),
    }
}

#[test]
fn caller_pins_the_shared_workflow_by_full_commit_sha() {
    let reference = shared_workflow_reference();
    let Some((path, remainder)) = reference.split_once('@') else {
        panic!("the shared workflow reference must carry an @<ref> pin: {reference}");
    };
    assert!(
        path.ends_with("/.github/workflows/mutation-cargo.yml"),
        "the caller must reference mutation-cargo.yml, got {path}"
    );
    let Some(pin) = remainder.split_whitespace().next() else {
        panic!("the shared workflow pin must not be empty: {reference}");
    };
    assert!(
        pin.len() == 40 && pin.chars().all(|c| c.is_ascii_hexdigit()),
        "the shared workflow must be pinned by a full 40-hex commit SHA, got {pin}"
    );
}

#[test]
fn caller_grants_least_privilege_with_the_oidc_scope() {
    assert!(
        WORKFLOW.contains("permissions: {}"),
        "the workflow-level default token scope must be empty"
    );
    assert!(
        WORKFLOW.contains("contents: read"),
        "the calling job must grant contents: read"
    );
    assert!(
        WORKFLOW.contains("id-token: write"),
        "the calling job must grant id-token: write for OIDC source resolution"
    );
}

#[test]
fn caller_serializes_runs_per_ref_without_cancelling() {
    let group = WORKFLOW
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("group:"));
    match group {
        Some(line) => assert!(
            line.contains("${{ github.ref }}"),
            "the concurrency group must key on the triggering ref, got {line}"
        ),
        None => panic!("the workflow must declare a concurrency group"),
    }
    assert!(
        WORKFLOW.contains("cancel-in-progress: false"),
        "informational runs must queue rather than cancel"
    );
}

#[test]
fn caller_keeps_the_scheduled_and_dispatch_triggers() {
    assert!(
        WORKFLOW.contains("schedule:") && WORKFLOW.contains("cron:"),
        "the workflow must keep its cron schedule"
    );
    assert!(
        WORKFLOW.contains("workflow_dispatch:"),
        "the workflow must remain manually dispatchable"
    );
}

#[test]
fn caller_carries_the_issue_571_configuration() {
    assert!(
        WORKFLOW.contains(r#"extra-args: "--all-features""#),
        "mutation runs must enable all features so feature-gated tests run (#571)"
    );
    for scaffold in [
        "src/codec/examples.rs",
        "src/test_helpers.rs",
        "src/connection/test_support.rs",
    ] {
        assert!(
            WORKFLOW.contains(scaffold),
            "mutation runs must exclude scaffolding path {scaffold} (#571)"
        );
    }
}

#[test]
fn caller_targets_the_companion_crate_separately() {
    assert!(
        WORKFLOW.contains(r#"extra-crate-dirs: "wireframe_testing""#),
        "wireframe_testing must remain a separate mutation target"
    );
}
