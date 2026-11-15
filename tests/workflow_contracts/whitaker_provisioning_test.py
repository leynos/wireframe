"""Protect the workflow-wide Whitaker provisioning contract.

CI installs Whitaker through exactly one shared ``install-whitaker`` action
step, which owns the installer version and takes no pin, and no workflow
script, ``make install-whitaker`` step or version variable provisions it
another way; the Makefile's pins serve contributors only. That check reads
every job and step of every workflow file, so a route added under a new step
name or in a new workflow still fails it. How a script is read for a Make
install lives in ``make_route_reader``, and the judgement of the install step
itself in ``whitaker_install_policy``.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import cast

import pytest
import yaml
from make_route_reader import runs_make_install_whitaker
from whitaker_install_policy import (
    LocatedStep,
    Step,
    install_problems,
    install_steps,
    step_labels,
)

WORKFLOW_PATH = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
MAKEFILE_PATH = Path(__file__).resolve().parents[2] / "Makefile"
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


def _workflow_steps() -> list[LocatedStep]:
    """Return (workflow file, job, step) for every step in every workflow."""
    found: list[LocatedStep] = []
    for path in sorted(WORKFLOWS_DIRECTORY.glob("*.y*ml")):
        # An empty workflow file parses to None; treat it as having no jobs.
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        jobs = cast("dict[str, dict[str, object]]", document.get("jobs") or {})
        for job_name, job in jobs.items():
            for step in cast("list[Step]", job.get("steps") or []):
                found.append((path.name, job_name, step))
    return found


def _build_test_steps() -> list[Step]:
    """Return the steps of `ci.yml`'s `build-test` job."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    return cast("list[Step]", workflow["jobs"]["build-test"]["steps"])


def _whitaker_routes(steps: list[LocatedStep]) -> list[str]:
    """Return ``file:job: step`` for each step that runs Whitaker by hand."""
    return step_labels(
        steps,
        lambda step: WHITAKER_ROUTE_RE.search(str(step.get("run", ""))) is not None,
    )


def _make_install_routes(steps: list[LocatedStep]) -> list[str]:
    """Return ``file:job: step`` for each step that runs ``make install-whitaker``."""
    return step_labels(
        steps, lambda step: runs_make_install_whitaker(str(step.get("run", "")))
    )


def test_whitaker_has_one_shared_action_install_before_lint() -> None:
    """Exactly one step installs Whitaker, through the shared action, in order.

    Invariant, over every job and step of every workflow: one shared
    ``install-whitaker`` action step, at a full commit SHA, in ci.yml's
    ``build-test`` job before the exact ``make lint`` step, passing no pin. The
    count is asserted first, because a contract over no steps is satisfied by
    deleting the install.
    """
    problems = install_problems(_workflow_steps(), _build_test_steps())
    assert not problems, problems


def test_no_other_route_installs_or_runs_whitaker_in_ci() -> None:
    """No Make install, script or cache step is another route.

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
    caches = step_labels(
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


def test_the_action_path_in_a_script_is_not_a_manual_route() -> None:
    """A script that only names the action path is exempt from the scan.

    The scanner reads ``run`` text, and a bare ``whitaker`` is a manual route.
    The ``install-whitaker`` path is exempt only because the pattern refuses a
    hyphen before the name; the final assertion shows that a pattern without
    that lookbehind would report this script, so removing the lookbehind makes
    this test fail.
    """
    run = "echo leynos/shared-actions/.github/actions/install-whitaker@" + "0" * 40
    steps = [("ci.yml", "build-test", {"name": "Note", "run": run})]
    assert _whitaker_routes(steps) == [], "the action path reads as a manual route"
    assert re.search(r"whitaker(?:-ls)?\b", run), (
        "the script must contain the bare name, or the lookbehind is untested"
    )


def test_the_real_install_step_is_not_another_route() -> None:
    """The sanctioned shared-action step is neither a manual nor a Make route."""
    steps = install_steps(_workflow_steps())
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
        "make install-whitaker && make lint",
        "echo ok\nmake install-whitaker\nmake lint",
        "make lint\nmake install-whitaker",
        "env -i make install-whitaker",
        "sudo -n make install-whitaker",
        "echo x#y; make install-whitaker",
        "if make install-whitaker; then :; fi",
        'echo "$(make install-whitaker)"',
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


def test_whitaker_pins_live_only_in_the_makefile() -> None:
    """The Makefile is the one reviewed location for every Whitaker pin."""
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    for token in WHITAKER_PIN_TOKENS:
        assert makefile.count(f"{token} ?=") == 1, (
            f"{token} must be assigned exactly once in the Makefile"
        )
