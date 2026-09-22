"""CV-005: CodeScene belongs to the main publisher, never to a pull request.

The rule this asserts is an estate rule (concordat CV-005), and the reason is
operational rather than stylistic. The CodeScene command-line tool is
installed from a URL at job time. The archive itself is pinned, since the
shared action picks it from a committed manifest and verifies its digest; what
is not pinned is what that archive talks to. The tool calls CodeScene's API and
refuses to run when the answer changes shape, which has happened twice: its
output format moved, and more recently projects stopped returning a gates
configuration at all. Either way a pull-request lane goes red for a reason no
change in the repository could have caused. On 2026-09-16 one such move reddened
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

#: The lane a reviewer's coverage number comes from, and the action that
#: produces it. Removing CodeScene from here must not remove the ratchet too.
RATCHET_LANE: typ.Final = "ci.yml"

#: The condition that step carries, pinned by value. ``ci.yml`` runs on push
#: to main as well as on pull requests, and main-branch coverage is
#: ``coverage-main.yml``'s job, so the generation step is deliberately
#: pull-request-only here. Pinned rather than merely permitted, because any
#: condition is a way to switch the ratchet off while the step stays visible
#: and every other assertion still sees it: ``if: false`` would read as
#: present and run never.
RATCHET_CONDITION: typ.Final = "github.event_name == 'pull_request'"
COVERAGE_ACTION: typ.Final = (
    "leynos/shared-actions/.github/actions/generate-coverage"
)

#: The repository variable that fed the uploader's old ``installer-checksum``
#: input. Nothing may read or refresh it: the uploader takes its digest from a
#: committed manifest, and a stale variable is a value nobody maintains.
RETIRED_VARIABLE: typ.Final = "CODESCENE_CLI_SHA256"

#: The ref the publisher may upload for. Compared in full rather than by
#: suffix: a branch named ``not-main`` ends in ``main``.
TRUNK_REF: typ.Final = "refs/heads/main"

#: The branch the publisher runs on, as ``push.branches`` must list it.
TRUNK_BRANCH: typ.Final = "main"

#: The upload step's condition, compared whole. A substring test would accept
#: ``(github.ref == 'refs/heads/main' || true) && (env.CS_ACCESS_TOKEN != ''
#: || true)``, which contains both halves and is true everywhere.
EXPECTED_UPLOAD_CONDITION: typ.Final = (
    f"env.{TOKEN} != '' && github.ref == '{TRUNK_REF}'"
)


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


#: Both prefixes GitHub accepts for a reusable workflow in this repository.
#: ``$/`` is the documented and recommended form and takes no ``@ref``; ``./``
#: is the older one. Reading only ``./`` would let a caller written the
#: recommended way slip out of every assertion here.
LOCAL_USES_PREFIXES: typ.Final = ("./", "$/")


def _local_callees(document: dict[str, object]) -> set[str]:
    """Return the workflows in this repository that one workflow calls.

    A ``jobs.<id>.uses`` beginning with ``./`` or ``$/`` names a workflow in
    this repository. Anything else carries ``{owner}/{repo}`` and is a
    cross-repository reference, which is somebody else's document to police.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    set[str]
        File names of the local reusable workflows this one calls.
    """
    callees: set[str] = set()
    for definition in (document.get("jobs") or {}).values():
        uses = str((definition or {}).get("uses", ""))
        if uses.startswith(LOCAL_USES_PREFIXES):
            callees.add(uses.split("@")[0].rsplit("/", 1)[-1])
    return callees


def pull_request_workflows() -> list[str]:
    """Return every workflow a pull request can reach, callees included.

    ``pull_request_target`` counts as a root: it runs on a pull request with
    write permissions, which is more dangerous rather than less.

    The closure matters as much as the roots. ``release-dry-run.yml`` is
    triggered by ``pull_request`` and calls ``release.yml``, which calls
    ``build-and-package.yml``. Reading only the roots would leave both of
    those outside every assertion here while a pull request still runs them,
    so a CodeScene step could live in either and nothing would say so.

    Returns
    -------
    list[str]
        File names, sorted.
    """
    documents = dict(_documents())
    pending = [
        name
        for name, document in documents.items()
        if {"pull_request", "pull_request_target"} & set(_triggers(document))
    ]
    reached: set[str] = set()
    # A visited set rather than recursion depth: two reusable workflows that
    # call each other would otherwise loop here, and a contract that hangs is
    # worse than one that is wrong.
    while pending:
        name = pending.pop()
        if name in reached or name not in documents:
            continue
        reached.add(name)
        pending.extend(_local_callees(documents[name]))
    return sorted(reached)


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
    whose path names CodeScene. Such a step runs a tool that calls a remote
    service at job time, so a change nobody in this repository made can fail
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
    triggers = _triggers(documents[PUBLISHER])
    assert "push" in triggers, f"{PUBLISHER} must run on push to main"
    branches = (triggers.get("push") or {}).get("branches")
    # Without this the publisher could run on every branch push and upload
    # each one as the trunk's coverage. The ref guard on the step would
    # refuse the upload, but the lane would burn a runner every time and the
    # two protections would disagree about what this workflow is for.
    assert branches == [TRUNK_BRANCH], (
        f"{PUBLISHER}'s push trigger must be restricted to "
        f"[{TRUNK_BRANCH!r}]; got {branches!r}"
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


def test_the_publisher_only_uploads_from_the_trunk() -> None:
    """Scenario: the publisher is dispatched from a feature branch.

    Invariant: the upload step's condition names the trunk ref as well as the
    token. ``workflow_dispatch`` can be run from any branch, and CodeScene
    accepts an upload for the analysed branch whatever the payload came from,
    so without the ref test a dispatch from a feature branch publishes that
    branch's coverage as the trunk's and moves the ratchet baseline with it.
    Nothing reports that, which is why it is asserted rather than trusted.
    """
    documents = dict(_documents())
    uploads = [
        step
        for step in _steps(documents[PUBLISHER])
        if CODESCENE_ACTION_MARKER in str(step.get("uses", "")).lower()
    ]
    assert len(uploads) == 1, (
        f"{PUBLISHER} should hold exactly one CodeScene step, found "
        f"{len(uploads)}"
    )
    condition = str(uploads[0].get("if", ""))
    # Compared whole, not by substring. Both halves appear in
    # ``(github.ref == 'refs/heads/main' || true) && (env.CS_ACCESS_TOKEN !=
    # '' || true)``, which is true on every branch, so a containment test
    # would pass the very expression it exists to refuse.
    assert condition == EXPECTED_UPLOAD_CONDITION, (
        f"{PUBLISHER}'s upload runs when {condition!r}; the reviewed "
        f"condition is {EXPECTED_UPLOAD_CONDITION!r}. Both halves are "
        "load-bearing: the ref test stops a dispatch from a feature branch "
        "publishing that branch's coverage as the trunk's, and the token "
        "test keeps a secret-less environment from failing the lane."
    )


def test_the_publisher_serializes_and_is_not_cancelled() -> None:
    """Scenario: two pushes to the trunk upload at once.

    Invariant: the publisher declares a concurrency group keyed on the ref,
    and does not cancel a run in progress. Concurrent uploads advance the
    ratchet baseline from whichever finishes last, which need not be the later
    commit; and a cancelled run leaves the baseline describing a commit that
    is no longer the tip, with nothing to say so.
    """
    documents = dict(_documents())
    concurrency = documents[PUBLISHER].get("concurrency")
    assert isinstance(concurrency, dict), (
        f"{PUBLISHER} must declare a concurrency block so two uploads cannot "
        f"race; got {concurrency!r}"
    )
    group = str(concurrency.get("group", ""))
    assert "github.ref" in group, (
        f"{PUBLISHER}'s concurrency group is {group!r}, which does not vary "
        "by ref, so a dispatch elsewhere would queue behind the trunk's run"
    )
    assert concurrency.get("cancel-in-progress") is False, (
        f"{PUBLISHER} must not cancel a run in progress: a cancelled upload "
        "leaves the ratchet baseline describing a commit that is no longer "
        f"the tip; got {concurrency.get('cancel-in-progress')!r}"
    )


def test_the_pull_request_lane_still_ratchets_coverage() -> None:
    """Scenario: CodeScene leaves and the ratchet leaves with it.

    Invariant: the pull-request lane is still started by ``pull_request`` and
    still runs ``generate-coverage`` with ``with-ratchet``. Everything else
    here forbids things, so all of it is satisfied by a repository that
    measures no coverage at all. This is the half that says what must remain:
    the ratchet is ours, runs offline, and is what a reviewer's number
    actually comes from once the CodeScene step is gone.
    """
    documents = dict(_documents())
    assert RATCHET_LANE in documents, f"{RATCHET_LANE} must exist"
    assert "pull_request" in _triggers(documents[RATCHET_LANE]), (
        f"{RATCHET_LANE} must still be started by a pull request"
    )
    generators = [
        step
        for step in _steps(documents[RATCHET_LANE])
        if str(step.get("uses", "")).split("@")[0] == COVERAGE_ACTION
    ]
    assert len(generators) == 1, (
        f"{RATCHET_LANE} should run {COVERAGE_ACTION} exactly once, found "
        f"{len(generators)}"
    )
    assert generators[0].get("with", {}).get("with-ratchet") == "true", (
        f"{RATCHET_LANE}'s coverage step must set with-ratchet, or it "
        "produces a report nothing compares against"
    )
    # Presence is not reachability. `if: false` leaves the step in the file,
    # where every check above still sees it, and runs it never, so the
    # condition is compared by value rather than merely tolerated.
    assert generators[0].get("if") == RATCHET_CONDITION, (
        f"{RATCHET_LANE}'s coverage step runs when "
        f"{generators[0].get('if')!r}; the reviewed condition is "
        f"{RATCHET_CONDITION!r}. Any other condition can disable the ratchet "
        "while leaving the step visible in the diff."
    )


def test_nothing_reads_or_refreshes_the_retired_variable() -> None:
    """Scenario: the CodeScene CLI digest variable outlives its reader.

    Invariant: no workflow in the repository mentions
    ``CODESCENE_CLI_SHA256``, at any scope, in a step input or in a script.

    The variable existed to feed the uploader's ``installer-checksum`` input.
    The uploader now takes its digest from a committed manifest, so the input
    is gone and the variable feeds nothing. A workflow that refreshes it
    spends a runner maintaining a value nobody reads, and one that passes it
    hands the uploader an input it refuses.

    Every workflow is read, not only the pull-request closure: a refresher on
    a schedule or a dispatch is exactly the shape this is meant to catch, and
    neither is reachable from a pull request.
    """
    offenders = sorted(
        name
        for name, document in _documents()
        if RETIRED_VARIABLE in str(document)
    )
    assert not offenders, (
        f"{RETIRED_VARIABLE} is retired and feeds nothing, but "
        f"{offenders} still mention it. The repository variable itself can "
        "be deleted once no workflow names it."
    )
