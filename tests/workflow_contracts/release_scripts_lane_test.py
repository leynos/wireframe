"""Hold CI to testing the scripts a release depends on.

A release runs rarely, so a script that broke since the last one would be found
on release day. These contracts keep the scripts' tests in the blocking lane.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
COMMAND = "make test-release-scripts"


def test_build_test_runs_the_release_script_tests_unconditionally() -> None:
    """Exactly one step runs them, and nothing lets it be skipped or ignored."""
    workflow = yaml.safe_load(CI_WORKFLOW.read_text(encoding="utf-8"))
    steps = workflow["jobs"]["build-test"]["steps"]
    matching = [step for step in steps if step.get("run") == COMMAND]
    assert len(matching) == 1, f"build-test must run `{COMMAND}` exactly once"
    assert "if" not in matching[0], "the step must run for every event"
    assert not matching[0].get("continue-on-error"), "a failure must fail the job"


def test_the_target_runs_every_test_and_doctest() -> None:
    """The target collects the whole directory, with doctests, on Python 3.14."""
    command = subprocess.run(
        ["make", "--no-print-directory", "--dry-run", "test-release-scripts"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert "--python 3.14" in command
    assert command.rstrip().endswith("--doctest-modules scripts/release")
