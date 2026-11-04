"""Prove that a nextest override names a target Cargo actually builds.

A nextest override names a package, a binary and a test. Nothing in nextest's
own configuration proves that pair exists: a target renamed, or moved from
``tests/name.rs`` to ``tests/name/mod.rs``, silently stops matching and the
test falls back to the short allowance. ``override_target_violations`` holds
the filter against the targets ``cargo metadata`` reports, so that rot fails a
contract instead of passing unnoticed.

The test-function check is source-level: it proves a crate-root ``#[test]``
function with the filter's exact name is written in the target's source, not
that nextest's own test list contains it. Nextest test-list evidence is
deliberately not used, because producing it would run the very compilation
this contract exists to bound. Because nextest matches ``test(=name)`` against
the fully qualified test name, the check computes the module path from inline
``mod`` blocks: a test defined only as ``contracts::compile_tests`` does not
satisfy ``test(=compile_tests)``. Contract runs the scanning in
``rust_source_scanning``, which also strips comments and literals so a
commented-out definition is not evidence.

The timeout tier chain lives in ``timeout_policy``; the two modules are split
by role rather than by size, and both are called from the tests beside them.
"""

from __future__ import annotations

import dataclasses
import re
import typing as typ

from rust_source_scanning import (
    defines_test_function,
    drives_trybuild,
    is_loom_gated,
    test_function_note,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

    from cargo_targets import CargoTestTarget

#: One exact matcher. ``=`` is nextest's exact form; ``~`` and a bare name are
#: refused because a regex or substring matcher can match a target the author
#: did not mean, and the override would then apply somewhere unreviewed.
_FILTER_TERM: typ.Final = re.compile(
    r"^(?P<key>package|binary|test)\(=(?P<value>[A-Za-z0-9_.\-]+)\)$"
)
_REQUIRED_FILTER_KEYS: typ.Final = frozenset({"package", "binary", "test"})


class FilterSyntaxError(ValueError):
    """A nextest filter this policy will not read."""


@dataclasses.dataclass(frozen=True)
class TargetFilter:
    """The three exact matchers an override filter must carry."""

    package: str
    binary: str
    test: str


def parse_target_filter(text: str) -> TargetFilter:
    """Parse a conjunction of exact ``package``, ``binary`` and ``test`` terms.

    Parameters
    ----------
    text
        One nextest override filter.

    Returns
    -------
    TargetFilter
        The three matcher values.

    Raises
    ------
    FilterSyntaxError
        When a term is not an exact matcher, a matcher is repeated, a required
        matcher is missing, or any other predicate or operator appears.

    Examples
    --------
    >>> parse_target_filter(
    ...     "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
    ... )
    TargetFilter(package='wireframe', binary='compile_error', test='compile_tests')
    """
    values: dict[str, str] = {}
    for raw_term in text.split("&"):
        match = _FILTER_TERM.match(raw_term.strip())
        if match is None:
            message = (
                f"filter term {raw_term.strip()!r} is not an exact "
                "package(=...), binary(=...) or test(=...) matcher"
            )
            raise FilterSyntaxError(message)
        key = match.group("key")
        if key in values:
            message = f"filter repeats the {key} matcher"
            raise FilterSyntaxError(message)
        values[key] = match.group("value")
    missing = _REQUIRED_FILTER_KEYS - values.keys()
    if missing:
        message = f"filter omits the {', '.join(sorted(missing))} matcher"
        raise FilterSyntaxError(message)
    return TargetFilter(
        package=values["package"],
        binary=values["binary"],
        test=values["test"],
    )


class _SourceCache:
    """Read each target's source at most once while checking overrides."""

    def __init__(self, read_source: typ.Callable[[Path], str]) -> None:
        self._read_source = read_source
        self._sources: dict[Path, str] = {}

    def __call__(self, target: CargoTestTarget) -> str:
        if target.src_path not in self._sources:
            self._sources[target.src_path] = self._read_source(target.src_path)
        return self._sources[target.src_path]


def _parsed_override(
    override: dict[str, typ.Any], index: int, violations: list[str]
) -> TargetFilter | None:
    """Return the filter an override states, or record why it cannot be read."""
    raw_filter = override.get("filter")
    if not isinstance(raw_filter, str):
        violations.append(f"override {index} has no string filter")
        return None
    try:
        return parse_target_filter(raw_filter)
    except FilterSyntaxError as exc:
        violations.append(f"override {index} filter {raw_filter!r}: {exc}")
        return None


def _matching_targets(
    targets: typ.Sequence[CargoTestTarget], parsed: TargetFilter
) -> list[CargoTestTarget]:
    """Return the discovered targets whose package and binary the filter names."""
    return [
        target
        for target in targets
        if target.package == parsed.package and target.name == parsed.binary
    ]


@dataclasses.dataclass(frozen=True)
class _ResolvedOverride:
    """One override resolved to its single target and that target's source."""

    index: int
    target: CargoTestTarget
    parsed: TargetFilter
    source: str


def _target_source_violations(
    resolved: _ResolvedOverride, violations: list[str]
) -> None:
    """Check the named target's source defines the test and drives trybuild."""
    target, parsed, source = (
        resolved.target,
        resolved.parsed,
        resolved.source,
    )
    if not defines_test_function(source, parsed.test):
        violations.append(
            f"override {resolved.index} names test {parsed.test!r}, which "
            f"{target.src_path} does not define at its crate root"
            f"{test_function_note(source, parsed.test)}"
        )
    if not drives_trybuild(source):
        violations.append(
            f"override {resolved.index} names {target.src_path}, which does "
            "not drive trybuild::TestCases"
        )


def _override_violations(
    overrides: typ.Sequence[dict[str, typ.Any]],
    targets: typ.Sequence[CargoTestTarget],
    source_of: _SourceCache,
    violations: list[str],
) -> set[tuple[str, str]]:
    """Check each override and return the targets they claim."""
    claimed: set[tuple[str, str]] = set()
    for index, override in enumerate(overrides):
        parsed = _parsed_override(override, index, violations)
        if parsed is None:
            continue
        matches = _matching_targets(targets, parsed)
        if len(matches) != 1:
            violations.append(
                f"override {index} names {parsed.package}/{parsed.binary}, "
                f"which resolves to {len(matches)} discovered test targets"
            )
            continue
        target = matches[0]
        claimed.add((target.package, target.name))
        _target_source_violations(
            _ResolvedOverride(index, target, parsed, source_of(target)),
            violations,
        )
    return claimed


def _expected_override_targets(
    targets: typ.Sequence[CargoTestTarget], source_of: _SourceCache
) -> set[tuple[str, str]]:
    """Return the trybuild-driving targets that are not crate-level Loom-gated."""
    return {
        (target.package, target.name)
        for target in targets
        if drives_trybuild(source_of(target)) and not is_loom_gated(source_of(target))
    }


def _target_set_violations(
    targets: typ.Sequence[CargoTestTarget],
    source_of: _SourceCache,
    claimed: set[tuple[str, str]],
    violations: list[str],
) -> None:
    """Require the claimed targets to equal the expected ones exactly."""
    expected = _expected_override_targets(targets, source_of)
    for package, name in sorted(expected - claimed):
        violations.append(
            f"trybuild target {package}/{name} has no nextest override"
        )
    for package, name in sorted(claimed - expected):
        violations.append(
            f"override target {package}/{name} is not an uncovered "
            "trybuild target"
        )


def override_target_violations(
    config: dict[str, typ.Any],
    targets: typ.Sequence[CargoTestTarget],
    read_source: typ.Callable[[Path], str],
) -> list[str]:
    """Return every way the overrides fail to name the targets they must.

    Each override must resolve to exactly one discovered test target whose
    package and binary match, whose source defines the named ``#[test]``
    function, and whose source drives ``trybuild``. The set of override
    targets must equal the set of trybuild-driving targets that are not
    crate-level Loom-gated, so neither an extra override nor an uncovered
    trybuild target can pass.

    Parameters
    ----------
    config
        The parsed ``.config/nextest.toml``.
    targets
        The test targets ``cargo metadata`` discovered.
    read_source
        Reads one target's source text. Injected so the function stays pure
        and a test can supply a synthetic target.

    Returns
    -------
    list[str]
        Violations; empty means the overrides are proved to apply.
    """
    default = config.get("profile", {}).get("default", {})
    overrides = default.get("overrides", [])
    if not isinstance(overrides, list):
        return ["profile.default.overrides is not a list"]
    violations: list[str] = []
    source_of = _SourceCache(read_source)
    claimed = _override_violations(overrides, targets, source_of, violations)
    _target_set_violations(targets, source_of, claimed, violations)
    return violations
