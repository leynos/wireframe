"""Contract tests for the isolated feature-gate lane.

``tests/feature_gate`` is a standalone Cargo package, so no workspace-wide
target builds it: ``make test`` and ``make test-ui`` both pass while the
consumer is broken, and a green lane would say nothing about the gate. These
tests hold the blocking lane to running it as an explicit step, hold that
step to failing the job, and hold the gate's premise — that the consumer
stays outside the workspace — so the check cannot become a tautology.

The step's command is tokenized rather than searched as text, so what is
asserted is what the shell would run: ``echo make test-feature-gate`` and a
command chained after the target keep every substring a text search looks
for and run nothing.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import shlex
import subprocess
import tomllib
import typing as typ
from pathlib import Path

from workflow_loader import repository_workflows

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
CONSUMER: typ.Final = REPO_ROOT / "tests" / "feature_gate"

#: The Make target the lane calls, and the script that target runs.
GATE_TARGET: typ.Final = "test-feature-gate"
GATE_SCRIPT: typ.Final = "./scripts/check-feature-gate.sh"


def _tokens(command: str) -> list[str]:
    """Split a shell command into words, with operators as their own tokens.

    ``punctuation_chars`` separates ``;``, ``|`` and ``&`` even when they
    touch a word, so ``make test-feature-gate;echo`` yields ``;`` rather
    than hiding it inside the target name.

    Parameters
    ----------
    command
        One workflow step's ``run`` text.

    Returns
    -------
    list[str]
        The command's words, with shell operators as separate tokens.

    Examples
    --------
    >>> _tokens("make test-feature-gate")
    ['make', 'test-feature-gate']
    """
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    return list(lexer)


def _build_test_steps() -> list[dict[str, object]]:
    """Return the steps of ``ci.yml``'s blocking ``build-test`` job."""
    documents = repository_workflows()
    assert "ci.yml" in documents, "the blocking lane lives in ci.yml"
    jobs = documents["ci.yml"].get("jobs")
    assert isinstance(jobs, dict), "ci.yml must declare a jobs mapping"
    job = jobs.get("build-test")
    assert isinstance(job, dict), "ci.yml must declare the build-test job"
    steps = job.get("steps")
    assert isinstance(steps, list), "build-test must declare steps"
    return [step for step in steps if isinstance(step, dict)]


def _gate_step() -> dict[str, object]:
    """Return the one build-test step whose command names the gate target."""
    gate_steps = [
        step
        for step in _build_test_steps()
        if GATE_TARGET in _tokens(str(step.get("run", "")))
    ]
    assert len(gate_steps) == 1, (
        f"exactly one step should run {GATE_TARGET}; found {len(gate_steps)}"
    )
    return gate_steps[0]


def test_the_lane_runs_the_feature_gate_as_its_own_command() -> None:
    """Scenario: the step is edited to run something else, or nothing.

    Invariant: exactly one step runs ``make test-feature-gate``, as its whole
    command. Comparing the tokenized command rather than searching it for the
    target name refuses ``echo make test-feature-gate`` and any command
    chained after the target.
    """
    command = _tokens(str(_gate_step()["run"]))
    assert command == ["make", GATE_TARGET], (
        f"the feature-gate step must run exactly `make {GATE_TARGET}`; "
        f"it runs {command}"
    )


def test_the_feature_gate_step_cannot_fail_softly() -> None:
    """Scenario: the step is skipped, or its failure does not fail the job.

    Invariant: the step declares neither ``if`` nor ``continue-on-error``.
    Either one leaves the lane green while the consumer does not compile,
    which is the only reason the gate gets its own step.
    """
    step = _gate_step()
    assert "if" not in step, (
        "the feature-gate step must run on every lane invocation; it is "
        f"conditional on {step['if']!r}"
    )
    assert "continue-on-error" not in step, (
        "the feature-gate step must fail the job; it sets continue-on-error "
        f"to {step['continue-on-error']!r}"
    )


def _gate_target_command() -> list[str]:
    """Return the command ``make test-feature-gate`` runs.

    The recipe is read through ``make --dry-run``, so a variable, a line
    continuation or an included file resolves exactly as it would when the
    lane runs.
    """
    printed = subprocess.run(
        ["make", "--no-print-directory", "--dry-run", GATE_TARGET],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    lines = [line for line in printed.splitlines() if line.strip()]
    assert len(lines) == 1, (
        f"{GATE_TARGET} should run one command; it prints {lines}"
    )
    return _tokens(lines[0])


def test_the_target_runs_the_check_script() -> None:
    """Scenario: the target is emptied, or runs something other than the gate.

    Invariant: the target runs exactly the check script. A recipe that prints
    a success message and exits keeps the script's name in the Makefile and
    checks nothing.
    """
    command = _gate_target_command()
    assert command == [GATE_SCRIPT], (
        f"{GATE_TARGET} must run exactly `{GATE_SCRIPT}`; it runs {command}"
    )


def test_the_consumer_stays_outside_the_workspace() -> None:
    """Scenario: the consumer joins the workspace, or loses its own boundary.

    Invariant: the consumer declares an empty ``[workspace]`` table and the
    root workspace does not list it as a member. Either change lets Cargo
    unify the consumer's features with the root's dev-dependencies, which
    enable ``wireframe/test-support``; the expected compile failure then
    disappears while the gate still reports success.
    """
    manifest = tomllib.loads((CONSUMER / "Cargo.toml").read_text("utf-8"))
    assert manifest.get("workspace") == {}, (
        "tests/feature_gate must declare an empty [workspace] table so Cargo "
        "does not walk up to the root manifest; found "
        f"{manifest.get('workspace')!r}"
    )
    root = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text("utf-8"))
    members = root.get("workspace", {}).get("members", [])
    assert "tests/feature_gate" not in members, (
        "tests/feature_gate must stay outside the root workspace; members "
        f"list {members}"
    )
