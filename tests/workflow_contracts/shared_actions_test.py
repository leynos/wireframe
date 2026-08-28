"""Contract-test shared action invocations across every workflow.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from collections.abc import Iterator
from pathlib import Path

import yaml

WORKFLOWS_DIR = Path(__file__).resolve().parents[2] / ".github" / "workflows"
WORKFLOW_SUFFIXES = frozenset({".yaml", ".yml"})
SHARED_ACTION_USES_RE = re.compile(
    r"^leynos/shared-actions/\.github/(?:actions/[a-z0-9][a-z0-9-]*|"
    r"workflows/[a-z0-9][a-z0-9-]*\.yml)@(?P<version>[0-9a-f]{40})$"
)


def _workflow_mappings(value: object) -> Iterator[dict[str, object]]:
    """Yield every mapping nested beneath a parsed workflow document."""
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from _workflow_mappings(child)
    elif isinstance(value, list):
        for child in value:
            yield from _workflow_mappings(child)


def _shared_action_invocations() -> list[tuple[Path, str]]:
    """Return each shared-action invocation and its containing workflow."""
    invocations = []
    for workflow_path in sorted(WORKFLOWS_DIR.iterdir()):
        if workflow_path.suffix not in WORKFLOW_SUFFIXES:
            continue
        workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        assert isinstance(workflow, dict), (
            f"{workflow_path.name} must be a mapping"
        )
        for mapping in _workflow_mappings(workflow):
            uses = mapping.get("uses")
            if isinstance(uses, str) and uses.startswith(
                "leynos/shared-actions/"
            ):
                invocations.append((workflow_path, uses))
    assert invocations, (
        "at least one workflow must invoke leynos/shared-actions"
    )
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
