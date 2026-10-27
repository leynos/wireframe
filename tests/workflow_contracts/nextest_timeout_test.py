"""Contracts for the coverage lane's cargo-nextest timeout policy."""

from __future__ import annotations

import tomllib
import typing as typ
from pathlib import Path

import pytest

from runner_placement_reader import REPO_ROOT, workflows

NEXTEST_CONFIG: Path = REPO_ROOT / ".config" / "nextest.toml"
#: The action this test is about, without its version pin.
COVERAGE_ACTION_NAME: str = "leynos/shared-actions/.github/actions/generate-coverage"
COMPILE_TEST_FILTER: str = (
    "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
)


def nextest_config() -> dict[str, typ.Any]:
    """Load the repository nextest configuration."""
    with NEXTEST_CONFIG.open("rb") as config_file:
        return tomllib.load(config_file)


def coverage_action_step() -> tuple[dict[str, typ.Any], dict[str, typ.Any]]:
    """Return the coverage job and its pinned generate-coverage step."""
    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    job = typ.cast(dict[str, typ.Any], workflow["jobs"]["build-test"])
    steps = typ.cast(list[dict[str, typ.Any]], job["steps"])
    matches = [
        step
        for step in steps
        if "generate-coverage" in str(step.get("uses", ""))
    ]
    assert len(matches) == 1, "CI must have one generate-coverage step"
    return job, matches[0]


def coverage_action_name(step: dict[str, typ.Any]) -> str:
    """Return the action a step invokes, without its version pin.

    The pin itself is not this contract's business: ``shared_actions_test``
    already requires every shared-actions invocation to carry a full commit
    SHA and to agree on one revision across the workflow tree. Repeating the
    literal SHA here made a dependabot bump of that shared revision fail a
    timeout test, because the two had to be updated together.

    Parameters
    ----------
    step
        One parsed workflow step.

    Returns
    -------
    str
        The ``uses`` value with its ``@`` pin removed.

    Examples
    --------
    >>> coverage_action_name(
    ...     {"uses": "leynos/shared-actions/.github/actions/generate-coverage@abc"}
    ... )
    'leynos/shared-actions/.github/actions/generate-coverage'
    """
    uses = str(step.get("uses", ""))
    name, separator, version = uses.partition("@")
    assert separator and name, f"action invocation {uses!r} carries no action name"
    assert version, f"action invocation {uses!r} carries an empty version pin"
    return name


@pytest.mark.parametrize(
    ("uses", "expected"),
    [
        (
            "leynos/shared-actions/.github/actions/generate-coverage@abc123",
            "leynos/shared-actions/.github/actions/generate-coverage",
        ),
        ("actions/checkout@v4", "actions/checkout"),
    ],
)
def test_coverage_action_name_strips_the_version_pin(uses: str, expected: str) -> None:
    """A pinned invocation yields the action path alone."""
    actual = coverage_action_name({"uses": uses})
    assert actual == expected, f"{uses!r} should name {expected!r}, got {actual!r}"


@pytest.mark.parametrize(
    "uses",
    [
        "actions/checkout",
        "actions/checkout@",
        "@v4",
        "",
    ],
)
def test_coverage_action_name_rejects_an_invocation_without_a_pin(uses: str) -> None:
    """A ``uses`` value with no pin, an empty pin, or no name is refused."""
    with pytest.raises(AssertionError, match="action invocation"):
        coverage_action_name({"uses": uses})


def test_coverage_action_can_discover_the_checked_in_config() -> None:
    """The coverage action uses the repository config without override."""
    job, step = coverage_action_step()

    assert NEXTEST_CONFIG.is_file()
    assert coverage_action_name(step) == COVERAGE_ACTION_NAME
    assert "NEXTEST_CONFIG" not in (job.get("env") or {})
    assert "NEXTEST_CONFIG" not in (step.get("env") or {})


def test_action_timeout_and_coverage_ceiling_are_preserved() -> None:
    """Ordinary tests retain the action policy and whole-run ceiling."""
    default = nextest_config()["profile"]["default"]

    assert default["slow-timeout"] == {
        "period": "180s",
        "terminate-after": 1,
        "grace-period": "5s",
    }
    assert default["global-timeout"] == "10m"


def test_only_the_aggregate_compile_test_gets_the_extended_budget() -> None:
    """The finite eight-minute override matches only the nested UI suite."""
    default = nextest_config()["profile"]["default"]
    overrides = default["overrides"]

    assert len(overrides) == 1
    override = overrides[0]
    assert override["filter"] == COMPILE_TEST_FILTER
    assert override["slow-timeout"] == {
        "period": "60s",
        "terminate-after": 8,
        "grace-period": "5s",
    }
    assert 60 * override["slow-timeout"]["terminate-after"] < 10 * 60


def assert_ui_job_is_mandatory(job: dict[str, typ.Any]) -> None:
    """Require the UI job to run the target after Rust and linker setup."""
    assert "if" not in job, "UI job must run for every CI event"
    assert not job.get("continue-on-error"), "UI job must fail CI on errors"
    steps = job["steps"]
    ui_steps = [step for step in steps if step.get("run") == "make test-ui"]
    assert len(ui_steps) == 1, "UI job must run make test-ui exactly once"
    ui = ui_steps[0]
    assert "if" not in ui and not ui.get("continue-on-error"), (
        "UI step must run and propagate failures"
    )
    setup = [
        step for step in steps
        if str(step.get("uses", "")).startswith(
            "leynos/shared-actions/.github/actions/setup-rust@"
        )
    ]
    assert len(setup) == 1, "UI job must set up Rust"
    assert "if" not in setup[0] and not setup[0].get("continue-on-error")
    assert str(setup[0].get("with", {}).get("install-mold")).lower() == "true"
    assert steps.index(setup[0]) < steps.index(ui), "setup must precede UI tests"
    for scope in (job, ui, setup[0]):
        env = scope.get("env") or {}
        flags = " ".join(str(env.get(key, "")) for key in (
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"
        ))
        assert "coverage" not in flags, "UI job must not enable coverage"


def test_ui_job_runs_without_suppression_for_all_ci_events() -> None:
    """CI retains a separate mandatory, non-instrumented UI gate."""
    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    events = workflow.get(True, workflow.get("on"))
    assert "pull_request" in events and "push" in events
    workflow_flags = workflow.get("env") or {}
    assert "coverage" not in str(workflow_flags.get("RUSTFLAGS", ""))
    assert "coverage" not in str(workflow_flags.get("CARGO_ENCODED_RUSTFLAGS", ""))
    assert "main" in events["push"]["branches"]
    assert_ui_job_is_mandatory(workflow["jobs"]["ui-test"])


def assert_ui_blocks_required_status(job: dict[str, typ.Any]) -> None:
    """The required build status depends on successful UI validation."""
    needs = job.get("needs", [])
    if isinstance(needs, str):
        needs = [needs]
    assert "ui-test" in needs, "build-test must depend on ui-test"
    assert "if" not in job, "build-test must not bypass failed dependencies"


def test_ui_failure_blocks_required_build_status() -> None:
    """A failed UI job prevents the required build status succeeding."""
    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    assert_ui_blocks_required_status(workflow["jobs"]["build-test"])


@pytest.mark.parametrize("mutation", ["missing_dependency", "always_run"])
def test_required_build_status_cannot_bypass_ui_failure(mutation: str) -> None:
    """Missing needs or an unconditional job expression must be rejected."""
    from copy import deepcopy

    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    job = deepcopy(workflow["jobs"]["build-test"])
    if mutation == "missing_dependency":
        job.pop("needs")
    else:
        job["if"] = "always()"
    with pytest.raises(AssertionError):
        assert_ui_blocks_required_status(job)


@pytest.mark.parametrize("mutation", ["conditional", "suppressed", "missing", "late_setup"])
def test_ui_job_contract_rejects_missing_or_ineffective_gate(mutation: str) -> None:
    """The gate contract detects realistic ways UI validation could be skipped."""
    from copy import deepcopy

    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    job = deepcopy(workflow["jobs"]["ui-test"])
    ui = next(step for step in job["steps"] if step.get("run") == "make test-ui")
    if mutation == "conditional":
        job["if"] = "false"
    elif mutation == "suppressed":
        ui["continue-on-error"] = True
    elif mutation == "missing":
        job["steps"].remove(ui)
    else:
        job["steps"].remove(ui)
        job["steps"].insert(0, ui)
    with pytest.raises(AssertionError):
        assert_ui_job_is_mandatory(job)


UI_HARNESSES = frozenset({
    REPO_ROOT / "tests" / "compile_error.rs",
    REPO_ROOT / "wireframe_testing" / "tests" / "loom_api_surface.rs",
})


def assert_coverage_guard_scope(guarded: set[Path]) -> None:
    """Keep coverage exclusion restricted to both compiler-driving tests."""
    assert guarded == UI_HARNESSES, "only the two UI harnesses need coverage guards"


def test_coverage_guards_only_compiler_driving_harnesses() -> None:
    """Coverage excludes nested compiler runs while ordinary UI tests remain."""
    guarded = {
        path
        for root in (REPO_ROOT / "tests", REPO_ROOT / "wireframe_testing" / "tests")
        for path in root.rglob("*.rs")
        if "#![cfg(not(coverage))]" in path.read_text()
    }
    assert_coverage_guard_scope(guarded)
    loom_harness = REPO_ROOT / "wireframe_testing" / "tests" / "loom_api_surface.rs"
    assert "#![cfg(loom)]" in loom_harness.read_text()
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())
    checks = manifest["workspace"]["lints"]["rust"]["unexpected_cfgs"]["check-cfg"]
    assert "cfg(loom)" in checks and "cfg(coverage)" in checks


def test_ui_target_keeps_both_harnesses() -> None:
    """The non-instrumented gate runs root and Loom compile-time suites."""
    makefile = (REPO_ROOT / "Makefile").read_text()
    target = makefile.split("\ntest-ui:", 1)[1].split("\n\n", 1)[0]
    assert "test --test compile_error" in target
    assert 'CARGO_ENCODED_RUSTFLAGS="--cfg=loom"' in target
    assert (
        "test -p wireframe_testing --test loom_api_surface --features runtime-benchmarks"
        in target
    )


@pytest.mark.parametrize("mutation", ["missing_guard", "unrelated_test"])
def test_coverage_guard_scope_rejects_missing_or_extra_guards(mutation: str) -> None:
    """A missing UI guard or an excluded runtime test violates the scope."""
    guarded = set(UI_HARNESSES)
    if mutation == "missing_guard":
        guarded.remove(REPO_ROOT / "tests" / "compile_error.rs")
    else:
        guarded.add(REPO_ROOT / "tests" / "request_handler.rs")
    with pytest.raises(AssertionError):
        assert_coverage_guard_scope(guarded)
