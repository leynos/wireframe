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

#: The upload step's two bindings of the token, pinned by value: the step's
#: ``env`` reads the secret, and the action's input reads that ``env``. A
#: misspelt secret name reads as empty and the upload silently skips.
TOKEN_BINDING: typ.Final = "${{ secrets.CS_ACCESS_TOKEN }}"
ACCESS_TOKEN_INPUT: typ.Final = "${{ env.CS_ACCESS_TOKEN }}"

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
