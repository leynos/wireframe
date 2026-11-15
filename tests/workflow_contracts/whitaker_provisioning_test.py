"""Protect the workflow-wide Whitaker provisioning contract.

CI installs Whitaker through exactly one shared ``install-whitaker`` action
step, which owns the installer version and takes no pin, and no workflow
script, ``make install-whitaker`` step or version variable provisions it
another way; the Makefile's pins serve contributors only. That check reads
every job and step of every workflow file, so a route added under a new step
name or in a new workflow still fails it. How a script is read for a Make
install lives in ``make_route_reader``.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from pathlib import Path
from typing import cast

import pytest
import yaml
from make_route_reader import runs_make_install_whitaker

WORKFLOW_PATH = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
MAKEFILE_PATH = Path(__file__).resolve().parents[2] / "Makefile"
INSTALL_WHITAKER_ACTION_RE = re.compile(
    r"^leynos/shared-actions/\.github/actions/install-whitaker@[0-9a-f]{40}$"
)
#: Pin names that must live only in the Makefile.
WHITAKER_PIN_TOKENS = (
    "WHITAKER_INSTALLER_VERSION",
    "CARGO_DYLINT_VERSION",
    "DYLINT_LINK_VERSION",
    "WHITAKER_SUITE_REV",
)
WORKFLOWS_DIRECTORY = WORKFLOW_PATH.parent
WHITAKER_ROUTE_RE = re.compile(
    r"whitaker-installer"
    r"|cargo\s+(\+\S+\s+)?(install|binstall)[^\n]*(cargo-dylint|dylint-link)"
    r"|cargo\s+(\+\S+\s+)?dylint\b"
    # A path-qualified binary such as `./target/debug/whitaker` is still a
    # manual route, so only a word character or hyphen before the name
    # disqualifies it: the `install-whitaker` action path must stay out of the
    # scan.
    r"|(?<![\w-])whitaker(?:-ls)?\b"
    r"|(?<![\w-])(cargo-dylint|dylint-link)\b",
)


def _workflow_steps() -> list[tuple[str, str, dict[str, object]]]:
    """Return (workflow file, job, step) for every step in every workflow."""
    found: list[tuple[str, str, dict[str, object]]] = []
    for path in sorted(WORKFLOWS_DIRECTORY.glob("*.y*ml")):
        # An empty workflow file parses to None; treat it as having no jobs.
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        jobs = cast("dict[str, dict[str, object]]", document.get("jobs") or {})
        for job_name, job in jobs.items():
            for step in cast("list[dict[str, object]]", job.get("steps") or []):
                found.append((path.name, job_name, step))
    return found


def _assert_installed_before_lint(install: dict[str, object]) -> None:
    """Require the install in `ci.yml`'s `build-test` job, before `make lint`.

    `make lint` runs `whitaker`, so provisioning it in another job, or after
    the `Lint` step, leaves the lint command running without its tool while a
    count-only check still passes.
    """
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    steps = cast("list[dict[str, object]]", workflow["jobs"]["build-test"]["steps"])
    install_index = next(
        (i for i, step in enumerate(steps) if step is install or step == install),
        None,
    )
    lint_index = next(
        (
            i
            for i, step in enumerate(steps)
            if str(step.get("run", "")).strip() == "make lint"
        ),
        None,
    )
    assert install_index is not None, "install-whitaker must be a build-test step"
    assert lint_index is not None, "build-test must run `make lint`"
    assert install_index < lint_index, (
        "install-whitaker must run before the Lint step that invokes whitaker"
    )


def _step_labels(
    steps: list[tuple[str, str, dict[str, object]]],
    predicate: Callable[[dict[str, object]], bool],
) -> list[str]:
    """Return ``file:job: step`` for each step the predicate selects."""
    return [
        f"{file}:{job}: {step.get('name')}"
        for file, job, step in steps
        if predicate(step)
    ]


def _whitaker_routes(
    steps: list[tuple[str, str, dict[str, object]]],
) -> list[str]:
    """Return ``file:job: step`` for each step that runs Whitaker by hand."""
    return _step_labels(
        steps, lambda step: WHITAKER_ROUTE_RE.search(str(step.get("run", ""))) is not None
    )


def _make_install_routes(
    steps: list[tuple[str, str, dict[str, object]]],
) -> list[str]:
    """Return ``file:job: step`` for each step that runs ``make install-whitaker``."""
    return _step_labels(
        steps, lambda step: runs_make_install_whitaker(str(step.get("run", "")))
    )


def _install_steps(
    steps: list[tuple[str, str, dict[str, object]]],
) -> list[tuple[str, str, dict[str, object]]]:
    """Return the steps that use the shared ``install-whitaker`` action."""
    return [
        (file, job, step)
        for file, job, step in steps
        if "install-whitaker@" in str(step.get("uses", ""))
    ]


def test_whitaker_has_one_shared_action_install_before_lint() -> None:
    """Exactly one step installs Whitaker, through the shared action, in order.

    Invariant, over every job and step of every workflow: one shared
    ``install-whitaker`` action step, at a full commit SHA, in ci.yml's
    ``build-test`` job before the exact ``make lint`` step. The count is
    asserted first, because a contract over no steps is satisfied by deleting
    the install.
    """
    installs = _install_steps(_workflow_steps())
    assert len(installs) == 1, (
        f"expected one shared install-whitaker action step, found {len(installs)}"
    )
    (file, job, install) = installs[0]
    assert (file, job) == ("ci.yml", "build-test"), (
        "the install must run in ci.yml's build-test job"
    )
    assert INSTALL_WHITAKER_ACTION_RE.fullmatch(str(install["uses"])), (
        f"the install action must be pinned to a full commit SHA: {install['uses']}"
    )
    _assert_installed_before_lint(install)


def test_the_install_action_passes_no_pin() -> None:
    """The action owns the installer version and refuses a suite pin."""
    (_file, _job, install) = _install_steps(_workflow_steps())[0]
    options = install.get("with") or {}
    assert isinstance(options, dict), "the install step's with: must be a mapping"
    pinned = sorted(
        key for key in options if key in {"suite-version", "installer-version"}
    )
    assert not pinned, f"the action owns the pins; the step passes {pinned}"


def test_no_other_route_installs_or_runs_whitaker_in_ci() -> None:
    """No Make install, script, cache step or version token is another route.

    The ``make install-whitaker`` target is the contributor's route; no ``run``
    script installs or invokes the Whitaker tools; and no repository-owned
    Whitaker cache step remains.
    """
    steps = _workflow_steps()
    make_installs = _make_install_routes(steps)
    assert not make_installs, (
        f"CI installs through the shared action, not Make: {make_installs}"
    )
    routes = _whitaker_routes(steps)
    assert not routes, f"a script installs or runs Whitaker by hand: {routes}"
    caches = _step_labels(
        steps,
        lambda step: str(step.get("uses", "")).startswith("actions/cache")
        and "whitaker" in str(step.get("with", {})).lower(),
    )
    assert not caches, f"a repository-owned Whitaker cache step remains: {caches}"


def test_no_workflow_declares_a_whitaker_version_token() -> None:
    """Installer and suite pins belong to the Makefile or the action."""
    text = "\n".join(
        path.read_text(encoding="utf-8")
        for path in WORKFLOWS_DIRECTORY.glob("*.y*ml")
    )
    for token in (*WHITAKER_PIN_TOKENS, "suite-version", "installer-version"):
        assert token not in text, (
            f"{token} belongs to the Makefile or the action, not a workflow"
        )


@pytest.mark.parametrize(
    "run",
    [
        "whitaker --all",
        "whitaker-ls",
        "whitaker-installer",
        "./target/debug/whitaker --all",
        "cargo dylint --all",
        "cargo install cargo-dylint",
    ],
)
def test_the_whitaker_route_contract_rejects_a_manual_invocation(run: str) -> None:
    """Every handwritten route is reported, including a bare ``whitaker``.

    The workflow contract is only as strong as its detector. A step added as
    ``whitaker --all`` must be caught, not accepted because only the installer
    and ``cargo dylint`` spellings were recognized, and a path-qualified
    ``./target/debug/whitaker`` must not slip past a lookbehind that rejects a
    leading slash.
    """
    steps = [("ci.yml", "build-test", {"name": "Lint", "run": run})]
    assert _whitaker_routes(steps) == ["ci.yml:build-test: Lint"]


def test_the_real_install_step_is_not_another_route() -> None:
    """The sanctioned shared-action step is neither a manual nor a Make route."""
    steps = _install_steps(_workflow_steps())
    assert len(steps) == 1, f"expected the one install step, found {len(steps)}"
    assert _whitaker_routes(steps) == [], "the install step reads as a manual route"
    assert _make_install_routes(steps) == [], "the install step reads as a Make route"


@pytest.mark.parametrize(
    "run",
    [
        "make install-whitaker",
        "make -s install-whitaker",
        "set -e\nmake -s \\\n  install-whitaker\nmake lint",
        "cd . && make lint && make install-whitaker",
        "env X=1 make install-whitaker",
        "MAKEFLAGS=-s make install-whitaker",
    ],
)
def test_the_contract_reports_a_make_install_step(run: str) -> None:
    """A Make install step in any workflow job is reported by label."""
    steps = [("ci.yml", "build-test", {"name": "Reinstall", "run": run})]
    assert _make_install_routes(steps) == ["ci.yml:build-test: Reinstall"]


def test_the_contract_leaves_a_lint_step_alone() -> None:
    """A step that runs other targets, or only names the target, is no route."""
    steps = [
        ("ci.yml", "build-test", {"name": "Lint", "run": "make lint"}),
        (
            "ci.yml",
            "build-test",
            {"name": "Note", "run": "echo 'make install-whitaker'"},
        ),
    ]
    assert _make_install_routes(steps) == []


@pytest.mark.parametrize(
    "uses",
    [
        "leynos/shared-actions/.github/actions/install-whitaker@main",
        "leynos/shared-actions/.github/actions/install-whitaker",
        "leynos/shared-actions/.github/actions/install-whitaker@" + "0" * 39,
    ],
)
def test_the_install_action_must_be_pinned_to_a_full_commit(uses: str) -> None:
    """A mutable or short reference does not satisfy the install pattern."""
    assert INSTALL_WHITAKER_ACTION_RE.fullmatch(uses) is None


def test_whitaker_pins_live_only_in_the_makefile() -> None:
    """The Makefile is the one reviewed location for every Whitaker pin."""
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    for token in WHITAKER_PIN_TOKENS:
        assert makefile.count(f"{token} ?=") == 1, (
            f"{token} must be assigned exactly once in the Makefile"
        )
