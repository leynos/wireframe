"""Hold CI to running doctests, and to failing when they fail.

``make test`` selects ``--all-targets``, which excludes doctests, and
cargo-nextest cannot run them, so one workflow step is all that stands between
a documented example and silent rot (#578). A workflow can keep that step and
still lose the gate, through a condition, ``continue-on-error``, or a recipe
that swallows the exit status, so each of those is checked here.

The recipe is executed rather than read. Reading it could not notice a ``-``
prefix or an ``|| true`` spelled some new way, whereas a stand-in ``cargo`` that
fails shows what the gate does with a real failure, without compiling anything.
The end-to-end check runs the workflow step's own command with that stand-in,
so deleting the step or pointing it elsewhere fails the contract.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import stat
import subprocess
from pathlib import Path

import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
DOCTEST_RUN = "make test-doc"
COMPANION_TEST_RUN = "make test-wireframe-testing"
FAILING_EXIT_STATUS = 101


def _build_test_steps() -> list[dict[str, object]]:
    """Return the ``build-test`` job's steps."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    steps = workflow["jobs"]["build-test"]["steps"]
    return list(steps)


def _steps_running(command: str) -> list[dict[str, object]]:
    """Return every ``build-test`` step that runs ``command``."""
    return [step for step in _build_test_steps() if step.get("run") == command]


def _failing_cargo(tmp_path: Path) -> tuple[Path, Path]:
    """Create a stand-in ``cargo`` that records its arguments and fails."""
    record = tmp_path / "cargo-arguments"
    cargo = tmp_path / "cargo"
    cargo.write_text(
        f'#!/bin/sh\nprintf \'%s\\n\' "$@" > "{record}"\nexit {FAILING_EXIT_STATUS}\n',
        encoding="utf-8",
    )
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    return cargo, record


def _run_recipe_with_failing_cargo(tmp_path: Path) -> tuple[int, list[str]]:
    """Run ``make test-doc`` on a failing ``cargo``; return status and arguments."""
    cargo, record = _failing_cargo(tmp_path)
    completed = subprocess.run(
        ["make", "--no-print-directory", "test-doc", f"CARGO={cargo}"],
        cwd=REPOSITORY_ROOT,
        env=os.environ.copy(),
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode, record.read_text(encoding="utf-8").splitlines()


def _run_workflow_step_with_failing_cargo(tmp_path: Path) -> int:
    """Run the workflow's doctest step command with a failing ``cargo``."""
    steps = _steps_running(DOCTEST_RUN)
    assert len(steps) == 1, f"build-test must run `{DOCTEST_RUN}` exactly once"
    command = steps[0]["run"]
    assert isinstance(command, str) and command, "the doctest step must run a command"
    cargo, _ = _failing_cargo(tmp_path)
    environment = os.environ.copy()
    environment["CARGO"] = str(cargo)
    completed = subprocess.run(
        ["bash", "-c", command],
        cwd=REPOSITORY_ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode


def test_build_test_runs_doctests_unconditionally() -> None:
    """Exactly one step runs doctests, and nothing lets it be skipped or ignored."""
    steps = _steps_running(DOCTEST_RUN)
    assert len(steps) == 1, f"build-test must run `{DOCTEST_RUN}` exactly once"
    step = steps[0]
    assert "if" not in step, "the doctest step must run for every event"
    assert not step.get("continue-on-error"), "a doctest failure must fail the job"


def test_build_test_runs_the_companion_crate_tests() -> None:
    """The explicit companion-crate gate runs in the same job (#578)."""
    steps = _steps_running(COMPANION_TEST_RUN)
    assert len(steps) == 1, f"build-test must run `{COMPANION_TEST_RUN}` exactly once"
    step = steps[0]
    assert "if" not in step, "the companion test step must run for every event"
    assert not step.get("continue-on-error"), "a companion failure must fail the job"


def test_a_failing_doctest_run_fails_the_make_target(tmp_path: Path) -> None:
    """The recipe propagates cargo's failure instead of swallowing it."""
    status, _ = _run_recipe_with_failing_cargo(tmp_path)
    assert status != 0, "`make test-doc` must fail when cargo fails"


def test_a_failing_doctest_fails_the_workflow_step(tmp_path: Path) -> None:
    """The workflow's own doctest command fails end to end when cargo fails."""
    status = _run_workflow_step_with_failing_cargo(tmp_path)
    assert status != 0, "the workflow's doctest step must fail when cargo fails"


def test_the_make_target_covers_every_workspace_doctest(tmp_path: Path) -> None:
    """The recipe selects doctests across the workspace with every feature on."""
    _, arguments = _run_recipe_with_failing_cargo(tmp_path)
    for required in ("test", "--workspace", "--doc", "--all-features"):
        assert required in arguments, f"`make test-doc` must pass {required}"
    assert "--exclude" not in arguments, "no crate may be left out of the gate"