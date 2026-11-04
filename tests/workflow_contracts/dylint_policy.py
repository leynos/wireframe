"""Enforce the repository's Dylint exclusion policy.

The root ``dylint.toml`` is the repository's only lint-exception register.
This module parses it and rejects every shape that would widen an exemption
past the diagnostic it was written for: crate-wide exclusions, exclusion keys
under a lint that does not read them, repository-wide tuning, values that are
not module paths, and entries whose comment block does not name the reproduced
diagnostic, the external constraint and the owning issue.

``dylint_policy_test.py`` drives the accepted and rejected shapes through
``assert_dylint_policy`` and checks the real ``dylint.toml`` against it.
"""

from __future__ import annotations

import re
import tomllib
from collections.abc import Iterator

#: Dylint tuning keys that weaken every lint at once. A shared threshold or
#: locale change is a policy decision rather than a local exception, so the
#: contract rejects it instead of trying to trace an approver.
REPOSITORY_WIDE_TUNING_KEYS = frozenset(
    {
        "extra_test_attributes",
        "locale",
        "locales",
        "test_attributes",
        "threshold",
        "thresholds",
    }
)
#: The pinned suite's only lint that reads ``excluded_paths``. Every other lint
#: rejects unknown keys, so an exclusion placed in their table is silently
#: replaced by that lint's defaults.
EXCLUSION_LINT = "no_std_fs_operations"
#: A fully qualified Rust module path such as ``wireframe::legacy_io``. A bare
#: crate name, a trailing separator, ``.`` or a glob would widen the exception
#: past the module that raised the diagnostic.
MODULE_PATH_RE = re.compile(r"^[a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)+$")
#: Evidence every exclusion must carry in the comment block above it.
EXCLUSION_EVIDENCE = ("reproduced by", "external constraint", "tracked by")
ISSUE_REFERENCE_RE = re.compile(r"(?:#\d+|https://\S+/issues/\d+)")
#: An ``excluded_paths`` assignment, including a dotted key or an inline-table
#: member, but never a commented-out example.
EXCLUDED_PATHS_KEY_RE = re.compile(r"^[^#]*\bexcluded_paths\s*=")
TABLE_HEADER_RE = re.compile(r"^\[[^\]]+\]$")

COMMENT = "comment"
STOP = "stop"
SKIP = "skip"


def _classify_line(stripped: str, has_collected: bool) -> str:
    """Classify one line while walking upwards for the justification comment."""
    if stripped.startswith("#"):
        return COMMENT
    if not stripped:
        return STOP
    if TABLE_HEADER_RE.fullmatch(stripped) and not has_collected:
        return SKIP
    return STOP


def preceding_comment_block(lines: list[str], index: int) -> str:
    """Return the comment text directly above ``lines[index]``.

    A table header between the comment and the key is skipped, so the evidence
    may sit above either the header or the key, matching the example in
    ``dylint.toml``. The walk stops at the first blank or non-comment line.
    """
    collected: list[str] = []
    for line in reversed(lines[:index]):
        stripped = line.strip()
        outcome = _classify_line(stripped, bool(collected))
        if outcome == COMMENT:
            collected.append(stripped.lstrip("#").strip())
        elif outcome == STOP:
            break
    return " ".join(reversed(collected)).lower()


def iter_config_entries(
    table: dict[str, object], path: str
) -> Iterator[tuple[str, str, object]]:
    """Yield ``(path, key, value)`` for every entry, descending into tables."""
    for key, value in table.items():
        yield path, key, value
        if isinstance(value, dict):
            yield from iter_config_entries(value, f"{path}.{key}")


def crate_wide_exclusions(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return every ``excluded_crates`` key, which exempts a whole crate."""
    return [f"{path}.{key}" for path, key, _ in entries if key == "excluded_crates"]


def repository_wide_tuning(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return every key that tunes a lint for the whole repository."""
    return [
        f"{path}.{key}" for path, key, _ in entries if key in REPOSITORY_WIDE_TUNING_KEYS
    ]


def misplaced_exclusion_keys(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return exclusion keys that sit under a lint which ignores them."""
    permitted = f"dylint.{EXCLUSION_LINT}"
    return [
        f"{path}.{key}"
        for path, key, _ in entries
        if key in {"excluded_paths", "excluded_crates"} and path != permitted
    ]


def widened_exclusions(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return ``excluded_paths`` values that are not lists of module paths."""
    return [
        f"{path}.{key}={value!r}"
        for path, key, value in entries
        if key == "excluded_paths" and not isinstance(value, list)
    ] + [
        f"{path}.{key}={entry!r}"
        for path, key, value in entries
        if key == "excluded_paths" and isinstance(value, list)
        for entry in value
        if not MODULE_PATH_RE.fullmatch(str(entry))
    ]


def assert_exclusions_are_justified(lines: list[str]) -> None:
    """Require evidence above every ``excluded_paths`` assignment."""
    unjustified = []
    for index, line in enumerate(lines):
        if not EXCLUDED_PATHS_KEY_RE.match(line):
            continue
        evidence = preceding_comment_block(lines, index)
        missing = [item for item in EXCLUSION_EVIDENCE if item not in evidence]
        if missing or not ISSUE_REFERENCE_RE.search(evidence):
            unjustified.append(f"line {index + 1} lacks {missing or ['an owning issue']}")
    assert not unjustified, (
        "every exclusion needs a comment naming the reproduced diagnostic, the "
        f"external constraint and the issue that removes it: {unjustified}"
    )


def _assert_absent(findings: list[str], message: str) -> None:
    """Fail with ``message`` when the contract found anything to reject."""
    assert not findings, f"{message}: {findings}"


def assert_dylint_policy(source: str) -> None:
    """Assert the exclusion policy encoded in ``dylint.toml``."""
    entries = list(iter_config_entries(tomllib.loads(source), "dylint"))

    _assert_absent(
        crate_wide_exclusions(entries),
        "crate-wide exclusions are forbidden; scope excluded_paths instead",
    )
    _assert_absent(
        repository_wide_tuning(entries),
        "repository-wide tuning keys need the ticket owner's approval and are "
        "rejected by this contract",
    )
    _assert_absent(
        widened_exclusions(entries),
        "exclusions must name a module path such as `wireframe::legacy_io`; "
        "these would widen past the diagnostic",
    )
    _assert_absent(
        misplaced_exclusion_keys(entries),
        f"only `{EXCLUSION_LINT}` reads exclusion keys; other lints discard them",
    )

    assert_exclusions_are_justified(source.splitlines())