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
MAKE_TARGET_RE = re.compile(r"\bmake\s+(?:-\S+\s+)*([\w-]+)")


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
    gates = [
        match.start()
        for match in MAKE_TARGET_RE.finditer(run)
        if match.group(1) in GATE_TARGETS
    ]
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
