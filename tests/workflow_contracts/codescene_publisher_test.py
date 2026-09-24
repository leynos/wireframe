"""CV-005: the push-to-main publisher is the one lane that talks to CodeScene.

``ci_codescene_placement_test`` forbids CodeScene everywhere a pull request
can reach; this module says what the one permitted lane must look like, so
the rule cannot be satisfied by deleting the upload, and so the upload cannot
publish a feature branch's coverage as the trunk's.
"""

from __future__ import annotations

import re

from codescene_placement_policy import (
    ACCESS_TOKEN_INPUT,
    CODESCENE_ACTION_MARKER,
    EXPECTED_UPLOAD_CONDITION,
    PUBLISHER,
    PUBLISHER_CONCURRENCY_GROUP,
    PUBLISHER_TRIGGERS,
    TOKEN,
    TOKEN_CHECK_COMMAND,
    TOKEN_CHECK_STEP_ID,
    TRUNK_BRANCH,
    UPLOADER_ACTION,
)
from codescene_placement_reader import (
    jobs,
    mentions,
    pull_request_closure,
    steps,
    triggers,
)
from workflow_loader import Document, repository_workflows

_documents = repository_workflows


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


def test_the_publisher_still_uploads() -> None:
    """Scenario: the rule is satisfied by deleting the upload entirely.

    Invariant: the push-to-main publisher exists, is not a pull-request lane,
    and still calls a CodeScene action. Without this, a repository could pass
    every assertion above by having no coverage reporting at all, which is
    compliance by amputation rather than by placement.
    """
    documents = _documents()
    assert PUBLISHER in documents, f"{PUBLISHER} must exist"
    assert PUBLISHER not in pull_request_closure(_documents()), (
        f"{PUBLISHER} must not be startable by a pull request"
    )
    publisher_triggers = triggers(documents[PUBLISHER])
    # The whole set, not membership: a pull-request trigger added beside
    # push would make the publisher a pull-request lane, and any other
    # addition or removal changes what the workflow is for unreviewed.
    assert set(publisher_triggers) == PUBLISHER_TRIGGERS, (
        f"{PUBLISHER} must be triggered by exactly "
        f"{sorted(PUBLISHER_TRIGGERS)}; got "
        f"{sorted(map(str, publisher_triggers))}"
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
    """Scenario: a later change widens the publisher's triggers.

    Invariant: the upload step's condition names the trunk ref as well as the
    token. The trigger set is pinned to ``push`` on the trunk today, so the
    ref test is the second protection: if ``workflow_dispatch`` or another
    branch is ever added, CodeScene would accept that branch's coverage as
    the trunk's, whatever the payload came from, and move the ratchet
    baseline with it. Nothing reports that, which is why it is asserted
    rather than trusted.
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
        "load-bearing: should the triggers ever widen, the ref test stops "
        "another branch's coverage being published as the trunk's, and the "
        "token test keeps a secret-less environment from failing the lane."
    )


def _token_check() -> tuple[int, Document]:
    """Return the publisher's token check step and its position."""
    matches = [
        (index, step)
        for index, step in enumerate(steps(_documents()[PUBLISHER]))
        if step.get("id") == TOKEN_CHECK_STEP_ID
    ]
    assert len(matches) == 1, (
        f"{PUBLISHER} must hold exactly one step with id "
        f"{TOKEN_CHECK_STEP_ID!r}; found {len(matches)}. Without it the "
        "upload's guard reads a missing output and the upload skips forever."
    )
    return matches[0]


def test_the_token_check_reports_availability_and_nothing_else() -> None:
    """Scenario: the check step is changed, guarded or deleted.

    Invariant: a step with id ``codescene-token`` runs exactly the one
    command that writes ``available`` from the secret's presence, with no
    ``if:`` and no ``env``, before the upload. A condition on it, or a longer
    command, could leave ``available`` unwritten or wrong while every other
    assertion here still sees the step.
    """
    index, check = _token_check()
    command = str(check.get("run", "")).strip()
    assert command == TOKEN_CHECK_COMMAND, (
        f"the token check must run exactly {TOKEN_CHECK_COMMAND!r}; it runs "
        f"{command!r}"
    )
    assert "if" not in check, (
        f"the token check must run unconditionally; it carries if: "
        f"{check.get('if')!r}"
    )
    assert not check.get("env"), (
        "the token check must bind nothing; it declares env "
        f"{check.get('env')!r}"
    )
    upload_index = steps(_documents()[PUBLISHER]).index(_sole_upload())
    assert index < upload_index, (
        "the token check must run before the upload that reads its output"
    )


def test_the_publisher_passes_the_token_as_an_input_only() -> None:
    """Scenario: the token is bound in an ``env`` on the publisher job.

    Invariant: the upload passes ``access-token`` directly from the secret,
    and ``CS_ACCESS_TOKEN`` appears in no ``env`` anywhere in the publisher:
    not at workflow scope, not on a job, not on a step. The uploader is a
    composite action that hands its step's ``env`` to nested artefact and
    cache steps, and a job-scoped token is readable by the tests that
    generate coverage. Only the check step and the upload may name it.
    """
    document = _documents()[PUBLISHER]
    upload = _sole_upload()
    access_token = (upload.get("with") or {}).get("access-token")
    assert access_token == ACCESS_TOKEN_INPUT, (
        f"{PUBLISHER}'s upload must pass access-token: "
        f"{ACCESS_TOKEN_INPUT!r}; got {access_token!r}"
    )
    envs = [document.get("env")] + [
        scope.get("env")
        for scope in [*jobs(document).values(), *steps(document)]
    ]
    assert not any(mentions(env, TOKEN) for env in envs if env), (
        f"{PUBLISHER} must bind {TOKEN} in no env; pass it as the upload's "
        "access-token input"
    )
    _, check = _token_check()
    holders = [
        str(step.get("name", step.get("uses", step.get("run"))))
        for step in steps(document)
        if step is not upload and step is not check and mentions(step, TOKEN)
    ]
    assert not holders, (
        f"only the token check and the upload may name {TOKEN}; also found "
        f"in {holders}"
    )


def test_the_uploader_is_pinned_and_passes_no_retired_input() -> None:
    """Scenario: the uploader is unpinned, or passes a retired input.

    Invariant: the upload calls the uploader at a full commit SHA and passes
    no ``installer-checksum``, which the uploader rejects when non-empty from
    shared-actions a5765019 onwards. The specific revision is deliberately
    not asserted, per the repository's pin policy; a branch or tag name is.
    """
    upload = _sole_upload()
    action, _, ref = str(upload.get("uses", "")).partition("@")
    assert action == UPLOADER_ACTION, (
        f"{PUBLISHER}'s upload must call {UPLOADER_ACTION}; it calls "
        f"{action!r}"
    )
    assert re.fullmatch(r"[0-9a-f]{40}", ref), (
        f"{PUBLISHER}'s upload must pin a full commit SHA; got {ref!r}"
    )
    assert "installer-checksum" not in (upload.get("with") or {}), (
        "installer-checksum is rejected when non-empty; pass nothing, the "
        "uploader's manifest is the trust anchor"
    )


def test_the_publisher_serializes_and_is_not_cancelled() -> None:
    """Scenario: two pushes to the trunk upload at once.

    Invariant: the publisher declares a concurrency group keyed on the ref,
    and does not cancel a run in progress. Without a group two uploads race
    and the baseline is set by whichever finishes last. With one, GitHub keeps
    a single pending run per group, so a newer push replaces an older pending
    run and, among triggered runs, the newest baseline wins (a manual re-run of
    an older run republishes that commit's until the next push); cancelling
    instead would abandon a
    running upload and its baseline write.
    """
    concurrency = _documents()[PUBLISHER].get("concurrency")
    assert isinstance(concurrency, dict), (
        f"{PUBLISHER} must declare a concurrency block so two uploads cannot "
        f"race; got {concurrency!r}"
    )
    group = str(concurrency.get("group", ""))
    # Compared whole. A group without the ref would put a branch dispatch in
    # the trunk's queue; one that also named the event would let a dispatch
    # and a push to main run at once and race on the baseline.
    assert group == PUBLISHER_CONCURRENCY_GROUP, (
        f"{PUBLISHER}'s concurrency group must be exactly "
        f"{PUBLISHER_CONCURRENCY_GROUP!r}, keyed on the ref alone; got "
        f"{group!r}"
    )
    assert concurrency.get("cancel-in-progress") is False, (
        f"{PUBLISHER} must not cancel a run in progress: a cancelled upload "
        "leaves the ratchet baseline describing a commit that is no longer "
        f"the tip; got {concurrency.get('cancel-in-progress')!r}"
    )
