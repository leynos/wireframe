"""Contract-test shared action invocations across every workflow.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from collections.abc import Iterable, Iterator
from pathlib import Path

import pytest
import yaml

WORKFLOWS_DIR = Path(__file__).resolve().parents[2] / ".github" / "workflows"
WORKFLOW_SUFFIXES = frozenset({".yaml", ".yml"})
SHARED_ACTION_USES_RE = re.compile(
    r"^leynos/shared-actions/\.github/(?:actions/[a-z0-9][a-z0-9-]*|"
    r"workflows/[a-z0-9][a-z0-9-]*\.yml)@(?P<version>[0-9a-f]{40})$"
)


def _workflow_paths(entries: Iterable[Path]) -> Iterator[Path]:
    """Yield supported workflow paths in deterministic order."""
    for workflow_path in sorted(entries):
        if workflow_path.suffix in WORKFLOW_SUFFIXES:
            yield workflow_path


def _shared_action_use(mapping: dict[str, object]) -> str | None:
    """Return a shared-actions invocation from a workflow mapping."""
    uses = mapping.get("uses")
    if isinstance(uses, str) and uses.startswith("leynos/shared-actions/"):
        return uses
    return None


def _workflow_mappings(value: object) -> Iterator[dict[str, object]]:
    """Yield every mapping nested beneath a parsed workflow document."""
    if isinstance(value, dict):
        yield from _mapping_and_children(value)
    if isinstance(value, list):
        yield from _mappings_in_values(value)


def _mapping_and_children(mapping: dict[str, object]) -> Iterator[dict[str, object]]:
    """Yield a mapping before traversing its nested values."""
    yield mapping
    yield from _mappings_in_values(mapping.values())


def _mappings_in_values(values: Iterable[object]) -> Iterator[dict[str, object]]:
    """Yield mappings nested within the supplied workflow values."""
    for value in values:
        yield from _workflow_mappings(value)


def _shared_action_invocations() -> list[tuple[Path, str]]:
    """Return each shared-action invocation and its containing workflow."""
    invocations = []
    try:
        workflow_entries = WORKFLOWS_DIR.iterdir()
    except OSError as error:
        raise AssertionError(
            f"workflow directory {WORKFLOWS_DIR} entries could not be read: {error}"
        ) from error
    for workflow_path in _workflow_paths(workflow_entries):
        workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        assert isinstance(workflow, dict), f"{workflow_path.name} must be a mapping"
        for mapping in _workflow_mappings(workflow):
            uses = _shared_action_use(mapping)
            if uses is not None:
                invocations.append((workflow_path, uses))
    assert invocations, "at least one workflow must invoke leynos/shared-actions"
    return invocations


def _shared_action_versions() -> list[tuple[Path, str]]:
    """Return validated workflow paths and their shared-action versions."""
    versions = []
    for workflow_path, uses in _shared_action_invocations():
        match = SHARED_ACTION_USES_RE.fullmatch(uses)
        assert match, (
            f"{workflow_path.name} must invoke an approved shared action or workflow "
            f"pinned to a 40-character lowercase hex commit SHA, "
            f"not {uses!r}"
        )
        versions.append((workflow_path, match["version"]))
    return versions


def test_workflow_paths_filter_supported_suffixes_in_order(tmp_path: Path) -> None:
    """Workflow path discovery keeps supported files in lexical order."""
    for filename in ("zebra.yml", "notes.txt", "alpha.yaml"):
        (tmp_path / filename).touch()

    assert [path.name for path in _workflow_paths(tmp_path.iterdir())] == [
        "alpha.yaml",
        "zebra.yml",
    ]


def test_shared_action_invocations_explain_unreadable_workflow_directory(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Workflow scanning reports an unreadable workflow directory clearly."""

    def unreadable_directory_entries(_: Path) -> Iterator[Path]:
        raise OSError("access denied")

    monkeypatch.setattr(Path, "iterdir", unreadable_directory_entries)

    with pytest.raises(
        AssertionError,
        match=(
            rf"workflow directory {re.escape(str(WORKFLOWS_DIR))} "
            r"entries could not be read: access denied"
        ),
    ):
        _shared_action_invocations()


@pytest.mark.parametrize(
    ("mapping", "expected"),
    [
        (
            {"uses": "leynos/shared-actions/.github/actions/check"},
            "leynos/shared-actions/.github/actions/check",
        ),
        ({"uses": "actions/checkout@v4"}, None),
        ({}, None),
        ({"uses": 42}, None),
    ],
)
def test_shared_action_use_filters_workflow_mappings(
    mapping: dict[str, object], expected: str | None
) -> None:
    """Shared-action filtering accepts only the expected string prefix."""
    assert _shared_action_use(mapping) == expected


def test_shared_action_invocations_have_expected_shape() -> None:
    """Shared actions use approved paths and immutable full-SHA references."""
    _shared_action_versions()


def test_shared_action_invocations_use_one_consistent_version() -> None:
    """Every workflow invokes the same shared-actions revision."""
    versions = _shared_action_versions()
    distinct_versions = {version for _, version in versions}
    assert len(distinct_versions) == 1, (
        "all leynos/shared-actions invocations must use one version; found "
        + ", ".join(f"{path.name}@{version}" for path, version in versions)
    )
