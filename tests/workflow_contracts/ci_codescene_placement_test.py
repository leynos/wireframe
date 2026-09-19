"""CV-005: CodeScene belongs to the main publisher, never to a pull request.

The rule this asserts is an estate rule (concordat CV-005), and the reason is
operational rather than stylistic. The CodeScene command-line tool is
installed from a URL at job time and is unpinned upstream: its installer and
its output format have both moved without notice, and each time they moved,
every pull-request lane that invoked the tool went red for a reason no change
in the repository could have caused. On 2026-09-16 one such move reddened
every branch in several repositories at once.

So a pull-request lane may generate coverage, because the ratchet is ours and
runs offline, but it may not talk to CodeScene. The upload happens once, on
push to main, where a failure delays a report instead of blocking a merge.
The changed-line gate a reviewer sees on the pull request is CodeScene's own
check against what that upload produced; it needs no step here.

Three things make a workflow guilty, and the contract reads all three rather
than only the obvious one:

- a step that ``uses:`` any action whose path names CodeScene;
- a ``run:`` step invoking ``cs-coverage``, which is how the tool is reached
  when nobody wants an action; and
- ``CS_ACCESS_TOKEN`` appearing anywhere in the document, at workflow, job or
  step scope, because a lane holding the token is a lane one line away from
  using it.

The last is what makes the contract narrow enough to be worth having. Deleting
only the step but leaving the token in the job environment would look clean in
a diff and would leave the hazard in place.
"""

from __future__ import annotations

import functools
import typing as typ
from pathlib import Path

import pytest
import yaml

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_DIR: typ.Final = REPO_ROOT / ".github" / "workflows"

#: Both spellings GitHub accepts for a workflow file's extension.
WORKFLOW_FILE_PATTERNS: typ.Final = ("*.yml", "*.yaml")

#: The one workflow allowed to reach CodeScene, and the trigger that makes it
#: safe: a push lane cannot block a merge.
PUBLISHER: typ.Final = "coverage-main.yml"

#: The token's name. Present anywhere in a pull-request workflow is a failure.
TOKEN: typ.Final = "CS_ACCESS_TOKEN"

#: Matched against an action reference, lowercased.
CODESCENE_ACTION_MARKER: typ.Final = "codescene"

#: Matched against a ``run:`` block, lowercased.
CODESCENE_COMMAND_MARKER: typ.Final = "cs-coverage"


@functools.cache
def _documents() -> tuple[tuple[str, dict[str, object]], ...]:
    """Read and parse every workflow once.

    Returns
    -------
    tuple[tuple[str, dict[str, object]], ...]
        Each workflow's file name with its parsed document, sorted by path.
    """
    paths = sorted(
        path
        for pattern in WORKFLOW_FILE_PATTERNS
        for path in WORKFLOW_DIR.glob(pattern)
    )
    documents = tuple(
        (path.name, yaml.safe_load(path.read_text(encoding="utf-8")) or {})
        for path in paths
    )
    assert documents, "the repository should define at least one workflow"
    return documents


def _triggers(document: dict[str, object]) -> dict[str, object]:
    """Return a workflow's ``on:`` mapping.

    ``on`` is YAML 1.1's boolean ``True`` unless the key was quoted, so both
    spellings are read. A workflow whose triggers are a bare string or a list
    is normalized to a mapping with empty values.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    dict[str, object]
        Trigger name to its configuration.
    """
    raw = document.get("on", document.get(True))
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, list):
        return dict.fromkeys(raw)
    if isinstance(raw, str):
        return {raw: None}
    return {}


def pull_request_workflows() -> list[str]:
    """Return every workflow a pull request can start.

    ``pull_request_target`` counts: it runs on a pull request with write
    permissions, which is more dangerous rather than less.

    Returns
    -------
    list[str]
        File names, sorted.
    """
    return sorted(
        name
        for name, document in _documents()
        if {"pull_request", "pull_request_target"} & set(_triggers(document))
    )


def _steps(document: dict[str, object]) -> list[dict[str, object]]:
    """Return every mapping step in every job of one workflow.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    list[dict[str, object]]
        Steps in document order across all jobs.
    """
    collected: list[dict[str, object]] = []
    for definition in (document.get("jobs") or {}).values():
        for step in (definition or {}).get("steps") or []:
            if isinstance(step, dict):
                collected.append(step)
    return collected


def _mentions_token(node: object) -> bool:
    """Return whether ``CS_ACCESS_TOKEN`` appears anywhere in a document.

    The whole tree is walked rather than the three scopes that are supposed
    to carry it, because the point is that the name must not appear at all.

    Parameters
    ----------
    node
        Any node of a parsed workflow document.

    Returns
    -------
    bool
        True when the token's name occurs in a key or a scalar.
    """
    if isinstance(node, dict):
        return any(
            TOKEN in str(key) or _mentions_token(value)
            for key, value in node.items()
        )
    if isinstance(node, list):
        return any(_mentions_token(item) for item in node)
    return TOKEN in str(node)


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_uses_a_codescene_action(name: str) -> None:
    """Scenario: a pull-request lane calls a CodeScene action.

    Invariant: no workflow a pull request can start references an action
    whose path names CodeScene. Such a step installs an unpinned upstream
    tool at job time, so a change nobody in this repository made can fail
    the lane and block the merge.
    """
    document = dict(_documents())[name]
    offenders = [
        str(step.get("uses"))
        for step in _steps(document)
        if CODESCENE_ACTION_MARKER in str(step.get("uses", "")).lower()
    ]
    assert not offenders, (
        f"{name} is a pull-request lane and must not call a CodeScene "
        f"action; found {offenders}. The upload belongs in {PUBLISHER}."
    )


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_runs_the_codescene_cli(name: str) -> None:
    """Scenario: a pull-request lane shells out to ``cs-coverage``.

    Invariant: no workflow a pull request can start invokes the CodeScene
    command directly. Reaching the tool without an action is the same hazard
    with none of the visibility, so it is read as well.
    """
    document = dict(_documents())[name]
    offenders = [
        str(step.get("name", step.get("run")))[:60]
        for step in _steps(document)
        if CODESCENE_COMMAND_MARKER in str(step.get("run", "")).lower()
    ]
    assert not offenders, (
        f"{name} is a pull-request lane and must not run "
        f"{CODESCENE_COMMAND_MARKER!r}; found {offenders}"
    )


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_receives_the_codescene_token(name: str) -> None:
    """Scenario: the step goes but the token stays.

    Invariant: ``CS_ACCESS_TOKEN`` appears nowhere in a workflow a pull
    request can start, at any scope. Removing the step while leaving the
    token in a job environment reads as clean in a diff and leaves the lane
    one line away from the hazard it was meant to lose.
    """
    document = dict(_documents())[name]
    assert not _mentions_token(document), (
        f"{name} is a pull-request lane and must not carry {TOKEN} at any "
        f"scope; the token belongs only to {PUBLISHER}"
    )


def test_the_publisher_still_uploads() -> None:
    """Scenario: the rule is satisfied by deleting the upload entirely.

    Invariant: the push-to-main publisher exists, is not a pull-request lane,
    and still calls a CodeScene action. Without this, a repository could pass
    every assertion above by having no coverage reporting at all, which is
    compliance by amputation rather than by placement.
    """
    documents = dict(_documents())
    assert PUBLISHER in documents, f"{PUBLISHER} must exist"
    assert PUBLISHER not in pull_request_workflows(), (
        f"{PUBLISHER} must not be startable by a pull request"
    )
    assert "push" in _triggers(documents[PUBLISHER]), (
        f"{PUBLISHER} must run on push to main"
    )
    uploads = [
        step
        for step in _steps(documents[PUBLISHER])
        if CODESCENE_ACTION_MARKER in str(step.get("uses", "")).lower()
    ]
    assert len(uploads) == 1, (
        f"{PUBLISHER} should hold exactly one CodeScene step, found "
        f"{len(uploads)}"
    )
    assert uploads[0].get("with", {}).get("mode") == "upload", (
        f"{PUBLISHER}'s CodeScene step must state mode: upload, so that it "
        "cannot silently become the pull-request check gate"
    )
