"""Contract tests for runner placement, ceilings, and the actionlint registry.

This module holds the assertions. The reviewed decisions and the reasons for
them live in ``runner_placement_policy``, and the machinery that reads the
workflow tree lives in ``runner_placement_reader``. The split is by role
rather than by size: what a person decided, what is derived from the tree,
and what enforces the one against the other.

It replaces ``namespace_runners_test.py``, which pinned four lanes to
``namespace-profile-default`` and the CI lane to ``ubuntu-latest``. Those
assignments are gone: the scheduled, delayed-comment and CodeScene-SHA lanes
are GitHub-hosted, where the placement rule keeps them and public-repository
minutes are free; the push coverage lane and the developer-blocking CI lane
are on Ubicloud.

The retired module's CI assertion carried a reason worth preserving rather
than deleting silently. It required ``ubuntu-latest`` to keep CI "on a runner
compatible with Whitaker's prebuilt cargo-dylint". That constraint is about
the Namespace image specifically, not about non-GitHub-hosted runners:
``whitaker-installer`` needs glibc 2.39, Namespace's shared profile is Ubuntu
22.04 with glibc 2.35, and ``ubicloud-standard-4`` is Ubuntu 24.04.
repovec-appliance runs the same installer 0.2.6 on Ubicloud, green, and this
repository's CI lane has now done so too. The constraint is therefore
satisfied by the new placement, not waived by it.

Four things here are easy to get wrong in ways a green run does not show.

First, the folded scalar. Written as

.. code-block:: yaml

    runs-on: >-
      ${{ github.event.pull_request.head.repo.fork
          && 'ubuntu-latest' || 'ubicloud-standard-4' }}

the more-indented continuation keeps its line break, so the parsed value
carries a newline in the middle of the expression. GitHub evaluates it
regardless and the job runs, so nothing fails and nothing is reported. The
guard is therefore on the *parsed* value, not on the file's text.

Second, the narrowness of the expression check. A contract that matches the
shape of the expression rather than its content passes when
``head.repo.fork`` is replaced by a sibling field that reads just as
plausibly and selects the wrong runner. The guard expression and both arms
are compared by equality against exact strings.

Third, the registry. ``.github/actionlint.yaml`` exists only because
actionlint does not know Ubicloud's labels. A registry checked in one
direction rots: the ``namespace-profile-default`` entry this change removes
would otherwise have sat there indefinitely, authorizing a runner family the
repository no longer uses. The registered labels and the labels in use are
compared as sets.

Fourth, the totality of the tables themselves. Both are keyed by coordinate,
and a table that is merely iterated can never report what is missing from it.
``test_the_reviewed_tables_are_total`` compares each table against the set it
is supposed to cover, so a lane cannot escape review by being left out of one
of them. Without it, a new GitHub-hosted job added to the placement table
with no ceiling entry passes every ceiling test, because the per-coordinate
test iterates the ceiling table and the tree walk reads only ``ubicloud-``
lanes; and a second ``None`` placement value escapes both the label test,
which skips ``None``, and the expression tests, which look only at the one
fork-fallback coordinate.

Mutation proof; each applied alone and reverted:

- indenting the continuation line of ``build-test``'s ``runs-on`` one level
  deeper fails ``test_the_runner_expression_parses_to_one_line``, and
  ``test_the_runner_expression_is_exactly_the_reviewed_one`` with it, since
  the embedded newline also stops the expression matching;
- replacing ``github.event.pull_request.head.repo.fork`` with
  ``github.event.pull_request.head.repo.private``, an otherwise identical
  expression, fails ``test_the_runner_expression_is_exactly_the_reviewed_one``
  alone, which is what makes the check narrow rather than merely sufficient;
- returning ``coverage-main.yml`` to ``namespace-profile-default`` fails
  three tests, not the two intended: ``test_the_job_carries_its_reviewed_label``,
  ``test_the_actionlint_registry_matches_the_labels_in_use`` because the
  retired label is no longer registered, and
  ``test_no_lane_uses_a_foreign_runner_family`` because ``namespace`` is on
  the prohibited list. Three independent readings catch the same drift, which
  is what makes a retirement hard to undo by accident;
- removing the ``advanced`` entry from ``EXPECTED_PLACEMENT`` fails
  ``test_every_job_is_pinned_by_coordinate`` and, because the ceiling table
  still holds that coordinate, ``test_the_reviewed_tables_are_total``;
- removing the ``advanced`` entry from ``EXPECTED_CEILING_MINUTES`` fails
  ``test_the_reviewed_tables_are_total`` alone, and nothing else: that is the
  gap the totality test was added to close;
- giving a second coordinate the ``None`` placement value fails
  ``test_the_reviewed_tables_are_total`` alone, for the same reason;
- deleting ``timeout-minutes`` from ``coverage-upload`` fails
  ``test_the_job_declares_its_reviewed_ceiling`` and
  ``test_every_ubicloud_job_has_a_ceiling``;
- adding a ``timeout-minutes`` to ``delay_and_comment`` fails
  ``test_the_delayed_comment_lane_declares_no_ceiling``, which is the point:
  the absence there is a decision, not an oversight.

A ninth is not a mutation but a real event, recorded because it is better
evidence than a contrived one. Moving both Ubicloud lanes from
``ubicloud-standard-2`` to ``ubicloud-standard-4`` after the smaller shape
killed a test failed ``test_the_job_carries_its_reviewed_label``,
``test_the_runner_expression_is_exactly_the_reviewed_one`` and
``test_the_actionlint_registry_matches_the_labels_in_use`` the moment the
label changed and before the registry was updated, unprompted.
``test_every_ubicloud_job_has_a_ceiling`` stayed green throughout, because it
tests the ``ubicloud-`` prefix rather than a literal label; written against a
literal it would have silently stopped noticing an unbounded lane at exactly
that moment, which is a dead-guard defect repovec-appliance found in its own
placement contract.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import pytest
from runner_placement_policy import (
    CEILING_FREE_JOB,
    DELEGATED_JOBS,
    EXPECTED_CEILING_MINUTES,
    EXPECTED_PLACEMENT,
    FOREIGN_RUNNER_FRAGMENTS,
    FORK_FALLBACK_JOB,
    FORK_GUARD,
    GITHUB_HOSTED_LABEL,
    GITHUB_HOSTED_LABELS,
    UBICLOUD_LABEL,
    UBICLOUD_LABEL_PREFIX,
)
from runner_placement_reader import (
    RUNNER_EXPRESSION,
    WORKFLOW_DIR,
    case_id,
    job_labels,
    jobs,
    labels_in_use,
    registered_labels,
    runner_value,
)

pytestmark = pytest.mark.skipif(
    not WORKFLOW_DIR.is_dir(),
    reason="workflow files not present in this working copy",
)


def test_every_job_is_pinned_by_coordinate() -> None:
    """Scenario: a lane is added and nobody decides where it runs.

    Invariant: the pinned coordinates and the tree's jobs are the same set.
    Iterating only the pinned coordinates would never examine a job nobody
    listed, so a new lane could carry any label at all and satisfy every
    other assertion here.
    """
    observed = set(jobs())
    expected = set(EXPECTED_PLACEMENT) | DELEGATED_JOBS
    assert observed == expected, (
        "runner placement is pinned per job; unpinned jobs "
        f"{sorted(observed - expected)} and stale pins "
        f"{sorted(expected - observed)} must be reconciled"
    )


def test_the_reviewed_tables_are_total() -> None:
    """Scenario: a lane is added to one reviewed table but not the others.

    Invariant: each table covers exactly the coordinates it is supposed to.
    Every other test here iterates a table, and an iterated table cannot
    report what was never put in it. Two gaps close here.

    A placed job absent from the ceiling table passes both ceiling tests: the
    per-coordinate one iterates that table, and the tree walk reads only
    ``ubicloud-`` lanes, so a new GitHub-hosted lane would inherit the
    six-hour default unremarked.

    A second coordinate carrying the ``None`` placement value escapes the
    label test, which skips ``None``, and the expression tests, which read
    only ``FORK_FALLBACK_JOB``. Its runner would then be checked by nothing
    at all.
    """
    expression_jobs = {
        coordinate
        for coordinate, label in EXPECTED_PLACEMENT.items()
        if label is None
    }
    assert expression_jobs == {FORK_FALLBACK_JOB}, (
        "only the fork-fallback lane may be checked by expression rather than "
        f"by label; {sorted(expression_jobs - {FORK_FALLBACK_JOB})} would have "
        "no runner assertion at all"
    )
    needing_ceilings = set(EXPECTED_PLACEMENT) - {CEILING_FREE_JOB}
    assert set(EXPECTED_CEILING_MINUTES) == needing_ceilings, (
        "every placed job except the ceiling-free lane needs a reviewed "
        f"ceiling; missing {sorted(needing_ceilings - set(EXPECTED_CEILING_MINUTES))} "
        f"and stale {sorted(set(EXPECTED_CEILING_MINUTES) - needing_ceilings)}"
    )


@pytest.mark.parametrize(
    ("coordinate", "label"),
    sorted(
        (key, value) for key, value in EXPECTED_PLACEMENT.items() if value is not None
    ),
    ids=case_id,
)
def test_the_job_carries_its_reviewed_label(
    coordinate: tuple[str, str], label: str
) -> None:
    """Scenario: a lane drifts back to Namespace, or onto a larger shape.

    Invariant: the job declares exactly the reviewed label and nothing
    beside it. Equality, not containment: a check that the declaration
    merely contains ``ubicloud-`` accepts ``ubicloud-standard-16`` as
    readily as the reviewed shape, and a list reading lets a second label
    ride along. Size is the thing being reviewed here, so the reading that
    ignores it is the wrong one.
    """
    declared = runner_value(jobs()[coordinate])
    assert declared == label, (
        f"{coordinate[0]}:{coordinate[1]} must run on exactly {label!r}, "
        f"got {declared!r}"
    )


def test_the_runner_expression_parses_to_one_line() -> None:
    """Scenario: the folded scalar's continuation is indented one level deeper.

    Invariant: the parsed value is a single line. A more-indented
    continuation keeps its line break, putting a newline inside the
    expression. GitHub evaluates the broken value and the job runs, so a
    green run is not evidence; only the parsed value shows it.
    """
    value = runner_value(jobs()[FORK_FALLBACK_JOB])
    assert isinstance(value, str), (
        f"{FORK_FALLBACK_JOB[0]}:{FORK_FALLBACK_JOB[1]} should declare runs-on "
        f"as one scalar, got {value!r}"
    )
    assert "\n" not in value, (
        f"{FORK_FALLBACK_JOB[0]}:{FORK_FALLBACK_JOB[1]} has a line break inside "
        f"its runs-on expression ({value!r}); the folded scalar's continuation "
        "line must sit at the same indent as the line above it"
    )


def test_the_runner_expression_is_exactly_the_reviewed_one() -> None:
    """Scenario: the fork guard is swapped for a plausible sibling field.

    Invariant: the guard and both arms equal the reviewed strings. Matching
    the expression's shape rather than its content passes when
    ``head.repo.fork`` becomes, say, ``head.repo.private``: an expression of
    exactly the same form that sends every fork pull request to a runner it
    cannot obtain, and every other pull request to the wrong place.
    """
    value = str(runner_value(jobs()[FORK_FALLBACK_JOB]))
    match = RUNNER_EXPRESSION.match(value)
    assert match is not None, (
        f"{FORK_FALLBACK_JOB[0]}:{FORK_FALLBACK_JOB[1]} should declare the "
        f"reviewed fork fallback expression; got {value!r}"
    )
    assert match["guard"] == FORK_GUARD, (
        f"the fallback is keyed on {match['guard']!r}; only {FORK_GUARD!r} "
        "identifies a fork"
    )
    assert match["fork_arm"] == GITHUB_HOSTED_LABEL, (
        f"forks are sent to {match['fork_arm']!r}; a fork cannot obtain an "
        f"Ubicloud runner, so the fork arm must be {GITHUB_HOSTED_LABEL!r}"
    )
    assert match["default_arm"] == UBICLOUD_LABEL, (
        f"non-fork events run on {match['default_arm']!r}, not the reviewed "
        f"{UBICLOUD_LABEL!r}"
    )


@pytest.mark.parametrize(
    ("coordinate", "minutes"),
    sorted(EXPECTED_CEILING_MINUTES.items()),
    ids=case_id,
)
def test_the_job_declares_its_reviewed_ceiling(
    coordinate: tuple[str, str], minutes: int
) -> None:
    """Scenario: a ceiling drifts to a number nobody reviewed.

    Invariant: each job carries the exact ceiling recorded for it. A
    per-minute runner bills until something stops it, so the six-hour
    default is one failure mode; a ceiling near the measured work is the
    other, because it cancels the run at the moment the overrun becomes
    interesting and discards the log that would explain it.
    """
    declared = jobs()[coordinate].get("timeout-minutes")
    assert declared == minutes, (
        f"{coordinate[0]}:{coordinate[1]} must set timeout-minutes: "
        f"{minutes}, got {declared!r}"
    )


def test_every_ubicloud_job_has_a_ceiling() -> None:
    """Scenario: a new Ubicloud lane inherits GitHub's six-hour default.

    Invariant: every job that can select any Ubicloud label declares a
    ceiling. The pinned table above is keyed by coordinate; this reads the
    tree instead, so a lane cannot escape by being absent from a list, and it
    tests the label prefix so a right-sized shape stays covered.
    """
    unbounded = [
        coordinate
        for coordinate, definition in jobs().items()
        if any(
            label.startswith(UBICLOUD_LABEL_PREFIX)
            for label in job_labels(definition)
        )
        and definition.get("timeout-minutes") is None
    ]
    assert not unbounded, (
        f"jobs on an Ubicloud runner without a ceiling: {sorted(unbounded)}"
    )


def test_the_delayed_comment_lane_declares_no_ceiling() -> None:
    """Scenario: someone adds the "missing" ceiling to the delayed lane.

    Invariant: that lane declares none. Its entire duration is a ``sleep`` of
    the caller's ``delay_minutes`` input, so a fixed ceiling cancels a
    legitimate longer delay: it would fire exactly when the delay became
    interesting. The lane is GitHub-hosted, so the six-hour default costs
    nothing. The absence is asserted rather than merely left, so it reads as
    a decision and not as the gap the previous test looks for.
    """
    definition = jobs()[CEILING_FREE_JOB]
    assert definition.get("timeout-minutes") is None, (
        f"{CEILING_FREE_JOB[0]}:{CEILING_FREE_JOB[1]} must not declare a "
        "ceiling: it sleeps for the caller's delay_minutes input, and a "
        "ceiling would cancel a legitimate longer delay"
    )
    assert runner_value(definition) == GITHUB_HOSTED_LABEL, (
        "the ceiling-free exception is only safe on a runner that does not "
        f"bill per minute, so this lane must stay on {GITHUB_HOSTED_LABEL!r}"
    )


@pytest.mark.parametrize("coordinate", sorted(DELEGATED_JOBS), ids=case_id)
def test_a_reusable_caller_declares_no_runner(coordinate: tuple[str, str]) -> None:
    """Scenario: a scheduled or administrative lane acquires a runner label.

    Invariant: these coordinates call a reusable workflow and declare no
    runner, so the callee places them. A ``runs-on`` appearing here is the
    repository taking a placement decision it has not reviewed.
    """
    definition = jobs()[coordinate]
    assert "uses" in definition, (
        f"{coordinate[0]}:{coordinate[1]} is recorded as a reusable-workflow "
        "caller but declares no uses key"
    )
    assert runner_value(definition) is None, (
        f"{coordinate[0]}:{coordinate[1]} calls a reusable workflow, so its "
        f"runner is the callee's; found runs-on {runner_value(definition)!r}"
    )


def test_the_actionlint_registry_matches_the_labels_in_use() -> None:
    """Scenario: the registry and the workflows drift apart.

    Invariant: the registered labels are exactly the non-GitHub-hosted
    labels in use, compared in both directions. An unregistered label makes
    actionlint report a lane nobody broke; a registered label no lane uses
    is worse, because it silently permits a runner family nobody reviewed
    for whatever lane adopts it next. This is what retires
    ``namespace-profile-default`` rather than leaving it behind.
    """
    needing_registration = labels_in_use() - set(GITHUB_HOSTED_LABELS)
    registered = registered_labels()
    assert registered == needing_registration, (
        "the actionlint runner registry must hold exactly the labels in use; "
        f"unregistered {sorted(needing_registration - registered)} and stale "
        f"{sorted(registered - needing_registration)}"
    )


def test_no_lane_uses_a_foreign_runner_family() -> None:
    """Scenario: a lane drifts back to Namespace, or onto Windows or macOS.

    Invariant: this repository has no such lane. The reading is by
    substring, which is the safe direction for a prohibition: a renamed or
    neutered label still leaves its family's text behind.
    """
    offenders = sorted(
        label
        for label in labels_in_use()
        if any(fragment in label.lower() for fragment in FOREIGN_RUNNER_FRAGMENTS)
    )
    assert not offenders, (
        "this repository has no Namespace, Windows, macOS or ad-hoc "
        f"self-hosted lane; found {offenders}"
    )
