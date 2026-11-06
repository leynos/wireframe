"""Hold CI to running doctests, and to failing when they fail.

``make test`` selects ``--all-targets``, which excludes doctests, and
cargo-nextest cannot run them, so one workflow step is all that stands between
a documented example and silent rot (#578). A workflow can keep that step and
still lose the gate, through a condition, ``continue-on-error``, or a recipe
that swallows the exit status, so each of those is checked here.

The recipe is executed rather than read. Reading it could not notice a ``-``
prefix or an ``|| true`` spelled some new way, whereas a stand-in ``cargo`` that
fails shows what the gate does with a real failure, without compiling anything.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import stat
import subprocess
from pathlib import Path

import pytest
import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
DOCTEST_RUN = "make test-doc"
DOCUMENTATION_CONTRACTS_RUN = "make test-documentation-contracts"
FAILING_EXIT_STATUS = 101


def _steps_running(run: str) -> list[dict[str, object]]:
    """Return every ``build-test`` step whose command is ``run``."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    steps = workflow["jobs"]["build-test"]["steps"]
    return [step for step in steps if step.get("run") == run]


def _run_recipe_with_failing_cargo(tmp_path: Path) -> tuple[int, list[str]]:
    """Run ``make test-doc`` on a failing ``cargo``; return status and arguments."""
    record = tmp_path / "cargo-arguments"
    cargo = tmp_path / "cargo"
    cargo.write_text(
        f'#!/bin/sh\nprintf \'%s\\n\' "$@" > "{record}"\nexit {FAILING_EXIT_STATUS}\n',
        encoding="utf-8",
    )
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    completed = subprocess.run(
        ["make", "--no-print-directory", "test-doc", f"CARGO={cargo}"],
        cwd=REPOSITORY_ROOT,
        env=os.environ.copy(),
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode, record.read_text(encoding="utf-8").splitlines()


@pytest.mark.parametrize("run", [DOCTEST_RUN, DOCUMENTATION_CONTRACTS_RUN])
def test_build_test_runs_documentation_gates_unconditionally(run: str) -> None:
    """Exactly one step runs each gate, and nothing lets it be skipped or ignored."""
    steps = _steps_running(run)
    assert len(steps) == 1, f"build-test must run `{run}` exactly once"
    step = steps[0]
    assert "if" not in step, f"`{run}` must run for every event"
    assert not step.get("continue-on-error"), f"a `{run}` failure must fail the job"


def test_documentation_contracts_build_the_examples_they_run() -> None:
    """The end-to-end contract is feature-gated, so its target must enable it."""
    dry_run = subprocess.run(
        ["make", "--no-print-directory", "-n", "test-documentation-contracts"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert "build --examples --all-features" in dry_run
    assert "--test documentation_examples_tests --all-features" in dry_run


def test_a_failing_doctest_run_fails_the_make_target(tmp_path: Path) -> None:
    """The recipe propagates cargo's failure instead of swallowing it."""
    status, _ = _run_recipe_with_failing_cargo(tmp_path)
    assert status != 0, "`make test-doc` must fail when cargo fails"


def test_the_make_target_covers_every_workspace_doctest(tmp_path: Path) -> None:
    """The recipe selects doctests across the workspace with every feature on."""
    _, arguments = _run_recipe_with_failing_cargo(tmp_path)
    for required in ("test", "--workspace", "--doc", "--all-features"):
        assert required in arguments, f"`make test-doc` must pass {required}"
    assert "--exclude" not in arguments, "no crate may be left out of the gate"
