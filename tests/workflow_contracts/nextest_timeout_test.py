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


def assert_ui_step_is_mandatory(job: dict[str, typ.Any]) -> None:
    """Require bounded UI tests after Rust and linker setup in the required job."""
    assert "if" not in job, "build-test must run for every CI event"
    assert not job.get("continue-on-error"), "build-test must propagate failures"
    assert "needs" not in job, "build-test must run without a skipped prerequisite"
    assert job.get("timeout-minutes") == 60, "build-test needs the reviewed budget"
    steps = job["steps"]
    ui_steps = [step for step in steps if step.get("name") == "Validate compile-time APIs"]
    assert len(ui_steps) == 1, "build-test must contain exactly one UI step"
    ui = ui_steps[0]
    assert ui.get("run") == "make test-ui", "UI step must run the complete target"
    assert ui.get("timeout-minutes") == 30, "UI step must have a finite budget"
    assert "if" not in ui and not ui.get("continue-on-error"), (
        "UI step must run and propagate failures"
    )
    assert_ui_setup_precedes(steps, ui)
    assert_no_coverage_flags(job, ui)


def assert_ui_setup_precedes(steps: list[dict[str, typ.Any]], ui: dict[str, typ.Any]) -> None:
    """Require Rust and linker setup before the non-instrumented UI step."""
    setup = [
        step for step in steps
        if str(step.get("uses", "")).startswith(
            "leynos/shared-actions/.github/actions/setup-rust@"
        )
    ]
    assert len(setup) == 1, "build-test must set up Rust"
    assert "if" not in setup[0] and not setup[0].get("continue-on-error"), (
        "Rust setup must run and propagate failures"
    )
    linker = [
        step for step in steps
        if step.get("name") == "Install development backend and linker"
    ]
    assert len(linker) == 1, "build-test must install the Linux linker"
    assert "if" not in linker[0] and not linker[0].get("continue-on-error"), (
        "linker setup must run and propagate failures"
    )
    assert steps.index(setup[0]) < steps.index(linker[0]) < steps.index(ui), (
        "Rust and linker setup must precede UI tests"
    )
    assert_no_coverage_flags(setup[0], linker[0])


def assert_no_coverage_flags(*scopes: dict[str, typ.Any]) -> None:
    """Keep coverage instrumentation out of the UI job and its setup."""
    for scope in scopes:
        env = scope.get("env") or {}
        flags = " ".join(str(env.get(key, "")) for key in (
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"
        ))
        assert "coverage" not in flags, "UI step must not enable coverage"


def assert_ui_permissions(job: dict[str, typ.Any]) -> None:
    """Restrict the required UI job to read-only repository contents."""
    assert job.get("permissions") == {"contents": "read"}, (
        "build-test must declare exactly contents: read permissions"
    )


def test_ui_step_runs_without_suppression_for_all_ci_events() -> None:
    """The required CI job runs a bounded non-instrumented UI gate."""
    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    events = workflow.get(True, workflow.get("on"))
    assert "pull_request" in events and "push" in events
    workflow_flags = workflow.get("env") or {}
    assert "coverage" not in str(workflow_flags.get("RUSTFLAGS", ""))
    assert "coverage" not in str(workflow_flags.get("CARGO_ENCODED_RUSTFLAGS", ""))
    assert "main" in events["push"]["branches"]
    assert_ui_step_is_mandatory(workflow["jobs"]["build-test"])
    assert_ui_permissions(workflow["jobs"]["build-test"])


@pytest.mark.parametrize("permissions", [None, {"contents": "write"}, {"contents": "read", "actions": "read"}])
def test_ui_permissions_reject_missing_or_broader_access(permissions: object) -> None:
    """The required UI gate must not omit or broaden its token permissions."""
    with pytest.raises(AssertionError, match="contents: read"):
        assert_ui_permissions({"permissions": permissions})


@pytest.mark.parametrize(
    "mutation", ["conditional", "suppressed", "missing", "wrong_order", "no_timeout"]
)
def test_ui_step_contract_rejects_missing_or_ineffective_gate(mutation: str) -> None:
    """Reject skipped, suppressed, missing, unbounded or prematurely run UI tests."""
    from copy import deepcopy

    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    job = deepcopy(workflow["jobs"]["build-test"])
    ui = next(step for step in job["steps"] if step.get("name") == "Validate compile-time APIs")
    if mutation == "conditional":
        ui["if"] = "false"
    elif mutation == "suppressed":
        ui["continue-on-error"] = True
    elif mutation == "missing":
        job["steps"].remove(ui)
    elif mutation == "no_timeout":
        ui.pop("timeout-minutes")
    else:
        job["steps"].remove(ui)
        job["steps"].insert(0, ui)
    with pytest.raises(AssertionError):
        assert_ui_step_is_mandatory(job)


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
