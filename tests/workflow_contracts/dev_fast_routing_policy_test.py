"""Prove warning and effective-target contracts for development routing."""

from __future__ import annotations

import platform
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import pytest

from dev_fast_routing_test import (
    LINKER_ARGUMENT,
    MAKEFILE_PATH,
    RUSTDOC_CONTRACT_FLAGS,
    _MakeDryRunOptions,
    _assert_caller_rustflags,
    _assert_dev_fast_fragment,
    _assert_warning_denial,
    _cargo_lines,
    _make_dry_run,
)

DEV_WARNING_FLAGS = "DEV_WARNING_FLAGS = $(strip $(RUSTFLAGS) -D warnings $(DEV_LINUX_LINK_ARG))"


@dataclass(frozen=True)
class _MakefileMutation:
    """Describe one isolated Makefile replacement used by a contract test."""

    target: str
    original: str
    replacement: str
    options: _MakeDryRunOptions = _MakeDryRunOptions()


@dataclass(frozen=True)
class _RoutingFlagMutation:
    """Pair one routing-flag mutation with the contract it must violate."""

    replacement: str
    assertion: Callable[[str, list[str]], None]
    expected_message: str


def _assert_rustdoc_flags(cargo_lines: list[str], rustdoc_flags: str) -> None:
    """Require lint's rustdoc command to preserve its configured flags."""
    doc_lines = [line for line in cargo_lines if "doc" in line.split()]
    assert len(doc_lines) == 1, "lint must retain exactly one rustdoc invocation"
    expected = f'RUSTDOCFLAGS="{rustdoc_flags}"'
    assert expected in doc_lines[0], (
        "lint rustdoc must preserve the configured RUSTDOC_FLAGS: "
        f"expected {expected!r} in {doc_lines[0]!r}"
    )


def _assert_debug_target_contract(target: str, cargo_lines: list[str]) -> None:
    """Require shared debug-routing policy before testing linker selection."""
    assert cargo_lines, f"{target} should produce a probe-cargo invocation"
    _assert_dev_fast_fragment(target, cargo_lines)
    _assert_caller_rustflags(target, cargo_lines)
    _assert_warning_denial(target, cargo_lines)


def _mutated_cargo_lines(
    tmp_path: Path,
    mutation: _MakefileMutation,
) -> list[str]:
    """Return Cargo lines after one private Makefile mutation."""
    source = MAKEFILE_PATH.read_text(encoding="utf-8")
    assert source.count(mutation.original) == 1, "the Makefile mutation must be unambiguous"
    mutated_makefile = tmp_path / "Makefile"
    mutated_makefile.write_text(
        source.replace(mutation.original, mutation.replacement, 1), encoding="utf-8"
    )
    return _cargo_lines(
        _make_dry_run(mutation.target, mutated_makefile, options=mutation.options)
    )


@pytest.mark.skipif(platform.system() != "Linux", reason="native Linux linker only")
@pytest.mark.parametrize(
    "target",
    ("build", "test", "test-bdd", "test-doc", "lint", "typecheck", "dev-build", "dev-test"),
)
def test_explicit_linux_target_retains_native_linker(target: str) -> None:
    """A Linux target triple keeps `mold` when selected on a Linux host."""
    options = _MakeDryRunOptions(build_target="x86_64-unknown-linux-gnu")
    cargo_lines = _cargo_lines(_make_dry_run(target, options=options))
    _assert_debug_target_contract(target, cargo_lines)
    assert all(LINKER_ARGUMENT in line for line in cargo_lines), (
        f"{target} must pass the Linux linker to an explicit Linux target"
    )


@pytest.mark.skipif(platform.system() != "Linux", reason="native Linux linker only")
@pytest.mark.parametrize(
    ("build_target", "build_target_os", "expects_linker"),
    (
        pytest.param("wasm32-wasip1", None, False, id="wasm"),
        pytest.param("x86_64-linux-android", None, False, id="android"),
        pytest.param("targets/custom-linux-target.json", None, False, id="custom"),
        pytest.param("targets/custom-linux-target.json", "Linux", True, id="custom-linux"),
    ),
)
@pytest.mark.parametrize(
    "target",
    ("build", "test", "test-bdd", "test-doc", "lint", "typecheck", "dev-build", "dev-test"),
)
def test_cross_target_uses_effective_target_os(
    target: str,
    build_target: str,
    build_target_os: str | None,
    expects_linker: bool,
) -> None:
    """Cross-target debug recipes route `mold` only to known Linux targets."""
    options = _MakeDryRunOptions(
        build_target=build_target, build_target_os=build_target_os
    )
    cargo_lines = _cargo_lines(_make_dry_run(target, options=options))
    _assert_debug_target_contract(target, cargo_lines)
    assert all((LINKER_ARGUMENT in line) is expects_linker for line in cargo_lines), (
        f"{target} linker route did not match {build_target!r} with "
        f"CARGO_BUILD_TARGET_OS={build_target_os!r}"
    )


def test_lint_preserves_configured_rustdoc_flags() -> None:
    """The lint rustdoc command carries the configured docs policy."""
    options = _MakeDryRunOptions(rustdoc_flags=RUSTDOC_CONTRACT_FLAGS)
    _assert_rustdoc_flags(
        _cargo_lines(_make_dry_run("lint", options=options)), RUSTDOC_CONTRACT_FLAGS
    )


@pytest.mark.parametrize(
    "mutation",
    (
        pytest.param(
            _RoutingFlagMutation(
                replacement="DEV_WARNING_FLAGS = $(strip -D warnings $(DEV_LINUX_LINK_ARG))",
                assertion=_assert_caller_rustflags,
                expected_message="must preserve",
            ),
            id="caller-rustflags",
        ),
        pytest.param(
            _RoutingFlagMutation(
                replacement="DEV_WARNING_FLAGS = $(strip $(RUSTFLAGS) $(DEV_LINUX_LINK_ARG))",
                assertion=_assert_warning_denial,
                expected_message="must retain",
            ),
            id="warning-denial",
        ),
    ),
)
def test_debug_contract_detects_removed_routing_flags(
    tmp_path: Path, mutation: _RoutingFlagMutation
) -> None:
    """Each private mutation must violate its named debug-routing contract."""
    cargo_lines = _mutated_cargo_lines(
        tmp_path,
        _MakefileMutation(
            target="test", original=DEV_WARNING_FLAGS, replacement=mutation.replacement
        ),
    )
    with pytest.raises(AssertionError, match=mutation.expected_message):
        mutation.assertion("test", cargo_lines)


def test_lint_contract_detects_removed_rustdoc_flags(tmp_path: Path) -> None:
    """A private mutation proves lint's rustdoc flags remain binding."""
    original = 'RUSTDOCFLAGS="$(RUSTDOC_FLAGS)"'
    options = _MakeDryRunOptions(rustdoc_flags=RUSTDOC_CONTRACT_FLAGS)
    cargo_lines = _mutated_cargo_lines(
        tmp_path,
        _MakefileMutation(
            target="lint", original=original, replacement="", options=options
        ),
    )
    with pytest.raises(AssertionError, match="RUSTDOC_FLAGS"):
        _assert_rustdoc_flags(cargo_lines, RUSTDOC_CONTRACT_FLAGS)
