"""Contract tests for pinning third-party workflow actions.

CodeQL's default-setup ``actions/unpinned-tag`` query reported alert 13
because ``delayed-pr-comment.yml`` called
``thollander/actions-comment-pull-request`` at the mutable tag ``v3``. A tag
can be repointed after review, and that lane runs the action with
``pull-requests: write``, so the reviewed code and the executed code could
differ.

The contract walks every workflow and refuses a mutable ref on any owner the
query does not trust. It mirrors the rule rather than listing today's pins, so
a new third-party action fails review even though CodeQL would only report it
after the merge.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import pytest

from action_pinning import (
    action_references,
    pinning_problem,
    repository_unpinned_references,
    unpinned_references,
)
from workflow_loader import load_workflow, repository_workflows

#: The reviewed commit behind ``thollander/actions-comment-pull-request@v3``.
PINNED_SHA = "24bffb9b452ba05a4f3f77933840a6a841d1b32b"
COMMENT_ACTION = "thollander/actions-comment-pull-request"


def test_every_third_party_action_reference_is_pinned() -> None:
    """Scenario: a workflow gains a mutable ref, or reverts a pin.

    Invariant: every ``uses:`` reference in every workflow is a trusted owner,
    a local or same-repository reference, or a full commit hash.
    """
    offenders = repository_unpinned_references()
    locations = [reference.describe() for reference in offenders]
    assert not offenders, (
        "these workflow references are not pinned to a full commit hash: "
        f"{locations}"
    )


def test_the_delayed_comment_lane_pins_its_comment_action() -> None:
    """The lane from alert 13 names a commit, not the mutable ``v3`` tag."""
    references = [
        reference
        for reference in action_references(repository_workflows())
        if reference.uses.startswith(f"{COMMENT_ACTION}@")
    ]
    assert len(references) == 1, (
        f"expected one {COMMENT_ACTION} reference; found {len(references)}"
    )
    problem = pinning_problem(references[0].uses)
    assert problem is None, f"{references[0].describe()}: {problem}"


def test_references_are_read_from_job_and_step_positions() -> None:
    """Scenario: a reader that sees only steps misses a reusable workflow.

    Invariant: a job-level ``uses:`` and both step spellings are reported,
    and an unnamed step is labelled by its one-based position.
    """
    document = load_workflow(
        f"""
jobs:
  call:
    uses: leynos/shared-actions/.github/workflows/build.yml@{PINNED_SHA}
  build:
    steps:
      - name: Named step
        uses: owner/action@{PINNED_SHA}
      - uses: owner/second@{PINNED_SHA}
"""
    )
    references = action_references({"synthetic.yml": document})
    assert [
        (reference.job, reference.step, reference.uses)
        for reference in references
    ] == [
        (
            "call",
            "(job)",
            f"leynos/shared-actions/.github/workflows/build.yml@{PINNED_SHA}",
        ),
        ("build", "Named step", f"owner/action@{PINNED_SHA}"),
        ("build", "step 2", f"owner/second@{PINNED_SHA}"),
    ]


def test_unpinned_references_reports_only_offending_references() -> None:
    """The checker discriminates instead of flagging every reference."""
    document = load_workflow(
        f"""
jobs:
  build:
    steps:
      - uses: owner/pinned@{PINNED_SHA}
      - uses: owner/mutable@v1
"""
    )
    offenders = unpinned_references({"synthetic.yml": document})
    assert [reference.uses for reference in offenders] == ["owner/mutable@v1"]


@pytest.mark.parametrize(
    ("uses", "is_accepted"),
    [
        (f"{COMMENT_ACTION}@{PINNED_SHA}", True),
        (f"{COMMENT_ACTION}@{PINNED_SHA.upper()}", True),
        (f"{COMMENT_ACTION}@{'a' * 64}", True),
        (f"{COMMENT_ACTION}@v3", False),
        (f"{COMMENT_ACTION}@main", False),
        (COMMENT_ACTION, False),
        (f"{COMMENT_ACTION}@{'a' * 39}", False),
        ("actions/checkout@v7", True),
        ("github/codeql-action/analyze@main", True),
        ("advanced-security/dependency-review@v1", True),
        ("./local-action", True),
        ("$/path/to/action", True),
        ("docker://alpine:3.19", False),
        (f"docker://alpine@sha256:{'a' * 64}", True),
        (f"docker://alpine@sha256:{'a' * 63}", False),
    ],
    ids=[
        "commit-sha1",
        "commit-uppercase",
        "commit-sha256",
        "tag",
        "branch",
        "no-ref",
        "short-hash",
        "trusted-actions-owner",
        "trusted-github-owner",
        "trusted-advanced-security-owner",
        "local-action",
        "same-repository-action",
        "container-tag",
        "container-digest",
        "short-container-digest",
    ],
)
def test_pinning_policy_matches_the_codeql_rule(
    uses: str, is_accepted: bool
) -> None:
    """Drive the policy both ways across every shape the rule distinguishes."""
    problem = pinning_problem(uses)
    assert (problem is None) is is_accepted, problem