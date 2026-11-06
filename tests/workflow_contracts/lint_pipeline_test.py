"""Protect the canonical ``make lint`` pipeline and its pins.

The pipeline is the single place CI and contributors obtain lint coverage:
rustdoc over every workspace package and feature, Clippy over every target and
feature, and Whitaker over every package, target and feature. Each stage denies
warnings. The same Makefile owns the Whitaker installation pins, so a reviewed
bump is one edit.

Run with ``make test-workflow-contracts``. The dry runs substitute probe
commands, so these assertions read Make's evaluated recipes without compiling
anything or installing a linter. The failure-propagation tests run
``make lint`` with stubbed commands, so its exit status proves the aggregate
is binding without compiling either.
"""

from __future__ import annotations

import os
import re
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from build_standard_test import RUSTFLAGS_RE
from dev_fast_routing_test import (
    MAKEFILE_PATH,
    ROOT,
    _cargo_lines,
    _make_dry_run,
)

INSTALL_TARGET = "install-whitaker"
LINT_LEAVES = ("lint-doc", "lint-clippy", "lint-whitaker")
# The Rust leaves above run cargo; this one holds docstring presence for the Python tests.
PYTHON_DOCS_LEAF = "lint-python-docs"
PIN_NAMES = (
    "WHITAKER_INSTALLER_VERSION",
    "CARGO_DYLINT_VERSION",
    "DYLINT_LINK_VERSION",
    "WHITAKER_SUITE_REV",
)
PIN_ASSIGNMENT_RE = re.compile(r"^(?P<name>[A-Z_]+) \?= (?P<value>\S+)$", re.MULTILINE)
FULL_COMMIT_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
RUSTDOCFLAGS_RE = re.compile(r'RUSTDOCFLAGS="([^"]*)"')
#: A leading shell-variable assignment, which is not an argument to anything.
ASSIGNMENT_PREFIX_RE = re.compile(r'^[A-Za-z_][A-Za-z0-9_]*="[^"]*"\s+')


def _lint_dry_run(makefile: Path = MAKEFILE_PATH) -> list[str]:
    """Return the evaluated commands ``make lint`` would run."""
    return _make_dry_run("lint", makefile)


def _install_dry_run(makefile: Path = MAKEFILE_PATH) -> list[str]:
    """Return the evaluated commands ``make install-whitaker`` would run."""
    environment = {key: value for key, value in os.environ.items() if key != "RUSTFLAGS"}
    result = subprocess.run(
        [
            "make",
            "--dry-run",
            "-B",
            "-f",
            str(makefile),
            INSTALL_TARGET,
            "CARGO=probe-cargo",
            "WHITAKER_INSTALLER=probe-installer",
        ],
        check=True,
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
    )
    return result.stdout.splitlines()


def _single(
    lines: list[str], predicate: Callable[[str], bool], description: str
) -> str:
    """Return the one line matching a predicate, naming what was expected."""
    matches = [line for line in lines if predicate(line)]
    assert len(matches) == 1, (
        f"expected exactly one {description}, found {len(matches)}"
    )
    return matches[0]


def _assigned_rustflags(line: str) -> str:
    """Return the ``RUSTFLAGS`` value a recipe assigns."""
    match = RUSTFLAGS_RE.search(line)
    assert match is not None, f"expected a RUSTFLAGS assignment in {line!r}"
    return match.group(1)


def _recipe_arguments(line: str) -> list[str]:
    """Return a recipe's arguments, ignoring leading shell assignments."""
    remainder = line
    while match := ASSIGNMENT_PREFIX_RE.match(remainder):
        remainder = remainder[match.end() :]
    return remainder.split()


def _after_the_separator(line: str) -> list[str]:
    """Return the arguments forwarded after the ``--`` separator."""
    _, separator, forwarded = line.partition(" -- ")
    assert separator, f"expected a `--` separator in {line!r}"
    return forwarded.split()


def _assert_clippy(line: str) -> None:
    """Require workspace, all-target, all-feature, warning-denying Clippy."""
    arguments = _recipe_arguments(line)
    for flag in ("--workspace", "--all-targets", "--all-features"):
        assert flag in arguments, f"`lint-clippy` must pass {flag}: {line}"
    forwarded = " ".join(_after_the_separator(line))
    assert "-D warnings" in forwarded, (
        f"`lint-clippy` must deny warnings after `--`: {line}"
    )
    assert "-D warnings" in _assigned_rustflags(line), (
        f"`lint-clippy` must deny warnings in RUSTFLAGS too: {line}"
    )


def _assert_rustdoc(line: str) -> None:
    """Require workspace, all-feature, no-dependency, warning-denying rustdoc."""
    arguments = _recipe_arguments(line)
    for flag in ("--workspace", "--all-features", "--no-deps"):
        assert flag in arguments, f"`lint-doc` must pass {flag}: {line}"
    rustdocflags = RUSTDOCFLAGS_RE.search(line)
    assert rustdocflags is not None, (
        f"`lint-doc` must assign RUSTDOCFLAGS: {line}"
    )
    assert "-D warnings" in rustdocflags.group(1), (
        f"`lint-doc` must deny warnings through RUSTDOCFLAGS: {line}"
    )
    assert "-D warnings" in _assigned_rustflags(line), (
        f"`lint-doc` must deny warnings in RUSTFLAGS too: {line}"
    )


def _assert_whitaker(line: str) -> None:
    """Require the full Cargo selection forwarded to Whitaker's inner check."""
    assert _assigned_rustflags(line) == "-D warnings", (
        f"`lint-whitaker` must deny warnings through RUSTFLAGS: {line}"
    )
    assert "--all" in _recipe_arguments(line), (
        f"`lint-whitaker` must load every lint: {line}"
    )
    forwarded = _after_the_separator(line)
    for flag in ("--workspace", "--all-targets", "--all-features"):
        assert flag in forwarded, (
            f"`lint-whitaker` must forward {flag} to `cargo check`: {line}"
        )


def _assert_lint_rule(source: str) -> None:
    """Require the aggregate target to sequence every leaf in order."""
    for leaf in (*LINT_LEAVES, PYTHON_DOCS_LEAF):
        assert any(line.startswith(f"{leaf}:") for line in source.splitlines()), (
            f"the Makefile must declare `{leaf}`"
        )
    lint_rules = [line for line in source.splitlines() if line.startswith("lint:")]
    assert len(lint_rules) == 1, "the Makefile must declare `lint` once"
    prerequisites = lint_rules[0].split("##")[0].split(":", 1)[1].split()
    assert prerequisites == [*LINT_LEAVES, PYTHON_DOCS_LEAF], (
        "`lint` must aggregate the Rust leaves in order, then the Python docstring check"
    )


def _assert_lint_pipeline(lines: list[str], makefile: Path = MAKEFILE_PATH) -> None:
    """Assert the whole lint pipeline, so a mutation fails for the right reason."""
    cargo_lines = _cargo_lines(lines)
    _assert_rustdoc(
        _single(cargo_lines, lambda line: "doc" in line.split(), "rustdoc command")
    )
    _assert_clippy(
        _single(cargo_lines, lambda line: "clippy" in line.split(), "Clippy command")
    )
    _assert_whitaker(
        _single(
            lines,
            lambda line: "probe-whitaker" in line,
            "Whitaker command",
        )
    )
    _assert_lint_rule(makefile.read_text(encoding="utf-8"))


def test_make_lint_runs_every_stage_with_full_coverage() -> None:
    """``make lint`` runs rustdoc, Clippy and Whitaker over the whole workspace."""
    _assert_lint_pipeline(_lint_dry_run())


def test_install_whitaker_is_reproducibly_pinned() -> None:
    """The install target pins every component and the suite revision."""
    lines = _install_dry_run()
    cargo_installs = [line for line in lines if "probe-cargo install" in line]
    assert len(cargo_installs) == 3, (
        f"expected three pinned Cargo installs, found {len(cargo_installs)}"
    )
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    pins = {
        match.group("name"): match.group("value")
        for match in PIN_ASSIGNMENT_RE.finditer(makefile)
    }
    # The installer is named by `$(WHITAKER_INSTALLER)`, which the dry run
    # substitutes, so it is matched by that substituted command rather than by
    # the package name it would install.
    for marker, pin in (
        ("cargo-dylint", "CARGO_DYLINT_VERSION"),
        ("dylint-link", "DYLINT_LINK_VERSION"),
        ("probe-installer", "WHITAKER_INSTALLER_VERSION"),
    ):
        line = _single(
            cargo_installs,
            lambda candidate, marker=marker: marker in candidate.split(),
            f"install of {marker}",
        )
        assert "--locked" in line.split(), f"`{marker}` must install with --locked"
        assert pins[pin] in line.split(), (
            f"`{marker}` must install the pinned {pin}={pins[pin]}"
        )
    suite_line = _single(
        lines,
        lambda line: "--suite-version" in line.split(),
        "Whitaker suite install",
    )
    assert "--suite-version" in suite_line.split(), (
        "the suite install must pin its revision"
    )
    suite_rev = pins["WHITAKER_SUITE_REV"]
    assert FULL_COMMIT_SHA_RE.fullmatch(suite_rev), (
        f"WHITAKER_SUITE_REV must be a full commit SHA, got {suite_rev!r}"
    )
    assert suite_rev in suite_line.split(), "the suite install must use the pinned SHA"


def test_each_whitaker_pin_is_assigned_once_in_the_makefile() -> None:
    """A pin duplicated elsewhere invites a silent divergence."""
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    for pin in PIN_NAMES:
        assert makefile.count(f"{pin} ?=") == 1, (
            f"{pin} must be assigned exactly once in the Makefile"
        )


@pytest.mark.parametrize(
    ("original", "mutated"),
    [
        pytest.param(
            "DOC_FLAGS ?= --workspace --all-features --no-deps",
            "DOC_FLAGS ?= --all-features --no-deps",
            id="rustdoc_loses_workspace",
        ),
        pytest.param(
            "RUSTDOC_FLAGS ?= --cfg docsrs -D warnings",
            "RUSTDOC_FLAGS ?= --cfg docsrs",
            id="rustdoc_loses_warning_denial",
        ),
        pytest.param(
            "CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings",
            "CLIPPY_FLAGS ?= --workspace --all-targets --all-features",
            id="clippy_loses_warning_denial",
        ),
        pytest.param(
            "WHITAKER_FLAGS ?= --all -- --workspace --all-targets --all-features",
            "WHITAKER_FLAGS ?= --all -- --workspace",
            id="whitaker_loses_target_and_feature_coverage",
        ),
        pytest.param(
            "lint: lint-doc lint-clippy lint-whitaker",
            "lint: lint-doc lint-clippy",
            id="aggregate_loses_whitaker",
        ),
    ],
)
def test_lint_pipeline_contract_fails_on_a_removed_guarantee(
    tmp_path: Path, original: str, mutated: str
) -> None:
    """A private Makefile mutation proves the positive contract is binding."""
    source = MAKEFILE_PATH.read_text(encoding="utf-8")
    assert source.count(original) == 1, "the mutation target must be unambiguous"
    mutated_makefile = tmp_path / "Makefile"
    mutated_makefile.write_text(source.replace(original, mutated), encoding="utf-8")
    with pytest.raises(AssertionError):
        _assert_lint_pipeline(_lint_dry_run(mutated_makefile), mutated_makefile)


#: A Cargo stand-in that fails for exactly one subcommand. Every lint leaf
#: shells out to ``$(CARGO)``, so this isolates one leaf without editing the
#: Makefile: a ``doc`` failure cannot be masked by ``clippy`` succeeding.
FAILING_CARGO_TEMPLATE = """#!/bin/sh
for argument in "$@"; do
    if [ "$argument" = "{subcommand}" ]; then
        exit 1
    fi
done
exit 0
"""


def _failing_cargo(tmp_path: Path, subcommand: str) -> Path:
    """Write a Cargo stand-in that fails only for ``subcommand``."""
    script = tmp_path / f"failing-cargo-{subcommand}.sh"
    script.write_text(
        FAILING_CARGO_TEMPLATE.format(subcommand=subcommand), encoding="utf-8"
    )
    return script


@pytest.mark.parametrize("subcommand", ["doc", "clippy"])
def test_make_lint_propagates_a_failing_cargo_leaf(
    tmp_path: Path, subcommand: str
) -> None:
    """The aggregate is binding for the rustdoc and Clippy leaves too.

    A recipe that prefixed its command with ``-`` or appended ``|| true``
    would report success while the leaf failed. The stub keeps the other
    leaves trivial, so a non-zero exit can only come from the failing one.
    """
    environment = {
        key: value for key, value in os.environ.items() if key != "RUSTFLAGS"
    }
    result = subprocess.run(
        [
            "make",
            "-f",
            str(MAKEFILE_PATH),
            "lint",
            f"CARGO=sh {_failing_cargo(tmp_path, subcommand)}",
            "WHITAKER=true",
            "DEV_FAST=",
            "DEV_WARNING_FLAGS=",
            "RUSTDOC_FLAGS=",
            "DOC_FLAGS=",
            "CLIPPY_FLAGS=",
        ],
        check=False,
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0, (
        f"`make lint` must fail when the {subcommand} leaf fails, "
        f"but it exited 0:\n{result.stdout}\n{result.stderr}"
    )
