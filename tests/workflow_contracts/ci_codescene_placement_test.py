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

Five things make a workflow guilty, and the contract reads all of them
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
  token without naming it to a document this contract cannot read.

"A workflow a pull request can start" is a closure, not a trigger list: every
clause runs over each pull-request workflow and everything it calls in this
repository, transitively. How that closure and the other readings are built
is in ``codescene_placement_reader``, and each reading is proved against a
constructed tree in ``codescene_placement_reader_test``.

On the publisher the token is confined further: it is declared on the upload
step alone, not on the job or the workflow, so checkout, setup and the tests
that generate coverage never hold it.
"""

from __future__ import annotations

import functools
import typing as typ
from pathlib import Path

import pytest
from codescene_placement_reader import (
    Document,
    calls,
    external_secret_inheritors,
    jobs,
    mentions,
    pull_request_closure,
    read_workflows,
    steps,
    triggers,
)

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_DIR: typ.Final = REPO_ROOT / ".github" / "workflows"

#: The one workflow allowed to reach CodeScene, and the trigger that makes it
#: safe: a push lane cannot block a merge.
PUBLISHER: typ.Final = "coverage-main.yml"

#: The token's name. Present anywhere in a pull-request workflow is a failure.
TOKEN: typ.Final = "CS_ACCESS_TOKEN"

#: Matched against an action or reusable-workflow reference, lowercased.
CODESCENE_ACTION_MARKER: typ.Final = "codescene"

#: Matched against a ``run:`` block, lowercased.
CODESCENE_COMMAND_MARKER: typ.Final = "cs-coverage"

#: CodeScene's service host, matched anywhere in a document, lowercased. A
#: ``curl`` to the API needs neither the action nor the command-line tool.
CODESCENE_HOST: typ.Final = "codescene.io"

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

#: The ref the publisher may upload for. Compared in full rather than by
#: suffix: a branch named ``not-main`` ends in ``main``.
TRUNK_REF: typ.Final = "refs/heads/main"

#: The branch the publisher runs on, as ``push.branches`` must list it.
TRUNK_BRANCH: typ.Final = "main"

#: The upload step's condition, compared whole. A substring test would accept
#: ``(github.ref == 'refs/heads/main' || true) && (env.CS_ACCESS_TOKEN != ''
#: || true)``, which contains both halves and is true everywhere, and equally
#: ``... && github.ref == 'refs/heads/main' || github.event_name ==
#: 'workflow_dispatch'``, which makes every conjunct optional.
EXPECTED_UPLOAD_CONDITION: typ.Final = (
    f"env.{TOKEN} != '' && github.ref == '{TRUNK_REF}'"
)


@functools.cache
def _documents() -> dict[str, Document]:
    """Read and parse every workflow once, refusing duplicate keys."""
    documents = read_workflows(WORKFLOW_DIR)
    assert documents, "the repository should define at least one workflow"
    return documents


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


def _uploads(document: Document) -> list[Document]:
    """Return the steps of a workflow that call a CodeScene action."""
    return [
        step
        for step in steps(document)
        if CODESCENE_ACTION_MARKER in str(step.get("uses", "")).lower()
    ]


def _sole_upload() -> Document:
    """Return the publisher's one CodeScene step, asserting there is one."""
    uploads = _uploads(_documents()[PUBLISHER])
    assert len(uploads) == 1, (
        f"{PUBLISHER} should hold exactly one CodeScene step, found "
        f"{len(uploads)}"
    )
    return uploads[0]


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


def test_the_publisher_still_uploads() -> None:
    """Scenario: the rule is satisfied by deleting the upload entirely.

    Invariant: the push-to-main publisher exists, is not a pull-request lane,
    and still calls a CodeScene action. Without this, a repository could pass
    every assertion above by having no coverage reporting at all, which is
    compliance by amputation rather than by placement.
    """
    documents = _documents()
    assert PUBLISHER in documents, f"{PUBLISHER} must exist"
    assert PUBLISHER not in pull_request_workflows(), (
        f"{PUBLISHER} must not be startable by a pull request"
    )
    publisher_triggers = triggers(documents[PUBLISHER])
    assert "push" in publisher_triggers, (
        f"{PUBLISHER} must run on push to main"
    )
    branches = (publisher_triggers.get("push") or {}).get("branches")
    # Without this the publisher could run on every branch push and upload
    # each one as the trunk's coverage. The ref guard on the step would
    # refuse the upload, but the lane would burn a runner every time and the
    # two protections would disagree about what this workflow is for.
    assert branches == [TRUNK_BRANCH], (
        f"{PUBLISHER}'s push trigger must be restricted to "
        f"[{TRUNK_BRANCH!r}]; got {branches!r}"
    )
    assert _sole_upload().get("with", {}).get("mode") == "upload", (
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
    condition = str(_sole_upload().get("if", ""))
    # Compared whole, not by substring or by conjunct. Both halves appear in
    # ``(github.ref == 'refs/heads/main' || true) && (env.CS_ACCESS_TOKEN !=
    # '' || true)``, which is true on every branch, and appending
    # ``|| github.event_name == 'workflow_dispatch'`` keeps every conjunct
    # while making all of them optional. Equality refuses both.
    assert condition == EXPECTED_UPLOAD_CONDITION, (
        f"{PUBLISHER}'s upload runs when {condition!r}; the reviewed "
        f"condition is {EXPECTED_UPLOAD_CONDITION!r}. Both halves are "
        "load-bearing: the ref test stops a dispatch from a feature branch "
        "publishing that branch's coverage as the trunk's, and the token "
        "test keeps a secret-less environment from failing the lane."
    )


def test_the_publisher_holds_the_token_on_the_upload_step_alone() -> None:
    """Scenario: the token sits in the job environment of the publisher.

    Invariant: in ``coverage-main.yml`` the token is declared in the upload
    step's own ``env`` and appears nowhere else: not at workflow scope, not on
    any job, not in any other step. A job-scoped token is readable by every
    step before the ref guard runs, including the tests that generate
    coverage, and a dispatch can run those from any branch.
    """
    document = _documents()[PUBLISHER]
    upload = _sole_upload()
    assert TOKEN in (upload.get("env") or {}), (
        f"{PUBLISHER}'s upload step must declare {TOKEN} in its own env"
    )
    wider = {key: value for key, value in document.items() if key != "jobs"}
    holders = [
        f"job {name}"
        for name, job in jobs(document).items()
        if mentions({k: v for k, v in job.items() if k != "steps"}, TOKEN)
    ] + [
        str(step.get("name", step.get("uses", step.get("run"))))
        for step in steps(document)
        if step is not upload and mentions(step, TOKEN)
    ]
    assert not mentions(wider, TOKEN), (
        f"{PUBLISHER} must not declare {TOKEN} at workflow scope"
    )
    assert not holders, (
        f"{PUBLISHER} must hold {TOKEN} on the upload step alone; also "
        f"found in {holders}"
    )


def test_the_publisher_serializes_and_is_not_cancelled() -> None:
    """Scenario: two pushes to the trunk upload at once.

    Invariant: the publisher declares a concurrency group keyed on the ref,
    and does not cancel a run in progress. Concurrent uploads advance the
    ratchet baseline from whichever finishes last, which need not be the later
    commit; and a cancelled run leaves the baseline describing a commit that
    is no longer the tip, with nothing to say so.
    """
    concurrency = _documents()[PUBLISHER].get("concurrency")
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
