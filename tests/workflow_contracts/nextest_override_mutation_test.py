"""Prove the override contract rejects the ways an override can go wrong.

The checked-in configuration exercises only the accepted shape, so a validator
that ignored a missing matcher, a non-exact matcher, or an uncovered trybuild
target would pass against it either way. Each mutation below breaks exactly one
property of the accepted configuration and must be rejected.
"""

from __future__ import annotations

import typing as typ
from copy import deepcopy
from pathlib import Path

import pytest

from cargo_targets import CargoTestTarget
from nextest_contract_support import (
    TRYSBUILD_SOURCE,
    assert_no_override_violations,
    read_source,
    repository_config,
    repository_targets,
)
from nextest_policy import override_target_violations
from rust_source_lexing import drives_trybuild
from rust_source_scanning import CrateGate, has_crate_scope_gate

if typ.TYPE_CHECKING:
    from collections.abc import Callable, Sequence


def _non_trybuild_target(targets: Sequence[CargoTestTarget]) -> CargoTestTarget:
    """Return a discovered target that does not drive trybuild."""
    return next(
        target
        for target in targets
        if not drives_trybuild(read_source(target.src_path))
        and not has_crate_scope_gate(read_source(target.src_path), CrateGate.LOOM)
    )


def _set_filter(
    config: dict[str, typ.Any], _targets: Sequence[CargoTestTarget], text: str
) -> None:
    """Replace the one override's filter with ``text``."""
    config["profile"]["default"]["overrides"][0]["filter"] = text


def _without_binary(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the binary matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & test(=compile_tests)")


def _without_package(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the package matcher from the filter."""
    _set_filter(config, targets, "binary(=compile_error) & test(=compile_tests)")


def _without_test(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the test matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & binary(=compile_error)")


def _wrong_package(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Name a package that discovers no such target."""
    _set_filter(
        config,
        targets,
        "package(=other) & binary(=compile_error) & test(=compile_tests)",
    )


def _non_exact_binary(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """State the binary matcher as a substring rather than an exact match."""
    _set_filter(
        config,
        targets,
        "package(=wireframe) & binary(compile_error) & test(=compile_tests)",
    )


def _extra_override(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Append an override for a target that does not drive trybuild."""
    target = _non_trybuild_target(targets)
    config["profile"]["default"]["overrides"].append({
        "filter": (
            f"package(={target.package}) & binary(={target.name})"
            " & test(=some_test)"
        ),
        "slow-timeout": {
            "period": "60s",
            "terminate-after": 8,
            "grace-period": "5s",
        },
    })


_OVERRIDE_MUTATIONS: typ.Final = {
    "no_binary": _without_binary,
    "no_package": _without_package,
    "no_test": _without_test,
    "wrong_package": _wrong_package,
    "non_exact": _non_exact_binary,
    "extra_override": _extra_override,
}


def _mutate_override_config(
    mutation: str,
    config: dict[str, typ.Any],
    targets: Sequence[CargoTestTarget],
) -> None:
    """Apply one override-config mutation in place."""
    try:
        mutate = _OVERRIDE_MUTATIONS[mutation]
    except KeyError:
        message = f"unknown override mutation {mutation!r}"
        raise AssertionError(message) from None
    mutate(config, targets)


def _source_reader_with(
    extra: dict[Path, str],
) -> Callable[[Path], str]:
    """Return a reader that serves ``extra`` before the filesystem."""

    def read(path: Path) -> str:
        return extra[path] if path in extra else read_source(path)

    return read


@pytest.mark.parametrize(
    "mutation",
    [
        "no_binary",
        "no_package",
        "no_test",
        "wrong_package",
        "non_exact",
        "extra_override",
        "uncovered_trybuild_target",
        "cfg_gated_test",
    ],
)
def test_override_mutations_are_rejected(mutation: str) -> None:
    """Each mutation alone leaves an override unproved or a target uncovered."""
    config = deepcopy(repository_config())
    targets: tuple[CargoTestTarget, ...] = repository_targets()
    extra_sources: dict[Path, str] = {}
    if mutation == "uncovered_trybuild_target":
        synthetic = CargoTestTarget(
            package="wireframe",
            name="trybuild_extra",
            kinds=("test",),
            src_path=Path("/synthetic/trybuild_extra.rs"),
        )
        targets = (*targets, synthetic)
        extra_sources[synthetic.src_path] = TRYSBUILD_SOURCE
    elif mutation == "cfg_gated_test":
        target = next(target for target in targets if target.name == "compile_error")
        source = read_source(target.src_path)
        gated = source.replace(
            "#[test]\nfn compile_tests()",
            '#[cfg(feature = "never")]\n#[test]\nfn compile_tests()',
        )
        assert gated != source, "the mutation must gate the compile_tests function"
        extra_sources[target.src_path] = gated
    else:
        _mutate_override_config(mutation, config, targets)
    with pytest.raises(AssertionError):
        assert_no_override_violations(
            config, targets, _source_reader_with(extra_sources)
        )


def test_non_list_overrides_are_rejected() -> None:
    """A scalar ``profile.default.overrides`` cannot be iterated as overrides."""
    config = {"profile": {"default": {"overrides": {"filter": "x"}}}}
    assert override_target_violations(config, (), read_source) == [
        "profile.default.overrides is not a list"
    ], "a non-list overrides value must be reported"


def test_non_string_override_filter_is_rejected() -> None:
    """A filter that is not text is reported rather than parsed."""
    config = {"profile": {"default": {"overrides": [{"filter": 7}]}}}
    assert override_target_violations(config, (), read_source) == [
        "override 0 has no string filter"
    ], "a non-string filter must be reported"
