"""Prove that a nextest override names a target Cargo actually builds.

A nextest override names a package, a binary and a test. Nothing in nextest's
own configuration proves that pair exists: a target renamed, or moved from
``tests/name.rs`` to ``tests/name/mod.rs``, silently stops matching and the
test falls back to the short allowance. ``override_target_violations`` holds
the filter against the targets ``cargo metadata`` reports, so that rot fails a
contract instead of passing unnoticed.

The test-function check is source-level: it proves the named ``#[test]``
function is written in the target's source, not that nextest's own test list
contains it. Nextest test-list evidence is deliberately not used, because
producing it would run the very compilation this contract exists to bound.

The timeout tier chain lives in ``timeout_policy``; the two modules are split
by role rather than by size, and both are called from the tests beside them.
"""

from __future__ import annotations

import dataclasses
import re
import typing as typ

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

#: A trybuild driver, however the path is spaced.
_TRYBUILD: typ.Final = re.compile(r"trybuild\s*::\s*TestCases")

#: A crate-level Loom gate. Such a target only builds under ``--cfg loom``, so
#: it is not part of the ordinary nextest run the override bounds.
_LOOM_GATE: typ.Final = re.compile(r"#!\s*\[\s*cfg\s*\(\s*loom\s*\)\s*\]")

#: A ``#[test]`` attribute, which may be followed by further attributes.
_TEST_ATTRIBUTE: typ.Final = re.compile(r"#\s*\[\s*test\s*\]")


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


def drives_trybuild(source: str) -> bool:
    """Return whether ``source`` constructs ``trybuild::TestCases``."""
    return _TRYBUILD.search(source) is not None


def is_loom_gated(source: str) -> bool:
    """Return whether ``source`` carries a crate-level ``#![cfg(loom)]``."""
    return _LOOM_GATE.search(source) is not None


def defines_test_function(source: str, name: str) -> bool:
    """Return whether ``source`` writes ``#[test] fn name``.

    Attributes may sit between the ``#[test]`` attribute and the function, so
    a ``#[cfg(...)]``-gated test is still recognised.
    """
    tail_pattern = re.compile(
        rf"(?:\s|//[^\n]*\n|#!?\[[^\]]*\]\s*)*fn\s+{re.escape(name)}\b"
    )
    return any(
        tail_pattern.match(source[match.end() :])
        for match in _TEST_ATTRIBUTE.finditer(source)
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
    violations: list[str] = []
    sources: dict[Path, str] = {}

    def source_of(target: CargoTestTarget) -> str:
        if target.src_path not in sources:
            sources[target.src_path] = read_source(target.src_path)
        return sources[target.src_path]

    default = config.get("profile", {}).get("default", {})
    overrides = default.get("overrides", [])
    if not isinstance(overrides, list):
        return ["profile.default.overrides is not a list"]

    claimed: set[tuple[str, str]] = set()
    for index, override in enumerate(overrides):
        raw_filter = override.get("filter")
        if not isinstance(raw_filter, str):
            violations.append(f"override {index} has no string filter")
            continue
        try:
            parsed = parse_target_filter(raw_filter)
        except FilterSyntaxError as exc:
            violations.append(f"override {index} filter {raw_filter!r}: {exc}")
            continue
        matches = [
            target
            for target in targets
            if target.package == parsed.package and target.name == parsed.binary
        ]
        if len(matches) != 1:
            violations.append(
                f"override {index} names {parsed.package}/{parsed.binary}, "
                f"which resolves to {len(matches)} discovered test targets"
            )
            continue
        target = matches[0]
        claimed.add((target.package, target.name))
        source = source_of(target)
        if not defines_test_function(source, parsed.test):
            violations.append(
                f"override {index} names test {parsed.test!r}, which "
                f"{target.src_path} does not define with #[test]"
            )
        if not drives_trybuild(source):
            violations.append(
                f"override {index} names {target.src_path}, which does not "
                "drive trybuild::TestCases"
            )

    expected = {
        (target.package, target.name)
        for target in targets
        if drives_trybuild(source_of(target)) and not is_loom_gated(source_of(target))
    }
    for package, name in sorted(expected - claimed):
        violations.append(
            f"trybuild target {package}/{name} has no nextest override"
        )
    for package, name in sorted(claimed - expected):
        violations.append(
            f"override target {package}/{name} is not an uncovered "
            "trybuild target"
        )
    return violations