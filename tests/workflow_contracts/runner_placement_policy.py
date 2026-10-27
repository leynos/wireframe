"""The reviewed runner-placement decisions, and why each one is what it is.

Everything here was decided by a person and is enforced by
``ci_runner_placement_test.py``; everything in ``runner_placement_reader`` is
derived from the workflow tree. That is the line the three modules are split
on, rather than on size: a reviewer who wants to know what was decided reads
this file alone, and a test that disagrees with it names the constant it
disagrees with.

The tables are keyed by ``(workflow file, job id)`` coordinate. Coordinates
rather than names, because two workflows may both define a job called
``build`` and a contract that keyed on the bare name would silently check one
of them twice and the other never.
"""

from __future__ import annotations

import typing as typ

UBICLOUD_LABEL_PREFIX: typ.Final = "ubicloud-"

#: The Ubicloud shape of ``coverage-upload``. Four vCPU, because that lane has
#: only been sampled on four: its 240 s run was measured on Namespace's
#: four-vCPU profile. The coverage action's 180 s deadline once killed the
#: aggregate trybuild test on two vCPU; the repository now gives that one test
#: an eight-minute nextest budget, but the smaller shape has not been sampled
#: with this policy.
UBICLOUD_LABEL: typ.Final = "ubicloud-standard-4"

#: The shape of ``build-test``'s non-fork arm. Two vCPU, measured on
#: 2026-10-17 with a warm cache: 8m50s at 1.28 busy cores on four vCPU
#: against 12m30s at 0.88 busy cores on two, with the trybuild case passing.
#: A larger shape pays for itself only if the lane keeps more than half of
#: it busy, and this lane does not. Kept apart from ``UBICLOUD_LABEL`` so the
#: two lanes can differ without either constant becoming a lie.
CI_UBICLOUD_LABEL: typ.Final = "ubicloud-standard-2"
GITHUB_HOSTED_LABEL: typ.Final = "ubuntu-latest"

#: The one guard the fork fallback may key on. Compared by equality, because a
#: sibling field such as ``head.repo.private`` yields an expression of exactly
#: the same shape that selects the wrong runner on every fork pull request.
FORK_GUARD: typ.Final = "github.event.pull_request.head.repo.fork"

#: The pull request's author, not ``github.actor``: the actor is whoever
#: triggered the run, so a person pushing to a Dependabot branch would flip the
#: runner. Hosted minutes cost nothing on a public repository, so Dependabot's
#: pull requests take the fork arm too.
DEPENDABOT_GUARD: typ.Final = (
    "github.event.pull_request.user.login == 'dependabot[bot]'"
)

#: The exact guard the fallback keys on: a fork or a Dependabot pull request.
HOSTED_GUARD: typ.Final = f"({FORK_GUARD} || {DEPENDABOT_GUARD})"

#: Both pull-request jobs require the same fallback for forks and Dependabot.
FORK_FALLBACK_JOBS: typ.Final = frozenset({
    ("ci.yml", "build-test"),
    ("ci.yml", "ui-test"),
})

#: Every job this repository places, with its reviewed label. A literal label
#: is written as itself; the fork-fallback lane is checked by its expression
#: instead and appears here as ``None`` so the coordinate set stays total.
#: ``test_the_reviewed_tables_are_total`` holds it to the reviewed fallback jobs.
EXPECTED_PLACEMENT: typ.Final = {
    ("ci.yml", "build-test"): None,
    ("ci.yml", "ui-test"): None,
    ("coverage-main.yml", "coverage-upload"): UBICLOUD_LABEL,
    ("advanced-tests.yml", "advanced"): GITHUB_HOSTED_LABEL,
    ("delayed-pr-comment.yml", "delay_and_comment"): GITHUB_HOSTED_LABEL,
}

#: Ceilings, pinned by value rather than bounded, because a ceiling can drift
#: to a number nobody chose while every inequality still holds.
#:
#: ``build-test`` at 30 and ``coverage-upload`` at 20 are measured, from 556 s
#: of work on four-vCPU runners (750 s on two vCPU, still inside 30 minutes)
#: and 240 s on four-vCPU runners. ``advanced`` at 60 and
#: ``refresh-sha`` at 10 are judgements and are recorded as such in the
#: developers' guide: ``advanced`` has failed every scheduled run since at
#: least 2026-09-09 and its last green run was 2025-10-04 at 32 s, which a
#: Loom suite has long since outgrown, and ``refresh-sha`` has never run at
#: all.
EXPECTED_CEILING_MINUTES: typ.Final = {
    ("ci.yml", "build-test"): 30,
    ("ci.yml", "ui-test"): 30,
    ("coverage-main.yml", "coverage-upload"): 20,
    ("advanced-tests.yml", "advanced"): 60,
}

#: The one lane that must NOT declare a ceiling. Its whole duration is a
#: ``sleep`` of the caller's ``delay_minutes`` input, so a fixed ceiling
#: cancels a legitimate longer delay: it would fire exactly when the delay
#: became interesting, which inverts the failure a ceiling exists to prevent.
#: It is GitHub-hosted, so the six-hour default costs nothing.
CEILING_FREE_JOB: typ.Final = ("delayed-pr-comment.yml", "delay_and_comment")

#: Jobs this repository does not place: thin callers of reusable workflows,
#: whose runner is the callee's to choose.
DELEGATED_JOBS: typ.Final = {
    ("mutation-testing.yml", "mutation"),
    ("dependabot-automerge.yml", "automerge"),
}

#: GitHub-hosted labels this repository may use without registering them with
#: actionlint, which knows them already.
GITHUB_HOSTED_LABELS: typ.Final = frozenset({GITHUB_HOSTED_LABEL})

#: Prohibited runner families. A prohibition reads by substring on purpose: a
#: renamed or neutered label still leaves its family's text behind.
#: ``namespace`` is listed because this repository has just left it, and a
#: lane drifting back would otherwise be invisible.
FOREIGN_RUNNER_FRAGMENTS: typ.Final = ("windows", "macos", "self-hosted", "namespace")
