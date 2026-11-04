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

#: Dylint tuning keys that weaken every lint at once. A shared threshold, locale
#: or test-attribute list is a policy decision rather than a local exception, so
#: the contract rejects it instead of trying to trace an approver.
REPOSITORY_WIDE_TUNING_KEYS = frozenset(
    {
        "additional_test_attributes",
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
#: A diagnostic identifier such as ``no_std_fs_operations``. Prose does not use
#: underscored words, so a match is a reliable sign that the comment names the
#: lint that produced the diagnostic.
DIAGNOSTIC_ID_RE = re.compile(r"\b[a-z][a-z0-9]*(?:_[a-z0-9]+)+\b")
ISSUE_REFERENCE_RE = re.compile(r"(?:#\d+|https://\S+/issues/\d+)")
#: An ``excluded_paths`` assignment, including a quoted key, a dotted key or an
#: inline-table member, but never a commented-out example.
EXCLUDED_PATHS_KEY_RE = re.compile(r"""^[^#]*\b["']?excluded_paths["']?\s*=""")
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

    Parameters
    ----------
    lines : list[str]
        The configuration file split into lines.
    index : int
        Zero-based position of the key whose evidence is sought.

    Returns
    -------
    str
        The joined, lower-cased comment text, or an empty string when the key
        has no comment above it.
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
    """Yield ``(path, key, value)`` for every entry, descending into tables.

    Parameters
    ----------
    table : dict[str, object]
        A parsed TOML table, such as the ``dylint`` root or one of its
        sub-tables.
    path : str
        Dotted path of ``table`` from the document root, used as the prefix of
        every yielded entry.

    Yields
    ------
    tuple[str, str, object]
        The dotted table path, the bare key, and its value. A nested table
        yields its own entry first and then the entries beneath it.
    """
    for key, value in table.items():
        yield path, key, value
        match value:
            case dict():
                yield from iter_config_entries(value, f"{path}.{key}")


def crate_wide_exclusions(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return every ``excluded_crates`` key, which exempts a whole crate.

    Parameters
    ----------
    entries : list[tuple[str, str, object]]
        Flattened configuration entries from :func:`iter_config_entries`.

    Returns
    -------
    list[str]
        Dotted names of the offending keys; empty when the policy holds.
    """
    return [f"{path}.{key}" for path, key, _ in entries if key == "excluded_crates"]


def repository_wide_tuning(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return every key that tunes a lint for the whole repository.

    Parameters
    ----------
    entries : list[tuple[str, str, object]]
        Flattened configuration entries from :func:`iter_config_entries`.

    Returns
    -------
    list[str]
        Dotted names of the offending keys; empty when the policy holds.
    """
    return [
        f"{path}.{key}" for path, key, _ in entries if key in REPOSITORY_WIDE_TUNING_KEYS
    ]


def misplaced_exclusion_keys(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return exclusion keys that sit under a lint which ignores them.

    Parameters
    ----------
    entries : list[tuple[str, str, object]]
        Flattened configuration entries from :func:`iter_config_entries`.

    Returns
    -------
    list[str]
        Dotted names of the misplaced keys; empty when every key sits under
        :data:`EXCLUSION_LINT`.
    """
    permitted = f"dylint.{EXCLUSION_LINT}"
    return [
        f"{path}.{key}"
        for path, key, _ in entries
        if key in {"excluded_paths", "excluded_crates"} and path != permitted
    ]


def widened_exclusions(entries: list[tuple[str, str, object]]) -> list[str]:
    """Return ``excluded_paths`` values that are not lists of module paths.

    Parameters
    ----------
    entries : list[tuple[str, str, object]]
        Flattened configuration entries from :func:`iter_config_entries`.

    Returns
    -------
    list[str]
        A description of every scalar value and every list member that is not
        a module path; empty when the policy holds.
    """
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
    """Require evidence above every ``excluded_paths`` assignment.

    Parameters
    ----------
    lines : list[str]
        The configuration file split into lines.

    Raises
    ------
    AssertionError
        When a key lacks the reproduced diagnostic, the external constraint,
        the owning issue, or any comment block at all.
    """
    unjustified = []
    for index, line in enumerate(lines):
        if not EXCLUDED_PATHS_KEY_RE.match(line):
            continue
        evidence = preceding_comment_block(lines, index)
        missing = [item for item in EXCLUSION_EVIDENCE if item not in evidence]
        if not DIAGNOSTIC_ID_RE.search(evidence):
            missing.append("a diagnostic identifier")
        if not ISSUE_REFERENCE_RE.search(evidence):
            missing.append("an owning issue")
        if missing:
            unjustified.append(f"line {index + 1} lacks {missing}")
    assert not unjustified, (
        "every exclusion needs a comment naming the reproduced diagnostic, the "
        f"external constraint and the issue that removes it: {unjustified}"
    )


def _assert_absent(findings: list[str], message: str) -> None:
    """Fail with ``message`` when the contract found anything to reject."""
    assert not findings, f"{message}: {findings}"


def assert_dylint_policy(source: str) -> None:
    """Assert the exclusion policy encoded in ``dylint.toml``.

    Parameters
    ----------
    source : str
        The full text of the configuration file.

    Raises
    ------
    AssertionError
        When any rule in this module rejects a shape in ``source``.
    """
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