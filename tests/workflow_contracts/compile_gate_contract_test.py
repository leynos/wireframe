"""Pin how CI runs compile-time validation and runtime coverage.

`build-test` must run `make test-ui` unconditionally, after Rust and linker
setup and with a finite budget, and it must not carry coverage instrumentation.
The gate itself keeps both harnesses, `tests/compile_error.rs` and
`wireframe_testing/tests/loom_api_surface.rs`, and coverage opts out of exactly
those two nested compiler drivers rather than the runtime suite around them.

The timeout ordering for the nextest policy and the coverage invocation lives
in `nextest_timeout_test`; this module pins which of those paths CI actually
executes. The `test-ui` recipe itself is pinned in `makefile_recipe_test`.
"""

from __future__ import annotations

import tomllib
import typing as typ
from copy import deepcopy
from pathlib import Path

import pytest

from runner_placement_reader import REPO_ROOT, workflows
from rust_source_scanning import has_coverage_gate, is_loom_gated


def assert_workflow_contracts_follow_setup(job: dict[str, typ.Any]) -> None:
    """Require the workflow-contract step to follow Rust setup in ``build-test``."""
    steps = job["steps"]
    contract_steps = [
        step for step in steps if step.get("run") == "make test-workflow-contracts"
    ]
    assert len(contract_steps) == 1, "build-test must run the contract suite once"
    contract = contract_steps[0]
    assert "if" not in contract and not contract.get("continue-on-error"), (
        "contract step must run and propagate failures"
    )
    setup = [
        step
        for step in steps
        if str(step.get("uses", "")).startswith(
            "leynos/shared-actions/.github/actions/setup-rust@"
        )
    ]
    assert len(setup) == 1, "build-test must set up Rust"
    assert steps.index(setup[0]) < steps.index(contract_steps[0]), (
        "workflow contracts must run after Rust setup"
    )


def test_workflow_contracts_step_follows_rust_setup() -> None:
    """The contract suite runs in the required job, after Rust is available."""
    job = typ.cast(
        "dict[str, typ.Any]", workflows()["ci.yml"]["jobs"]["build-test"]
    )
    assert_workflow_contracts_follow_setup(job)


def test_workflow_contracts_order_rejects_a_premature_step() -> None:
    """A contract step moved ahead of Rust setup violates the ordering."""
    job = deepcopy(workflows()["ci.yml"]["jobs"]["build-test"])
    contract = next(
        step for step in job["steps"]
        if step.get("run") == "make test-workflow-contracts"
    )
    job["steps"].remove(contract)
    job["steps"].insert(0, contract)
    with pytest.raises(AssertionError):
        assert_workflow_contracts_follow_setup(job)


@pytest.mark.parametrize(
    ("field", "value"),
    [("if", "false"), ("continue-on-error", True)],
    ids=["conditional", "suppressed"],
)
def test_workflow_contracts_step_rejects_suppression(
    field: str, value: object
) -> None:
    """A skipped or non-blocking contract step no longer gates the build."""
    job = deepcopy(workflows()["ci.yml"]["jobs"]["build-test"])
    contract = next(
        step for step in job["steps"]
        if step.get("run") == "make test-workflow-contracts"
    )
    contract[field] = value
    with pytest.raises(AssertionError):
        assert_workflow_contracts_follow_setup(job)


def assert_ui_step_is_mandatory(job: dict[str, typ.Any]) -> None:
    """Require bounded UI tests after Rust and linker setup in the required job."""
    assert "if" not in job, "build-test must run for every CI event"
    assert not job.get("continue-on-error"), "build-test must propagate failures"
    assert "needs" not in job, "build-test must run without a skipped prerequisite"
    assert job.get("timeout-minutes") == 60, "build-test needs the reviewed budget"
    steps = job["steps"]
    ui_steps = [
        step for step in steps if step.get("name") == "Validate compile-time APIs"
    ]
    assert len(ui_steps) == 1, "build-test must contain exactly one UI step"
    ui = ui_steps[0]
    assert ui.get("run") == "make test-ui", "UI step must run the complete target"
    assert ui.get("timeout-minutes") == 30, "UI step must have a finite budget"
    assert "if" not in ui and not ui.get("continue-on-error"), (
        "UI step must run and propagate failures"
    )
    assert_ui_setup_precedes(steps, ui)
    assert_no_coverage_flags(job, ui)


def assert_ui_setup_precedes(
    steps: list[dict[str, typ.Any]], ui: dict[str, typ.Any]
) -> None:
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
    workflow = typ.cast("dict[str, typ.Any]", workflows()["ci.yml"])
    events = workflow.get(True, workflow.get("on"))
    assert "pull_request" in events and "push" in events, (
        "build-test must run for pull requests and pushes"
    )
    workflow_flags = workflow.get("env") or {}
    assert "coverage" not in str(workflow_flags.get("RUSTFLAGS", "")), (
        "workflow RUSTFLAGS must not enable coverage"
    )
    assert "coverage" not in str(workflow_flags.get("CARGO_ENCODED_RUSTFLAGS", "")), (
        "workflow CARGO_ENCODED_RUSTFLAGS must not enable coverage"
    )
    assert "main" in events["push"]["branches"], (
        "pushes to main must run the required UI gate"
    )
    assert_ui_step_is_mandatory(workflow["jobs"]["build-test"])
    assert_ui_permissions(workflow["jobs"]["build-test"])


@pytest.mark.parametrize(
    "permissions",
    [None, {"contents": "write"}, {"contents": "read", "actions": "read"}],
)
def test_ui_permissions_reject_missing_or_broader_access(permissions: object) -> None:
    """The required UI gate must not omit or broaden its token permissions."""
    with pytest.raises(AssertionError, match="contents: read"):
        assert_ui_permissions({"permissions": permissions})


@pytest.mark.parametrize(
    "mutation", ["conditional", "suppressed", "missing", "wrong_order", "no_timeout"]
)
def test_ui_step_contract_rejects_missing_or_ineffective_gate(mutation: str) -> None:
    """Reject skipped, suppressed, missing, unbounded or prematurely run UI tests."""
    workflow = typ.cast("dict[str, typ.Any]", workflows()["ci.yml"])
    job = deepcopy(workflow["jobs"]["build-test"])
    ui = next(
        step
        for step in job["steps"]
        if step.get("name") == "Validate compile-time APIs"
    )
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
        if has_coverage_gate(path.read_text())
    }
    assert_coverage_guard_scope(guarded)
    loom_harness = REPO_ROOT / "wireframe_testing" / "tests" / "loom_api_surface.rs"
    assert is_loom_gated(loom_harness.read_text()), (
        "the Loom harness must stay gated out of the ordinary test run"
    )
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())
    checks = manifest["workspace"]["lints"]["rust"]["unexpected_cfgs"]["check-cfg"]
    assert "cfg(loom)" in checks and "cfg(coverage)" in checks, (
        "both conditional-compilation flags must be declared to the lints"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param("//! Harness.\n\n#![cfg(not(coverage))]\n", True, id="active"),
        pytest.param("// #![cfg(not(coverage))]\n", False, id="line-comment"),
        pytest.param("/* #![cfg(not(coverage))] */\n", False, id="block-comment"),
        pytest.param(
            'const G: &str = "#![cfg(not(coverage))]";', False, id="string"
        ),
        pytest.param(
            "mod m {\n    #![cfg(not(coverage))]\n}\n", False, id="nested"
        ),
    ],
)
def test_coverage_guard_must_be_active(source: str, expected: bool) -> None:
    """A shadowed or nested gate cannot exclude a harness from coverage."""
    assert has_coverage_gate(source) is expected, (
        "only a live crate-level coverage gate excludes a harness"
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
