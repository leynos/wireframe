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
#: A word that is not a target: nothing at all, an option, or a `NAME=value`.
NOT_A_TARGET_RE = re.compile(r"^$|^-|=")


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
        if NOT_A_TARGET_RE.search(word):
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


def _installs_the_linker(step: WorkflowStep) -> bool:
    """Report whether a step installs `mold`, by apt or the setup-rust input."""
    return _first_positions(step)[0] is not None


def _runs_coverage_before_the_linker(job: dict) -> bool:
    """Report whether a job runs `generate-coverage` before installing `mold`.

    Coverage builds Rust through the action, not through a `make` gate target,
    so the ordering check above cannot see it. Its nested cargo runs (the
    trybuild case) read `.cargo/config.toml` and link with `mold`.
    """
    for step in job.get("steps") or []:
        if "generate-coverage" in str(step.get("uses", "")):
            return True
        if _installs_the_linker(step):
            return False
    return False


def _coverage_jobs_missing_the_linker() -> list[str]:
    """Return the Linux jobs that run coverage without installing `mold` first."""
    return [
        name for name, job in _linux_jobs() if _runs_coverage_before_the_linker(job)
    ]


def test_coverage_jobs_install_the_linker_before_generating_coverage() -> None:
    """A coverage lane without `mold` fails the trybuild link on the runner."""
    missing = _coverage_jobs_missing_the_linker()
    assert missing == [], f"jobs run coverage before installing `mold`: {missing}"


def test_the_main_coverage_lane_is_held_to_the_linker_contract() -> None:
    """The check covers `coverage-main.yml`, so it cannot pass by omission."""
    names = {
        name
        for name, job in _linux_jobs()
        if any("generate-coverage" in str(s.get("uses", "")) for s in job.get("steps") or [])
    }
    assert "coverage-main.yml:coverage-upload" in names


@pytest.mark.parametrize(
    ("steps", "unsafe"),
    [
        ([{"uses": "o/shared-actions/.github/actions/generate-coverage@x"}], True),
        (
            [
                {"uses": "o/shared-actions/.github/actions/setup-rust@x"},
                {"uses": "o/shared-actions/.github/actions/generate-coverage@x"},
            ],
            True,
        ),
        (
            [
                {
                    "uses": "o/shared-actions/.github/actions/setup-rust@x",
                    "with": {"install-mold": "true"},
                },
                {"uses": "o/shared-actions/.github/actions/generate-coverage@x"},
            ],
            False,
        ),
        (
            [
                {"run": "sudo apt-get install -y mold"},
                {"uses": "o/shared-actions/.github/actions/generate-coverage@x"},
            ],
            False,
        ),
    ],
    ids=["bare", "setup-rust-without-mold", "input", "apt"],
)
def test_the_coverage_ordering_check_reads_each_way_to_install_the_linker(
    steps: list[WorkflowStep], unsafe: bool
) -> None:
    """Only an install before the coverage step makes the job safe."""
    assert _runs_coverage_before_the_linker({"steps": steps}) is unsafe


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


def _uv_cache_problem(step: WorkflowStep) -> str | None:
    """Return why a `setup-uv` step's cache setting is not a stated policy.

    The action's default turns caching on and warns when its dependency globs
    match nothing, so a step must either disable the cache or name the files it
    keys on. Anything else, including an absent or non-boolean setting, fails.
    """
    inputs = step.get("with") or {}
    enabled = inputs.get("enable-cache")
    if enabled is False:
        return None
    if enabled is True and inputs.get("cache-dependency-glob"):
        return None
    return (
        f"setup-uv step {step.get('name', '?')!r} must set `enable-cache: false`, "
        "or `enable-cache: true` with a `cache-dependency-glob`; "
        f"found enable-cache={enabled!r}"
    )


def _uv_steps() -> list[WorkflowStep]:
    """Return every `setup-uv` step of every workflow in the repository."""
    steps = []
    for path in sorted((ROOT / ".github" / "workflows").glob("*.y*ml")):
        workflow = yaml.safe_load(path.read_text("utf-8")) or {}
        for job in (workflow.get("jobs") or {}).values():
            steps.extend(
                step
                for step in job.get("steps") or []
                if str(step.get("uses", "")).startswith("astral-sh/setup-uv@")
            )
    return steps


def test_every_setup_uv_step_states_its_cache_policy() -> None:
    """No workflow leaves `setup-uv` on its default cache, which warns when the
    dependency globs match nothing."""
    steps = _uv_steps()
    assert steps, "no setup-uv step found to hold to the policy"
    problems = [problem for step in steps if (problem := _uv_cache_problem(step))]
    assert problems == [], problems


@pytest.mark.parametrize(
    ("inputs", "accepted"),
    [
        ({"enable-cache": False}, True),
        ({"enable-cache": True, "cache-dependency-glob": "uv.lock"}, True),
        ({"enable-cache": True}, False),
        ({"enable-cache": "auto"}, False),
        ({"enable-cache": "false"}, False),
        ({}, False),
        ({"python-version": "3.13"}, False),
    ],
)
def test_the_uv_cache_policy_accepts_only_stated_forms(
    inputs: dict[str, object], accepted: bool
) -> None:
    """Disabled, or enabled with a dependency glob; every other form fails."""
    step: WorkflowStep = {"name": "uv", "uses": "astral-sh/setup-uv@x", "with": inputs}
    assert (_uv_cache_problem(step) is None) is accepted
