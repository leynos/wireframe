"""CV-005: the push-to-main publisher is the one lane that talks to CodeScene.

``ci_codescene_placement_test`` forbids CodeScene everywhere a pull request
can reach; this module says what the one permitted lane must look like, so
the rule cannot be satisfied by deleting the upload, and so the upload cannot
publish a feature branch's coverage as the trunk's.
"""

from __future__ import annotations

from codescene_placement_policy import (
    ACCESS_TOKEN_INPUT,
    CODESCENE_ACTION_MARKER,
    EXPECTED_UPLOAD_CONDITION,
    PUBLISHER,
    PUBLISHER_TRIGGERS,
    TOKEN,
    TOKEN_BINDING,
    TRUNK_BRANCH,
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
    assert (upload.get("env") or {}).get(TOKEN) == TOKEN_BINDING, (
        f"{PUBLISHER}'s upload step must bind {TOKEN} in its own env as "
        f"{TOKEN_BINDING!r}"
    )
    access_token = (upload.get("with") or {}).get("access-token")
    assert access_token == ACCESS_TOKEN_INPUT, (
        f"{PUBLISHER}'s upload must pass access-token: "
        f"{ACCESS_TOKEN_INPUT!r}; got {access_token!r}"
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
