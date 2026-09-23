"""Contract tests for the scheduled Loom lane.

The lane failed to compile for months while its name said it model-checked
the push queue, and a lane that only compiles its models reads the same from
the outside as one that runs them. These tests hold the two halves of the
lane to what they must do: the workflow step calls ``make test-loom``, and
that target runs the Loom package's tests under ``--cfg loom`` with a bounded
preemption count.

The target is read through ``make --dry-run`` and tokenized, not searched as
text, so what is asserted is the command Make would run: the program, its
arguments and the environment it is given. A step rewritten as ``echo make
test-loom``, or a target piped into another command, keeps every substring a
text search would look for and runs nothing.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import shlex
import subprocess
import typing as typ
from pathlib import Path

import yaml

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_PATH: typ.Final = REPO_ROOT / ".github" / "workflows" / "advanced-tests.yml"

#: The Make target the lane calls, and the package holding the models.
LOOM_TARGET: typ.Final = "test-loom"
LOOM_PACKAGE: typ.Final = "wireframe-loom"

#: Shell operators that would hand the status to another command or chain
#: a second one after the models.
SHELL_OPERATORS: typ.Final = frozenset({"|", "||", "&&", ";", "&"})

#: The outer bound for the step, in minutes. Loom has no clock, so a model
#: that cannot finish hangs rather than fails; the step's timeout is what
#: turns that into a red run.
MAX_STEP_MINUTES: typ.Final = 30


def _tokens(command: str) -> list[str]:
    """Split a shell command into words, with operators as their own tokens.

    ``punctuation_chars`` separates ``;``, ``|`` and ``&`` even when they
    touch a word, so ``cargo test;echo`` yields ``;`` rather than hiding it
    inside ``test;echo``.
    """
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    return list(lexer)


def _steps() -> list[dict[str, object]]:
    """Return the steps of the lane's one job."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    jobs = workflow.get("jobs")
    assert isinstance(jobs, dict), "the workflow must declare a jobs mapping"
    assert len(jobs) == 1, f"the Loom lane should hold one job; found {list(jobs)}"
    (job,) = jobs.values()
    steps = job.get("steps")
    assert isinstance(steps, list), "the Loom job must declare steps"
    return [step for step in steps if isinstance(step, dict)]


def _loom_command() -> tuple[dict[str, str], list[str]]:
    """Return the environment and argument list ``make test-loom`` would run.

    The recipe is read through ``make --dry-run``, so a variable, a line
    continuation or an included file resolves exactly as it would when the
    lane runs.
    """
    printed = subprocess.run(
        ["make", "--no-print-directory", "--dry-run", LOOM_TARGET],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    lines = [line for line in printed.splitlines() if line.strip()]
    assert len(lines) == 1, (
        f"{LOOM_TARGET} should run exactly one command; it prints {lines}"
    )
    tokens = _tokens(lines[0])
    environment: dict[str, str] = {}
    while tokens and "=" in tokens[0] and not tokens[0].startswith("-"):
        name, _, value = tokens.pop(0).partition("=")
        environment[name] = value
    return environment, tokens


def test_the_lane_calls_the_loom_target() -> None:
    """Scenario: the step is edited to run something else, or nothing.

    Invariant: exactly one step runs, as its whole command,
    ``make test-loom``. Comparing the tokenized command rather than
    searching it for the target name refuses ``echo make test-loom`` and a
    command chained after it.
    """
    loom_steps = [
        step for step in _steps() if LOOM_TARGET in _tokens(str(step.get("run", "")))
    ]
    assert len(loom_steps) == 1, (
        f"exactly one step should run {LOOM_TARGET}; found {len(loom_steps)}"
    )
    command = _tokens(str(loom_steps[0]["run"]))
    assert command == ["make", LOOM_TARGET], (
        f"the Loom step must run exactly `make {LOOM_TARGET}`; it runs {command}"
    )


def test_the_loom_step_is_bounded() -> None:
    """Scenario: a model that cannot finish hangs the lane for six hours.

    Invariant: the Loom step carries its own ``timeout-minutes`` of at most
    thirty. Loom explores interleavings, not durations, so a model with an
    interleaving that never completes hangs rather than failing; the step's
    timeout is what reports it.
    """
    (step,) = [
        step
        for step in _steps()
        if str(step.get("run", "")).strip() == f"make {LOOM_TARGET}"
    ]
    minutes = step.get("timeout-minutes")
    assert isinstance(minutes, int) and 0 < minutes <= MAX_STEP_MINUTES, (
        f"the Loom step must set timeout-minutes between 1 and "
        f"{MAX_STEP_MINUTES}; got {minutes!r}"
    )


def test_the_target_runs_the_models_rather_than_compiling_them() -> None:
    """Scenario: the target compiles the models, or runs the wrong package.

    Invariant: the target runs ``cargo test -p wireframe-loom`` with no
    ``--no-run``, and with no shell operator after it. A compile-only lane
    was the state this contract exists to end.
    """
    _, argv = _loom_command()
    assert argv[:2] == ["cargo", "test"], (
        f"{LOOM_TARGET} must run `cargo test`; it runs {argv}"
    )
    assert "--no-run" not in argv, (
        f"{LOOM_TARGET} must run the models, not only compile them: {argv}"
    )
    assert "-p" in argv and argv[argv.index("-p") + 1] == LOOM_PACKAGE, (
        f"{LOOM_TARGET} must select the {LOOM_PACKAGE} package; it runs {argv}"
    )
    operators = sorted(SHELL_OPERATORS.intersection(argv))
    assert not operators, (
        f"{LOOM_TARGET} must end at the test run, so its status is the "
        f"lane's; found {operators} in {argv}"
    )


def test_the_target_selects_the_loom_configuration() -> None:
    """Scenario: ``--cfg loom`` is dropped from the target.

    Invariant: the target's ``RUSTFLAGS`` is exactly ``--cfg loom``. The
    model files compile to nothing without it, so the run would report zero
    tests and pass.
    """
    environment, _ = _loom_command()
    assert shlex.split(environment.get("RUSTFLAGS", "")) == ["--cfg", "loom"], (
        f"{LOOM_TARGET} must set RUSTFLAGS to --cfg loom; got "
        f"{environment.get('RUSTFLAGS')!r}"
    )


def test_the_target_bounds_loom_exploration() -> None:
    """Scenario: the preemption bound is removed or zeroed.

    Invariant: ``LOOM_MAX_PREEMPTIONS`` is set to a positive integer. The
    bound is not a performance knob: an unbounded exploration does not fail
    a model, it fails to finish, and zero preemptions explores almost
    nothing.
    """
    environment, _ = _loom_command()
    bound = environment.get("LOOM_MAX_PREEMPTIONS", "")
    assert bound.isdigit() and int(bound) > 0, (
        f"{LOOM_TARGET} must set LOOM_MAX_PREEMPTIONS to a positive integer; "
        f"got {bound!r}"
    )
