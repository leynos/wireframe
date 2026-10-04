"""Contract tests for where CI installs `mold` relative to the gate commands.

The Makefile restates the build standard's `mold` flag for its gate targets,
so every Linux job must install `mold` before the first gate target runs.
The Makefile and configuration clauses live in ``build_standard_test.py``.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import TypedDict

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
#: Make targets whose CI invocation builds Rust, so the job needs `mold`.
GATE_TARGETS = {
    "test",
    "lint",
    "typecheck",
    "build",
    "all",
    "dev-build",
    "dev-test",
    "test-bdd",
    "test-doc",
}
#: The word after each `make`. This contract judges the invocations this
#: repository's workflows use, which are `make <target>`; any other form (an
#: option, or a `NAME=value` assignment before the target) is an error, since it
#: would hide the target from a pattern that guessed at it.
MAKE_WORD_RE = re.compile(r"\bmake\b[ \t]*([^\s;&|]*)")
TARGET_RE = re.compile(r"[\w-]+")


class UnrecognizedMakeError(ValueError):
    """A `make` invocation in a workflow step that this contract cannot read."""


def _make_targets(run: str) -> list[tuple[int, str]]:
    """Return each `make <target>` in a step's ``run`` text, with its offset.

    Raises:
        UnrecognizedMakeError: for `make` followed by an option, a variable
            assignment, or nothing, rather than guessing at the target.
    """
    targets = []
    for match in MAKE_WORD_RE.finditer(run):
        word = match.group(1)
        if not word or word.startswith("-") or "=" in word:
            raise UnrecognizedMakeError(
                f"unrecognized `make` invocation at offset {match.start()}: "
                f"{match.group(0)!r}; only `make <target>` is recognized"
            )
        target = TARGET_RE.match(word)
        if target:
            targets.append((match.start(), target.group(0)))
    return targets


#: The fields of a workflow step this contract reads. Functional syntax,
#: because ``with`` is a keyword.
WorkflowStep = TypedDict(
    "WorkflowStep",
    {
        "name": str,
        "run": str,
        "uses": str,
        "env": dict[str, str],
        "with": dict[str, object],
    },
    total=False,
)


def _linux_jobs() -> list[tuple[str, dict]]:
    """Return every CI job not placed on Windows or macOS, named by file."""
    jobs = []
    for path in sorted((ROOT / ".github" / "workflows").glob("*.y*ml")):
        workflow = yaml.safe_load(path.read_text("utf-8")) or {}
        jobs.extend(
            (f"{path.name}:{name}", job)
            for name, job in (workflow.get("jobs") or {}).items()
            if not re.search(
                r"windows|macos", str(job.get("runs-on", "")), re.IGNORECASE
            )
        )
    return jobs


def _first_positions(step: WorkflowStep) -> tuple[int | None, int | None]:
    """Return where a step first installs `mold` and first runs a gate target.

    Positions are offsets into the step's ``run`` text; a setup-rust step that
    installs `mold` through its input counts as installing at offset 0.
    """
    run = str(step.get("run", ""))
    inputs = step.get("with") or {}
    installs = [
        match.start()
        for match in re.finditer(r"apt(-get)?\s+install[^\n]*\bmold\b", run)
    ]
    if "setup-rust" in str(step.get("uses", "")) and (
        str(inputs.get("install-mold", "")).lower() == "true"
    ):
        installs.append(0)
    gates = [at for at, target in _make_targets(run) if target in GATE_TARGETS]
    return min(installs, default=None), min(gates, default=None)


def _gate_precedes_install(install_at: int | None, gate_at: int | None) -> bool:
    """Report whether a step's first gate target comes before its `mold` install."""
    if gate_at is None:
        return False
    return install_at is None or gate_at < install_at


def _runs_a_gate_target_first(job: dict) -> bool:
    """Report whether a job runs a gate target before any step installs `mold`,
    comparing positions within a step that does both."""
    for step in job.get("steps") or []:
        install_at, gate_at = _first_positions(step)
        if _gate_precedes_install(install_at, gate_at):
            return True
        if install_at is not None:
            return False
    return False


def _jobs_missing_the_linker() -> list[str]:
    """Return the Linux CI jobs that run a gate target without installing
    `mold` first."""
    return [name for name, job in _linux_jobs() if _runs_a_gate_target_first(job)]


def test_ci_installs_the_linker_before_gate_targets() -> None:
    """The gate targets restate `mold`, so a Linux job must install it first."""
    missing = _jobs_missing_the_linker()
    assert missing == [], f"jobs run a gate target before installing `mold`: {missing}"


def _step_running(run: str) -> WorkflowStep:
    """Return a step that runs ``run``, for the reader's own tests."""
    return {"name": "step", "run": run}


@pytest.mark.parametrize(
    "run",
    [
        "make BUILD_JOBS=-j4 test",
        "make -j 4 test",
        "make -C subdir test",
        "make --jobs=4 lint",
        "make",
    ],
)
def test_an_unrecognized_make_invocation_is_an_error(run: str) -> None:
    """A form other than `make <target>` fails with a named error instead of
    being read as another target and letting a gate escape the ordering check."""
    with pytest.raises(UnrecognizedMakeError, match="only `make <target>`"):
        _first_positions(_step_running(run))


@pytest.mark.parametrize(
    ("run", "gate_first"),
    [
        ("make test", True),
        ("make lint && make test", True),
        ("sudo apt-get install -y mold && make test", False),
        ("make check-fmt", False),
        ("make markdownlint, then make nixie", False),
    ],
)
def test_the_recognized_make_form_orders_gates_against_the_install(
    run: str, gate_first: bool
) -> None:
    """`make <target>` is read as before, so a gate before the install is caught
    and a non-gate target, or a gate after the install, is not."""
    install_at, gate_at = _first_positions(_step_running(run))
    assert _gate_precedes_install(install_at, gate_at) is gate_first


def test_the_repository_workflows_use_only_the_recognized_make_form() -> None:
    """Every workflow step's `make` invocation reads without error."""
    for _name, job in _linux_jobs():
        for step in job.get("steps") or []:
            _make_targets(str(step.get("run", "")))
