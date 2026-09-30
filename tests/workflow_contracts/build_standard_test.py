"""Contract tests for the Rust build standard.

The standard makes the parallel ``rustc`` frontend the default for every
development build and `mold` the default linker on Linux. Cargo reads both from
``.cargo/config.toml``, but it applies a single ``rustflags`` source rather
than merging them, and an assigned ``RUSTFLAGS`` replaces every source. So the
flags must be repeated in each configuration source, restated wherever the
Makefile assigns ``RUSTFLAGS`` for a development target, and kept out of the
coverage and release recipes, which measure or ship and so stay on the default
flags.

The Makefile clauses run ``make -n`` and read the commands it would run,
rather than the Makefile's text, so a flag lost through a variable or a recipe
edit fails here. Each assigned value is expanded by the shell, with and without
an inherited ``RUSTFLAGS``, exactly as the recipe would expand it. The clauses
run as a Linux host and as a macOS host, because `mold` is added on Linux
alone. The workflow clause checks that every Linux CI job running a gate target
installs `mold` first.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import re
import shlex
import subprocess
import tomllib
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
THREADS_FLAG = "-Zthreads=8"
LINKER_FLAG = "-Clink-arg=-fuse-ld=mold"
LINUX_TABLES = {"x86_64-unknown-linux-gnu", 'cfg(target_os = "linux")'}
#: Whether the pinned toolchain is a nightly. `-Zthreads` is a nightly flag, so
#: on a stable pin the standard is `mold` alone and the frontend flag must
#: appear nowhere.
NIGHTLY = True
RUSTFLAGS_RE = re.compile(r'RUSTFLAGS="([^"]*)"')
#: A caller's own flags, distinct from anything a recipe adds, to prove a
#: recipe composes an exported ``RUSTFLAGS`` with the standard flags rather than
#: replacing either.
INHERITED = "--cfg inherited_from_caller"
#: Words that mark a command whose RUSTFLAGS the contract reads.
#: Whitaker is left out: it runs on its own pinned toolchain without the
#: development flags, which `dev_fast_routing_test.py` holds.
COMMAND_WORDS = ("cargo",)
#: Makefile targets that build for development. A command in one either
#: assigns RUSTFLAGS with the standard flags or assigns none and so takes the
#: configuration's.
DEVELOPMENT_TARGETS = ["test", "typecheck", "lint", "build"]
#: Development targets that must assign RUSTFLAGS in at least one command, so
#: the restatement checks cannot pass by finding nothing to check.
ASSIGNING_TARGETS = ["test", "typecheck", "lint", "build"]
#: Makefile targets that measure or ship, and so must take neither flag.
#: Coverage has no Makefile target here; CI runs it through the shared
#: coverage action under the job's own `RUSTFLAGS`.
HELD_OUT_TARGETS = ["release"]
#: Make targets whose CI invocation builds Rust, so the job needs `mold`.
GATE_TARGETS = {"test", "lint", "typecheck", "build", "all"}
MAKE_TARGET_RE = re.compile(r"\bmake\s+(?:-\S+\s+)*([\w-]+)")


def _normalized(flags: list[str]) -> list[str]:
    """Join ``-C value`` pairs into ``-Cvalue`` so spellings compare equal."""
    joined: list[str] = []
    for flag in flags:
        if joined and joined[-1] == "-C":
            joined[-1] = f"-C{flag}"
        else:
            joined.append(flag)
    return joined


def _sources() -> dict[str, list[str]]:
    """Return every ``rustflags`` source in the configuration, by table."""
    config = tomllib.loads((ROOT / ".cargo" / "config.toml").read_text("utf-8"))
    sources = {"build": config.get("build", {}).get("rustflags")}
    for key, table in config.get("target", {}).items():
        sources[key] = table.get("rustflags")
    # A declared empty list is still a source Cargo would select, so only an
    # absent key is dropped.
    return {
        key: _normalized(flags) for key, flags in sources.items() if flags is not None
    }


def _expanded(value: str, inherited: str | None) -> list[str]:
    """Expand an assigned value as the recipe's shell would, then split it."""
    env = {key: val for key, val in os.environ.items() if key != "RUSTFLAGS"}
    if inherited is not None:
        env["RUSTFLAGS"] = inherited
    result = subprocess.run(
        ["bash", "-c", f'printf "%s" "{value}"'],
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    return _normalized(shlex.split(result.stdout))


def _make_rustflags(
    target: str,
    host: str = "Linux",
    inherited: str | None = None,
    overrides: tuple[str, ...] = (),
) -> list[list[str] | None]:
    """Return, per cargo or whitaker command ``make -n TARGET`` would run on
    the named host, the ``RUSTFLAGS`` it assigns, or ``None`` when it assigns
    none."""
    env = {key: val for key, val in os.environ.items() if key != "RUSTFLAGS"}
    if inherited is not None:
        env["RUSTFLAGS"] = inherited
    result = subprocess.run(
        ["make", "-n", "-B", f"BUILD_HOST_OS={host}", *overrides, target],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    # A recipe continued with a trailing backslash is one command.
    commands = [
        line
        for line in result.stdout.replace("\\\n", " ").splitlines()
        # An `echo` of a tool's path names it without running it.
        if any(word in line for word in COMMAND_WORDS)
        and line.split(maxsplit=1)[:1] not in (["echo"], ["printf"])
    ]
    assert commands, f"`make -n {target}` runs no cargo command"
    assigned: list[list[str] | None] = []
    for line in commands:
        match = RUSTFLAGS_RE.search(line)
        # Any other spelling still replaces the configuration's sources, so a
        # form this reader cannot parse fails rather than passing.
        assert match or "RUSTFLAGS=" not in line, (
            f"unreadable RUSTFLAGS assignment in {line!r}"
        )
        assigned.append(_expanded(match.group(1), inherited) if match else None)
    return assigned


def _contains(flags: list[str], wanted: list[str]) -> bool:
    """Return whether ``wanted`` appears in ``flags`` as a contiguous run."""
    return any(
        flags[start : start + len(wanted)] == wanted
        for start in range(len(flags) - len(wanted) + 1)
    )


def _flag_problems(
    where: str, flags: list[str], *, expects_linker: bool, inherited: str | None
) -> list[str]:
    """Check one assigned ``RUSTFLAGS`` value against the standard.

    ``where`` names the command and host, so a finding says which recipe broke.
    """
    problems = []
    if (THREADS_FLAG in flags) != NIGHTLY:
        problems.append(f"{where} gets {THREADS_FLAG} wrong: {flags}")
    if (LINKER_FLAG in flags) != expects_linker:
        problems.append(f"{where} gets `mold` wrong: {flags}")
    if inherited is not None and not _contains(flags, shlex.split(inherited)):
        problems.append(f"{where} drops the caller's RUSTFLAGS: {flags}")
    return problems


def _unassigned_problems(target: str, inherited: str | None) -> list[str]:
    """Report a command with no assignment that would take only the caller's flags.

    Without an assignment the command takes the configuration's flags, unless
    the caller exports RUSTFLAGS, which displaces them; setup-rust does exactly
    that in CI.
    """
    if inherited is None:
        return []
    return [f"`make {target}` runs a command that takes only the caller's RUSTFLAGS"]


def _development_problems(
    host: str,
    *,
    expects_linker: bool,
    inherited: str | None = None,
    overrides: tuple[str, ...] = (),
) -> list[str]:
    """Check every development target on one host: an assigned ``RUSTFLAGS``,
    empty or not, carries the frontend flag, and carries `mold` exactly when on
    Linux."""
    problems = []
    for target in DEVELOPMENT_TARGETS:
        for flags in _make_rustflags(target, host, inherited, overrides):
            problems += (
                _unassigned_problems(target, inherited)
                if flags is None
                else _flag_problems(
                    f"`make {target}` on {host}",
                    flags,
                    expects_linker=expects_linker,
                    inherited=inherited,
                )
            )
    return problems


def test_every_rustflags_source_carries_the_parallel_frontend() -> None:
    """Cargo applies one source, so each must name the flag itself.

    On a stable pin the flag would stop every build, so it must be absent.
    """
    sources = _sources()
    if not NIGHTLY:
        carrying = [key for key, flags in sources.items() if THREADS_FLAG in flags]
        assert carrying == [], f"{THREADS_FLAG} on a stable pin in {carrying}"
        return
    assert "build" in sources, "no [build] rustflags for non-Linux hosts"
    missing = [key for key, flags in sources.items() if THREADS_FLAG not in flags]
    assert missing == [], f"{THREADS_FLAG} missing from {missing}"


def test_linker_is_confined_to_linux() -> None:
    """`mold` ships for Linux only; a wider source would break other hosts."""
    sources = _sources()
    linux = [key for key in sources if key in LINUX_TABLES]
    assert linux, "no Linux target table carries rustflags"
    assert all(LINKER_FLAG in sources[key] for key in linux), "Linux lost `mold`"
    wider = [
        key
        for key, flags in sources.items()
        if key not in LINUX_TABLES and LINKER_FLAG in flags
    ]
    assert wider == [], f"`mold` named beyond Linux in {wider}"


def test_sources_differ_only_by_the_linker() -> None:
    """A flag named in one source and not another vanishes on some host."""
    stripped = {
        tuple(f for f in flags if f != LINKER_FLAG) for flags in _sources().values()
    }
    assert len(stripped) == 1, f"rustflags sources disagree: {stripped}"


def test_development_targets_restate_both_flags_on_linux() -> None:
    """An assigned RUSTFLAGS replaces the configuration's sources."""
    problems = _development_problems("Linux", expects_linker=True)
    assert problems == [], problems
    for target in ASSIGNING_TARGETS:
        assert any(flags is not None for flags in _make_rustflags(target)), (
            f"`make {target}` assigns no RUSTFLAGS"
        )


def test_development_targets_keep_the_standard_under_inherited_rustflags() -> None:
    """A caller's exported RUSTFLAGS is composed with the standard flags.

    setup-rust exports ``RUSTFLAGS`` in CI, so a recipe that assigned instead
    of composing would drop the caller's flags, and one that inherited without
    restating would drop the standard ones.
    """
    problems = _development_problems("Linux", expects_linker=True, inherited=INHERITED)
    assert problems == [], problems


def test_development_targets_keep_the_frontend_but_not_the_linker_elsewhere() -> None:
    """`mold` is a Linux linker; other hosts keep only the frontend flag."""
    problems = _development_problems("Darwin", expects_linker=False)
    assert problems == [], problems


def test_development_targets_leave_the_linker_off_a_non_linux_target() -> None:
    """Cargo matches ``[target.*]`` sources against the compilation target.

    A Linux host building for another platform through ``CARGO_BUILD_TARGET``
    must not be handed `mold`, while a Linux target keeps it.
    """
    problems = _development_problems(
        "Linux",
        expects_linker=False,
        overrides=("CARGO_BUILD_TARGET=aarch64-apple-darwin",),
    )
    problems += _development_problems(
        "Linux",
        expects_linker=True,
        overrides=("CARGO_BUILD_TARGET=aarch64-unknown-linux-gnu",),
    )
    # Cargo resolves `host-tuple` to the host's own triple.
    problems += _development_problems(
        "Linux", expects_linker=True, overrides=("CARGO_BUILD_TARGET=host-tuple",)
    )
    assert problems == [], problems


@pytest.mark.parametrize("target", HELD_OUT_TARGETS)
def test_coverage_and_release_take_neither_flag(target: str) -> None:
    """Coverage measures and release ships, so both stay on default flags.

    Every command must assign RUSTFLAGS, since only an assignment displaces
    the configuration's sources.
    """
    for flags in _make_rustflags(target):
        assert flags is not None, (
            f"`make {target}` runs a command that takes the configuration's flags"
        )
        assert THREADS_FLAG not in flags, f"`make {target}` takes {THREADS_FLAG}"
        assert LINKER_FLAG not in flags, f"`make {target}` takes {LINKER_FLAG}"


def _linux_jobs() -> list[tuple[str, dict]]:
    """Return every CI job not placed on Windows or macOS, named by file."""
    jobs = []
    for path in sorted((ROOT / ".github" / "workflows").glob("*.y*ml")):
        workflow = yaml.safe_load(path.read_text("utf-8")) or {}
        jobs.extend(
            (f"{path.name}:{name}", job)
            for name, job in (workflow.get("jobs") or {}).items()
            if not re.search(r"windows|macos", str(job.get("runs-on", "")), re.I)
        )
    return jobs


def _first_positions(step: dict) -> tuple[int | None, int | None]:
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
