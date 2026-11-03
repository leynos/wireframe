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
satisfy ``test(=compile_tests)``. Comments and literals are stripped first, so
a commented-out definition is not evidence either.

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

#: The items that change a test's module path or name it. ``mod NAME {``
#: consumes its opening brace so the module block is counted once.
_ITEM_TOKEN: typ.Final = re.compile(
    r"""
    (?P<module>\bmod\s+(?P<module_name>[A-Za-z_][A-Za-z0-9_]*)\s*\{)
    | (?P<open>\{)
    | (?P<close>\})
    | (?P<test>\#\s*\[\s*test\s*\])
    | (?P<function>\bfn\s+(?P<function_name>[A-Za-z_][A-Za-z0-9_]*))
    """,
    re.VERBOSE,
)

#: The start of a raw string literal, including its hash fence.
_RAW_STRING: typ.Final = re.compile(r'r(?P<hashes>#{0,32})"')


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


def _skip_line_comment(source: str, index: int) -> int:
    """Return the index of the newline ending a ``//`` comment."""
    newline = source.find("\n", index + 2)
    return len(source) if newline == -1 else newline


def _skip_block_comment(source: str, index: int) -> int:
    """Return the index just past a nestable ``/* ... */`` comment."""
    depth = 1
    index += 2
    while index < len(source) and depth:
        if source.startswith("/*", index):
            depth += 1
            index += 2
        elif source.startswith("*/", index):
            depth -= 1
            index += 2
        else:
            index += 1
    return index


def _skip_string(source: str, index: int) -> int:
    """Return the index just past a standard ``"..."`` literal."""
    index += 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
        elif source[index] == '"':
            return index + 1
        else:
            index += 1
    return index


def _follows_identifier(source: str, index: int) -> bool:
    """Return whether the character before ``index`` could end an identifier."""
    if index == 0:
        return False
    previous = source[index - 1]
    return previous.isalnum() or previous == "_"


def _skip_raw_string(source: str, index: int) -> int | None:
    """Return the index past a raw string, or ``None`` if this is not one."""
    if _follows_identifier(source, index):
        return None
    match = _RAW_STRING.match(source, index)
    if match is None:
        return None
    terminator = '"' + match.group("hashes")
    end = source.find(terminator, match.end())
    return len(source) if end == -1 else end + len(terminator)


def _skip_char(source: str, index: int) -> int | None:
    """Return the index past a character literal, or ``None`` for a lifetime."""
    if index + 1 >= len(source):
        return None
    if source[index + 1] == "\\":
        end = source.find("'", index + 2)
        return None if end == -1 else end + 1
    if index + 2 < len(source) and source[index + 2] == "'":
        return index + 3
    return None


def _char_replacement(source: str, index: int) -> tuple[str, int]:
    """Return the replacement for a lifetime start or a character literal."""
    end = _skip_char(source, index)
    if end is None:
        return source[index], index + 1
    return "' '", end


def _literal_replacement(source: str, index: int) -> tuple[str, int] | None:
    """Return the text replacing the comment or literal at ``index``."""
    if source.startswith("//", index):
        return " ", _skip_line_comment(source, index)
    if source.startswith("/*", index):
        return " ", _skip_block_comment(source, index)
    if source[index] == '"':
        return '""', _skip_string(source, index)
    if source[index] == "'":
        return _char_replacement(source, index)
    end = _skip_raw_string(source, index)
    if end is None:
        return None
    return '""', end


def _without_comments_and_literals(source: str) -> str:
    """Return ``source`` with comments and literals replaced by spaces.

    Discovery runs over the result, so a commented-out ``#[test]`` or a string
    that happens to contain ``trybuild::TestCases`` cannot count as evidence.
    """
    pieces: list[str] = []
    index = 0
    while index < len(source):
        replacement = _literal_replacement(source, index)
        if replacement is None:
            pieces.append(source[index])
            index += 1
        else:
            text, index = replacement
            pieces.append(text)
    return "".join(pieces)


def qualified_test_names(source: str) -> frozenset[str]:
    """Return the module-qualified names of the tests nextest would list.

    Nextest names a test by its module path, so an inline ``mod contracts {
    #[test] fn compile_tests() }`` is ``contracts::compile_tests``. A
    ``#[test]`` inside any other block (an ``impl`` or a function body) is
    not a test nextest runs, so it contributes no name. Comments and literals
    are stripped first, so a commented-out definition contributes nothing.

    Examples
    --------
    >>> sorted(qualified_test_names("#[test]\\nfn a() {}"))
    ['a']
    >>> sorted(qualified_test_names("mod m { #[test] fn b() {} }"))
    ['m::b']
    """
    text = _without_comments_and_literals(source)
    blocks: list[str | None] = []
    paths: set[str] = set()
    is_test_pending = False
    for match in _ITEM_TOKEN.finditer(text):
        if match.group("module"):
            blocks.append(match.group("module_name"))
            is_test_pending = False
        elif match.group("open"):
            blocks.append(None)
            is_test_pending = False
        elif match.group("close"):
            if blocks:
                blocks.pop()
            is_test_pending = False
        elif match.group("test"):
            is_test_pending = True
        elif match.group("function") and is_test_pending:
            is_test_pending = False
            modules = [block for block in blocks if block is not None]
            if len(modules) == len(blocks):
                paths.add("::".join([*modules, match.group("function_name")]))
    return frozenset(paths)


def drives_trybuild(source: str) -> bool:
    """Return whether live code in ``source`` constructs ``trybuild::TestCases``."""
    return _TRYBUILD.search(_without_comments_and_literals(source)) is not None


def is_loom_gated(source: str) -> bool:
    """Return whether live code in ``source`` carries ``#![cfg(loom)]``."""
    return _LOOM_GATE.search(_without_comments_and_literals(source)) is not None


def defines_test_function(source: str, name: str) -> bool:
    """Return whether ``source`` defines a crate-root ``#[test] fn name``.

    Nextest matches ``test(=NAME)`` against the fully qualified name, so a
    definition inside an inline module is ``module::NAME`` and the exact
    matcher does not select it. Comments and literals are ignored, so a
    commented-out definition is not evidence.

    Examples
    --------
    >>> defines_test_function("#[test]\\nfn compile_tests() {}", "compile_tests")
    True
    >>> defines_test_function("mod m { #[test] fn compile_tests() {} }", "compile_tests")
    False
    """
    return name in qualified_test_names(source)


def test_function_note(source: str, name: str) -> str:
    """Explain a definition the filter's exact name does not reach."""
    qualified = sorted(
        path
        for path in qualified_test_names(source)
        if path.rsplit("::", 1)[-1] == name
    )
    if not qualified:
        return ""
    return (
        "; it is defined only as "
        + ", ".join(qualified)
        + ", which nextest names by module path"
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