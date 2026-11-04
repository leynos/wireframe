"""Protect Whitaker's whole-workspace coverage and the lint sequencing.

Run with ``make test-workflow-contracts``. The dry runs substitute a probe
Whitaker command, so these assertions read Make's evaluated recipes without
installing a linter or compiling the Rust workspace.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAKEFILE_PATH = ROOT / "Makefile"
DEV_FAST_ARGUMENT = "--config tools/dev-fast/config.toml"
LINKER_ARGUMENT = "-Clink-arg=-fuse-ld=mold"
#: The three lint stages the aggregate must sequence.
LINT_LEAVES = ("lint-doc", "lint-clippy", "lint-whitaker")


def _make_dry_run(target: str) -> list[str]:
    """Return the evaluated recipe lines for ``target`` without running them."""
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment.pop("CARGO_BUILD_TARGET", None)
    result = subprocess.run(
        [
            "make",
            "--dry-run",
            "-B",
            "-f",
            str(MAKEFILE_PATH),
            target,
            "CARGO=probe-cargo",
            "WHITAKER=probe-whitaker",
        ],
        check=True,
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
    )
    return result.stdout.splitlines()


# A controlled failing gate: the lint recipe must neither guard, ignore nor
# divert this exit status. `false` is used rather than `exit 7`, because a
# trailing shell comment or stray argument would let a `|| true` guard hide an
# `exit` and the test would stop distinguishing a binding gate from a soft skip.
# Kept as a command rather than a fixture file so no executable bit is needed.
FAILING_GATE = "false"


def _whitaker_lines() -> list[str]:
    """Return the single Whitaker invocation ``make lint`` would run."""
    lines = [line for line in _make_dry_run("lint") if "probe-whitaker" in line]
    assert len(lines) == 1, (
        f"lint should produce one Whitaker invocation, found {len(lines)}"
    )
    return lines


def test_whitaker_checks_every_workspace_member_target_and_feature() -> None:
    """Whitaker forwards the full Cargo selection to the inner ``cargo check``.

    ``whitaker --all`` selects lint libraries, not Cargo packages. The
    workspace, all-target and all-feature flags must follow the ``--``
    separator so they reach the ``cargo check`` Whitaker drives; without them
    the helper crates and non-default targets are never dylint-checked.
    """
    line = _whitaker_lines()[0]
    before_separator, _, after_separator = line.partition(" -- ")
    assert before_separator.split(), f"missing Whitaker command in: {line}"
    assert "--all" in before_separator.split(), (
        f"Whitaker must load every discovered lint library: {line}"
    )
    forwarded = after_separator.split()
    for flag in ("--workspace", "--all-targets", "--all-features"):
        assert flag in forwarded, (
            f"Whitaker must forward {flag} to `cargo check`: {line}"
        )


def test_whitaker_stays_isolated_from_development_flags() -> None:
    """The shared installer's toolchain does not inherit development routing."""
    line = _whitaker_lines()[0]
    assert DEV_FAST_ARGUMENT not in line, (
        "Whitaker must not receive the development Cargo configuration"
    )
    assert LINKER_ARGUMENT not in line, (
        "Whitaker must not receive Make's development linker flag"
    )


def test_lint_runs_every_leaf_before_whitaker_even_under_parallel_make() -> None:
    """Sequencing is declared, not incidental to prerequisite order.

    Prerequisites of one target run concurrently under ``make -j``, so the
    recipe must opt out of that for the aggregate target.
    """
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    assert ".NOTPARALLEL: lint" in makefile.splitlines(), (
        "`lint` must declare .NOTPARALLEL so its leaves cannot interleave"
    )
    assert "lint: lint-doc lint-clippy lint-whitaker" in makefile, (
        "`lint` must aggregate the rustdoc, Clippy and Whitaker leaves in order"
    )


def test_lint_leaves_are_phony_and_install_is_separate() -> None:
    """Each leaf is directly runnable, and installation stays explicit."""
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    for leaf in (*LINT_LEAVES, "install-whitaker"):
        assert any(
            leaf in line.removeprefix(".PHONY:").split()
            for line in makefile.splitlines()
            if line.startswith(".PHONY:")
        ), f"`{leaf}` must be declared phony"
    lint_rule = next(
        line for line in makefile.splitlines() if line.startswith("lint:")
    )
    assert "install-whitaker" not in lint_rule.split(), (
        "`lint` must not install tooling as a prerequisite"
    )


def test_a_failing_whitaker_fails_the_aggregate_lint_target() -> None:
    """The gate is binding, not merely mentioned in a recipe.

    ``make lint`` must surface a Whitaker failure. A guard such as
    ``|| true`` or an ignored-error prefix would hide it and turn the suite
    into advice.
    """
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    result = subprocess.run(
        [
            "make",
            "-f",
            str(MAKEFILE_PATH),
            "lint",
            f"WHITAKER={FAILING_GATE}",
            "CARGO=true",
            "CLIPPY_FLAGS=--version",
            "DOC_FLAGS=",
            "DEV_WARNING_FLAGS=",
            "RUSTDOC_FLAGS=",
            "DEV_FAST=",
        ],
        check=False,
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0, (
        "`make lint` must fail when the Whitaker gate fails, "
        f"but it exited 0:\n{result.stdout}\n{result.stderr}"
    )
