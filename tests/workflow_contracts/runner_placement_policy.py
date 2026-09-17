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

#: The reviewed Ubicloud shape. Four vCPU rather than two, and that is a
#: measurement rather than a preference for the larger machine.
#: ``tests/compile_error.rs`` is a trybuild case that spawns its own cargo
#: build of the whole dependency tree while the other 987 tests compete for
#: the same cores. On four vCPU it takes 64.5 s of the suite's 65.4 s; on
#: ``ubicloud-standard-2`` it was still running when nextest killed it at its
#: default 180 s, and the coverage step failed at exit 100 with 987 of 988
#: passed. Both lanes that run the suite carry this shape, so one constant
#: serves both; placement is still pinned per coordinate below, so they can
#: diverge later without this constant becoming a lie.
UBICLOUD_LABEL: typ.Final = "ubicloud-standard-4"
GITHUB_HOSTED_LABEL: typ.Final = "ubuntu-latest"

#: The one guard the fork fallback may key on. Compared by equality, because a
#: sibling field such as ``head.repo.private`` yields an expression of exactly
#: the same shape that selects the wrong runner on every fork pull request.
FORK_GUARD: typ.Final = "github.event.pull_request.head.repo.fork"

#: The lane that carries the fork fallback: the only one serving pull
#: requests, and so the only one that can meet a fork.
FORK_FALLBACK_JOB: typ.Final = ("ci.yml", "build-test")

#: Every job this repository places, with its reviewed label. A literal label
#: is written as itself; the fork-fallback lane is checked by its expression
#: instead and appears here as ``None`` so the coordinate set stays total.
#: ``test_the_reviewed_tables_are_total`` holds it to exactly one ``None``.
EXPECTED_PLACEMENT: typ.Final = {
    ("ci.yml", "build-test"): None,
    ("coverage-main.yml", "coverage-upload"): UBICLOUD_LABEL,
    ("advanced-tests.yml", "advanced"): GITHUB_HOSTED_LABEL,
    ("delayed-pr-comment.yml", "delay_and_comment"): GITHUB_HOSTED_LABEL,
    ("get-codescene-sha.yml", "refresh-sha"): GITHUB_HOSTED_LABEL,
}

#: Ceilings, pinned by value rather than bounded, because a ceiling can drift
#: to a number nobody chose while every inequality still holds.
#:
#: ``build-test`` at 30 and ``coverage-upload`` at 20 are measured, from 556 s
#: and 240 s of work on four-vCPU runners. ``advanced`` at 60 and
#: ``refresh-sha`` at 10 are judgements and are recorded as such in the
#: developers' guide: ``advanced`` has failed every scheduled run since at
#: least 2026-09-09 and its last green run was 2025-10-04 at 32 s, which a
#: Loom suite has long since outgrown, and ``refresh-sha`` has never run at
#: all.
EXPECTED_CEILING_MINUTES: typ.Final = {
    ("ci.yml", "build-test"): 30,
    ("coverage-main.yml", "coverage-upload"): 20,
    ("advanced-tests.yml", "advanced"): 60,
    ("get-codescene-sha.yml", "refresh-sha"): 10,
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
