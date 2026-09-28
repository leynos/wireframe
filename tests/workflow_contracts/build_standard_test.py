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
RUSTFLAGS_RE = re.compile(r'RUSTFLAGS="([^"]*)"')
#: A caller's own flags, to prove a recipe keeps the standard when it inherits
#: an exported ``RUSTFLAGS``.
INHERITED = "-D warnings"
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
    return {key: _normalized(flags) for key, flags in sources.items() if flags}


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
    target: str, host: str = "Linux", inherited: str | None = None
) -> list[list[str] | None]:
    """Return, per cargo or whitaker command ``make -n TARGET`` would run on
    the named host, the ``RUSTFLAGS`` it assigns, or ``None`` when it assigns
    none."""
    result = subprocess.run(
        ["make", "-n", "-B", f"BUILD_HOST_OS={host}", target],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    # A recipe continued with a trailing backslash is one command.
    commands = [
        line
        for line in result.stdout.replace("\\\n", " ").splitlines()
        if any(word in line for word in COMMAND_WORDS)
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


def _development_problems(
    host: str, *, expects_linker: bool, inherited: str | None = None
) -> list[str]:
    """Check every development target on one host: an assigned ``RUSTFLAGS``,
    empty or not, carries the frontend flag, and carries `mold` exactly when on
    Linux."""
    problems = []
    for target in DEVELOPMENT_TARGETS:
        for flags in _make_rustflags(target, host, inherited):
            if flags is None:
                continue
            if THREADS_FLAG not in flags:
                problems.append(
                    f"`make {target}` on {host} drops {THREADS_FLAG}: {flags}"
                )
            if (LINKER_FLAG in flags) != expects_linker:
                problems.append(f"`make {target}` on {host} gets `mold` wrong: {flags}")
    return problems


def test_every_rustflags_source_carries_the_parallel_frontend() -> None:
    """Cargo applies one source, so each must name the flag itself."""
    sources = _sources()
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
    """A caller's exported RUSTFLAGS must not strip the standard flags."""
    problems = _development_problems("Linux", expects_linker=True, inherited=INHERITED)
    assert problems == [], problems


def test_development_targets_keep_the_frontend_but_not_the_linker_elsewhere() -> None:
    """`mold` is a Linux linker; other hosts keep only the frontend flag."""
    problems = _development_problems("Darwin", expects_linker=False)
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


def _jobs_missing_the_linker() -> list[str]:
    """Return the Linux CI jobs that run a gate target without installing
    `mold` first."""
    missing = []
    for path in sorted((ROOT / ".github" / "workflows").glob("*.y*ml")):
        workflow = yaml.safe_load(path.read_text("utf-8")) or {}
        for name, job in (workflow.get("jobs") or {}).items():
            if re.search(r"windows|macos", str(job.get("runs-on", "")), re.I):
                continue
            installed = False
            for step in job.get("steps") or []:
                run = str(step.get("run", ""))
                if re.search(r"apt(-get)?\s+install[^\n]*\bmold\b", run):
                    installed = True
                if not installed and GATE_TARGETS & set(MAKE_TARGET_RE.findall(run)):
                    missing.append(f"{path.name}:{name}")
                    break
    return missing


def test_ci_installs_the_linker_before_gate_targets() -> None:
    """The gate targets restate `mold`, so a Linux job must install it first."""
    missing = _jobs_missing_the_linker()
    assert missing == [], f"jobs run a gate target before installing `mold`: {missing}"
