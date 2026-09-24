"""Reviewed decisions for the CodeScene placement contract (CV-005).

The values here are what a person decided; ``codescene_placement_reader``
derives facts from the tree, and ``ci_codescene_placement_test`` and
``codescene_publisher_test`` hold the one against the other. The split is by
role, as for runner placement, and keeps each module under the 400-line
limit.
"""

from __future__ import annotations

import typing as typ

#: The one workflow allowed to reach CodeScene, and the trigger that makes it
#: safe: a push lane cannot block a merge.
PUBLISHER: typ.Final = "coverage-main.yml"

#: The publisher's complete trigger set.
PUBLISHER_TRIGGERS: typ.Final = frozenset({"push"})

#: The token's name. Present anywhere in a pull-request workflow is a failure.
TOKEN: typ.Final = "CS_ACCESS_TOKEN"

#: The step that reports whether the token is configured, and its sole
#: command. The expression is evaluated to ``true`` or ``false`` before the
#: shell runs, so the token itself is in no step's ``env`` and the shell sees
#: no conditional. A guard on ``env.CS_ACCESS_TOKEN != ''`` instead passes
#: with its binding deleted, and the upload then skips forever.
TOKEN_CHECK_STEP_ID: typ.Final = "codescene-token"
TOKEN_CHECK_COMMAND: typ.Final = (
    'echo "available=${{ secrets.CS_ACCESS_TOKEN != '
    '\'\' }}" >> "$GITHUB_OUTPUT"'
)

#: The upload action's token input, passed directly from the secret. The
#: uploader is a composite action that hands its step's ``env`` to nested
#: artefact and cache steps, so the token goes in as an input, never an
#: ``env`` value.
ACCESS_TOKEN_INPUT: typ.Final = "${{ secrets.CS_ACCESS_TOKEN }}"

#: The uploader, matched on its path. Its revision is not named here: the
#: repository's pin policy (developers' guide, "Workflow pins and
#: Dependabot") has contracts assert a full commit SHA, never a specific one,
#: so a Dependabot bump does not fail the suite. The floor, shared-actions
#: a5765019, whose manifest pins the CodeScene CLI, is recorded in a comment
#: beside the call instead.
UPLOADER_ACTION: typ.Final = (
    "leynos/shared-actions/.github/actions/upload-codescene-coverage"
)

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

#: The publisher's concurrency group, compared whole. Keyed on the ref alone:
#: a group that also varied by event would let a dispatch and a push to main
#: run side by side and race on the ratchet baseline, where one group for the
#: ref keeps a single pending run, so among triggered runs the newest wins.
PUBLISHER_CONCURRENCY_GROUP: typ.Final = "coverage-main-${{ github.ref }}"

#: The ref the publisher may upload for. Compared in full rather than by
#: suffix: a branch named ``not-main`` ends in ``main``.
TRUNK_REF: typ.Final = "refs/heads/main"

#: The branch the publisher runs on, as ``push.branches`` must list it.
TRUNK_BRANCH: typ.Final = "main"

#: The upload step's condition, compared whole. A substring test would accept
#: a condition holding both halves inside ``(... || true)``, which is true
#: everywhere. Equality also refuses any extra conjunct, so an ``||`` hidden
#: behind one (``<guard> && github.actor != 'x' || github.event_name ==
#: 'workflow_dispatch'``) fails here with no separate ``||`` rule.
EXPECTED_UPLOAD_CONDITION: typ.Final = (
    f"steps.{TOKEN_CHECK_STEP_ID}.outputs.available == 'true' "
    f"&& github.ref == '{TRUNK_REF}'"
)
