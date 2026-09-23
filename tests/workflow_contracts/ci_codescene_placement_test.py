"""CV-005: CodeScene belongs to the main publisher, never to a pull request.

The rule this asserts is an estate rule (concordat CV-005), and the reason is
operational rather than stylistic. The CodeScene command-line tool is
installed from a URL at job time. The archive itself is pinned, since the
shared action picks it from a committed manifest and verifies its digest; what
is not pinned is what that archive talks to. The tool calls CodeScene's API and
refuses to run when the answer changes shape, which has happened twice: its
output format moved, and more recently projects stopped returning a gates
configuration at all. Either way a pull-request lane goes red for a reason no
change in the repository could have caused. On 2026-09-16 one such move
reddened every branch in several repositories at once.

So a pull-request lane may generate coverage, because the ratchet is
repository-owned and runs offline, but it may not talk to CodeScene. The
upload happens once, on push to main, where a failure delays a report instead
of blocking a merge.
The changed-line gate a reviewer sees on the pull request is CodeScene's own
check against what that upload produced; it needs no step here.

Six things make a workflow guilty, and the contract reads all of them
rather than only the obvious one:

- a step or job that ``uses:`` anything whose path names CodeScene;
- a ``run:`` step invoking ``cs-coverage``, which is how the tool is reached
  when nobody wants an action;
- ``CS_ACCESS_TOKEN`` appearing anywhere in the document, at workflow, job or
  step scope, in a ``run`` body, an input or a ``secrets:`` forwarding,
  because a lane holding the token is a lane one line away from using it;
- ``codescene.io`` appearing anywhere, since ``curl`` needs neither the action
  nor the tool; and
- ``secrets: inherit`` into another repository's workflow, which forwards the
  token without naming it to a document this contract cannot read; and
- a call to this repository's own workflow by ``owner/repo`` and ``@ref``,
  which runs the file at that ref rather than the one the closure reads.

"A workflow a pull request can start" is a closure, not a trigger list: every
clause runs over each pull-request workflow and everything it calls in this
repository, transitively. How that closure and the other readings are built
is in ``codescene_placement_reader``, and each reading is proved against a
constructed tree in ``codescene_placement_reader_test``.

The publisher's own shape, including the token confined to its upload step,
is asserted in ``codescene_publisher_test``, and the reviewed values both
modules hold the tree to live in ``codescene_placement_policy``.
"""

from __future__ import annotations

import functools

import pytest
from codescene_placement_policy import (
    CODESCENE_ACTION_MARKER,
    CODESCENE_COMMAND_MARKER,
    CODESCENE_HOST,
    COVERAGE_ACTION,
    PUBLISHER,
    RATCHET_CONDITION,
    RATCHET_LANE,
    TOKEN,
)
from codescene_placement_reader import (
    calls,
    external_secret_inheritors,
    mentions,
    pull_request_closure,
    qualified_self_callers,
    steps,
    triggers,
)
from workflow_loader import repository_workflows

_documents = repository_workflows


@functools.cache
def pull_request_workflows() -> list[str]:
    """Return every workflow a pull request can reach, callees included.

    The closure matters as much as the roots. No workflow here calls another
    today, but a ``workflow_call`` workflow called from a pull-request job
    runs on that pull request, with the token if the caller writes
    ``secrets: inherit``. Reading only the roots would leave it outside every
    assertion here while a pull request still runs it.
    """
    return pull_request_closure(_documents())


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_uses_a_codescene_action(name: str) -> None:
    """Scenario: a pull-request lane calls a CodeScene action.

    Invariant: no workflow a pull request can start references an action or
    reusable workflow whose path names CodeScene. Such a step runs a tool
    that calls a remote service at job time, so a change nobody in this
    repository made can fail the lane and block the merge.
    """
    offenders = [
        str(call.get("uses"))
        for call in calls(_documents()[name])
        if CODESCENE_ACTION_MARKER in str(call.get("uses", "")).lower()
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
    offenders = [
        str(step.get("name", step.get("run")))[:60]
        for step in steps(_documents()[name])
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
    assert not mentions(_documents()[name], TOKEN), (
        f"{name} is a pull-request lane and must not carry {TOKEN} at any "
        f"scope; the token belongs only to {PUBLISHER}"
    )


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_names_the_codescene_host(name: str) -> None:
    """Scenario: a pull-request lane calls CodeScene's API directly.

    Invariant: ``codescene.io`` appears nowhere in a workflow a pull request
    can start. A ``curl`` to the API is the same remote dependency as the
    action with neither the action's path nor the tool's name to notice it
    by, and the token clause above cannot see it when the credential arrives
    under another name.
    """
    assert not mentions(
        _documents()[name], CODESCENE_HOST, ignore_case=True
    ), f"{name} is a pull-request lane and must not reach {CODESCENE_HOST}"


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_inherits_secrets_into_another_repository(
    name: str,
) -> None:
    """Scenario: a pull-request job hands every secret to a foreign workflow.

    Invariant: no job in the closure calls a workflow outside this repository
    with ``secrets: inherit``. Inheritance forwards ``CS_ACCESS_TOKEN``
    without naming it, so the token clause cannot see it, and the callee is a
    document this contract cannot read. A local callee is fine: it is in the
    closure and read like every other workflow here.
    """
    offenders = external_secret_inheritors(_documents()[name])
    assert not offenders, (
        f"{name} is a pull-request lane and must not inherit secrets into "
        f"another repository's workflow; jobs {offenders} do"
    )


@pytest.mark.parametrize("name", pull_request_workflows())
def test_no_pull_request_lane_calls_this_repository_by_ref(name: str) -> None:
    """Scenario: a pull-request job calls this repository's workflow by ref.

    Invariant: no job in the closure writes
    ``leynos/wireframe/.github/workflows/x.yml@main``. GitHub runs that
    call at the named ref, not at the pull request's head, so the file this
    contract reads is not the file that runs: the named revision could hold
    a CodeScene step every clause here passes over. ``./`` runs the
    checked-out file the closure reads.
    """
    offenders = qualified_self_callers(_documents()[name])
    assert not offenders, (
        f"{name} is a pull-request lane; jobs {offenders} call this "
        "repository's workflows by ref, which this contract cannot read"
    )


def test_the_pull_request_lane_still_ratchets_coverage() -> None:
    """Scenario: CodeScene leaves and the ratchet leaves with it.

    Invariant: the pull-request lane is still started by ``pull_request`` and
    still runs ``generate-coverage`` with ``with-ratchet``. Everything else
    here forbids things, so all of it is satisfied by a repository that
    measures no coverage at all. This is the half that says what must remain:
    the ratchet is repository-owned, runs offline, and is what a reviewer's
    number actually comes from once the CodeScene step is gone.
    """
    documents = _documents()
    assert RATCHET_LANE in documents, f"{RATCHET_LANE} must exist"
    assert "pull_request" in triggers(documents[RATCHET_LANE]), (
        f"{RATCHET_LANE} must still be started by a pull request"
    )
    generators = [
        step
        for step in steps(documents[RATCHET_LANE])
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
