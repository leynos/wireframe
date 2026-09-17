"""Contract tests for runner placement, ceilings, and the actionlint registry.

This module replaces ``namespace_runners_test.py``, which pinned four lanes to
``namespace-profile-default`` and the CI lane to ``ubuntu-latest``. Those
assignments are gone: the scheduled, delayed-comment and CodeScene-SHA lanes
are GitHub-hosted, where the placement rule keeps them and public-repository
minutes are free; the push coverage lane and the developer-blocking CI lane
are on Ubicloud.

The retired module's CI assertion carried a reason worth preserving rather
than deleting silently. It required ``ubuntu-latest`` to keep CI "on a runner
compatible with Whitaker's prebuilt cargo-dylint". That constraint is about
the Namespace image specifically, not about non-GitHub-hosted runners:
repovec-appliance runs the same ``whitaker-installer`` 0.2.6 on
``ubicloud-standard-4``, and previously on ``ubicloud-standard-2``, green. The
constraint is therefore satisfied by the new placement, not waived by it.

Three things here are easy to get wrong in ways a green run does not show.

First, the folded scalar. Written as

.. code-block:: yaml

    runs-on: >-
      ${{ github.event.pull_request.head.repo.fork
          && 'ubuntu-latest' || 'ubicloud-standard-2' }}

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

Mutation proof, recorded 2026-09-16; each applied alone and reverted:

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
  ``test_every_job_is_pinned_by_coordinate`` alone;
- deleting ``timeout-minutes`` from ``coverage-upload`` fails
  ``test_the_job_declares_its_reviewed_ceiling`` and
  ``test_every_ubicloud_job_has_a_ceiling``;
- adding a ``timeout-minutes`` to ``delay_and_comment`` fails
  ``test_the_delayed_comment_lane_declares_no_ceiling``, which is the point:
  the absence there is a decision, not an oversight.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import pytest
import yaml

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_DIR: typ.Final = REPO_ROOT / ".github" / "workflows"
ACTIONLINT_CONFIG: typ.Final = REPO_ROOT / ".github" / "actionlint.yaml"

#: Both spellings GitHub accepts for a workflow file's extension. Reading one
#: of them would let a lane in the other sit outside every check here.
WORKFLOW_FILE_PATTERNS: typ.Final = ("*.yml", "*.yaml")

UBICLOUD_LABEL_PREFIX: typ.Final = "ubicloud-"
UBICLOUD_LABEL: typ.Final = "ubicloud-standard-2"
GITHUB_HOSTED_LABEL: typ.Final = "ubuntu-latest"

#: The one guard the fork fallback may key on. Compared by equality, because a
#: sibling field such as ``head.repo.private`` yields an expression of exactly
#: the same shape that selects the wrong runner on every fork pull request.
FORK_GUARD: typ.Final = "github.event.pull_request.head.repo.fork"

#: The lane that carries the fork fallback: the only one serving pull requests.
FORK_FALLBACK_JOB: typ.Final = ("ci.yml", "build-test")

#: Every job this repository places, with its reviewed label. A literal label
#: is written as itself; the fork-fallback lane is checked by
#: ``test_the_runner_expression_is_exactly_the_reviewed_one`` instead and
#: appears here as ``None`` so the coordinate set stays total.
EXPECTED_PLACEMENT: typ.Final = {
    ("ci.yml", "build-test"): None,
    ("coverage-main.yml", "coverage-upload"): UBICLOUD_LABEL,
    ("advanced-tests.yml", "advanced"): GITHUB_HOSTED_LABEL,
    ("delayed-pr-comment.yml", "delay_and_comment"): GITHUB_HOSTED_LABEL,
    ("get-codescene-sha.yml", "refresh-sha"): GITHUB_HOSTED_LABEL,
}

#: Ceilings, pinned by value rather than bounded, because a ceiling can drift
#: to a number nobody chose while every inequality still holds. The
#: measurements behind each are in the pull request and the commit message.
EXPECTED_CEILING_MINUTES: typ.Final = {
    ("ci.yml", "build-test"): 30,
    ("coverage-main.yml", "coverage-upload"): 20,
    ("advanced-tests.yml", "advanced"): 60,
    ("get-codescene-sha.yml", "refresh-sha"): 10,
}

#: The one lane that must NOT declare a ceiling. Its whole duration is a
#: ``sleep`` of the caller's ``delay_minutes`` input, so a fixed ceiling
#: cancels a legitimate longer delay. It is GitHub-hosted, so the six-hour
#: default costs nothing.
CEILING_FREE_JOB: typ.Final = ("delayed-pr-comment.yml", "delay_and_comment")

#: Jobs this repository does not place: thin callers of reusable workflows,
#: whose runner is the callee's to choose.
DELEGATED_JOBS: typ.Final = {
    ("mutation-testing.yml", "mutation"),
    ("dependabot-automerge.yml", "automerge"),
}

#: GitHub-hosted labels this repository may use without registering them.
GITHUB_HOSTED_LABELS: typ.Final = frozenset({GITHUB_HOSTED_LABEL})

#: Prohibited runner families. A prohibition reads by substring on purpose: a
#: renamed or neutered label still leaves its family's text behind.
#: ``namespace`` is listed because this repository has just left it, and a
#: lane drifting back would otherwise be invisible.
FOREIGN_RUNNER_FRAGMENTS: typ.Final = ("windows", "macos", "self-hosted", "namespace")

#: One expression, anchored end to end. ``[^'\n]`` in the arms and ``\S`` in
#: the guard keep a value carrying an embedded line break from matching here
#: as well, so the line-break failure is reported by its own test.
RUNNER_EXPRESSION: typ.Final = re.compile(
    r"^\$\{\{ (?P<guard>\S+)"
    r" && '(?P<fork_arm>[^'\n]*)'"
    r" \|\| '(?P<default_arm>[^'\n]*)' \}\}$"
)

#: Every quoted literal in an expression, used to read the labels a lane can
#: actually select.
EXPRESSION_LITERAL: typ.Final = re.compile(r"'([^'\n]*)'")

pytestmark = pytest.mark.skipif(
    not WORKFLOW_DIR.is_dir(),
    reason="workflow files not present in this working copy",
)


def _case_id(value: object) -> str:
    """Render one parametrized case identifier."""
    if isinstance(value, tuple):
        return "-".join(str(item) for item in value)
    return str(value)


def _workflow_paths() -> list[Path]:
    """Return every workflow document's path, in a stable order.

    GitHub accepts both spellings of the extension and runs a workflow
    written either way. Reading only one of them would leave a lane in
    ``.github/workflows/*.yaml`` outside every assertion below.
    """
    return sorted(
        path
        for pattern in WORKFLOW_FILE_PATTERNS
        for path in WORKFLOW_DIR.glob(pattern)
    )


def _workflows() -> dict[str, dict[str, object]]:
    """Parse every workflow document, keyed by file name."""
    documents = {
        path.name: yaml.safe_load(path.read_text(encoding="utf-8"))
        for path in _workflow_paths()
    }
    assert documents, "the repository should define at least one workflow"
    return documents


def _jobs() -> dict[tuple[str, str], dict[str, object]]:
    """Return every job in the repository, keyed by ``(workflow, job id)``."""
    keyed: dict[tuple[str, str], dict[str, object]] = {}
    for name, document in _workflows().items():
        for job_id, definition in ((document or {}).get("jobs") or {}).items():
            keyed[(name, job_id)] = definition
    return keyed


def _runner_value(definition: dict[str, object]) -> object | None:
    """Return a job's ``runs-on`` value, if it declares one."""
    return definition.get("runs-on")


def _runner_declarations(definition: dict[str, object]) -> list[object]:
    """Return a job's ``runs-on`` declarations, which may be a list of labels."""
    value = _runner_value(definition)
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def _declaration_labels(declaration: object) -> set[str]:
    """Return every label one ``runs-on`` declaration can select.

    Both arms of an expression count. A label reachable only on the fork
    branch is as much in use as one reachable on the other.
    """
    text = str(declaration)
    if "${{" in text:
        return set(EXPRESSION_LITERAL.findall(text))
    return {text.strip()}


def _job_labels(definition: dict[str, object]) -> set[str]:
    """Return every label one job can select, across all its declarations."""
    return {
        label
        for declaration in _runner_declarations(definition)
        for label in _declaration_labels(declaration)
    }


def _labels_in_use() -> set[str]:
    """Return every label any lane in the repository can select."""
    return {label for definition in _jobs().values() for label in _job_labels(definition)}


def _registered_labels() -> set[str]:
    """Return the labels ``.github/actionlint.yaml`` registers."""
    assert ACTIONLINT_CONFIG.exists(), (
        "this repository uses a runner label actionlint does not know, so "
        f"{ACTIONLINT_CONFIG.relative_to(REPO_ROOT)} must exist"
    )
    config = yaml.safe_load(ACTIONLINT_CONFIG.read_text(encoding="utf-8")) or {}
    return set((config.get("self-hosted-runner") or {}).get("labels") or [])


def test_every_job_is_pinned_by_coordinate() -> None:
    """Scenario: a lane is added and nobody decides where it runs.

    Invariant: the pinned coordinates and the tree's jobs are the same set.
    Iterating only the pinned coordinates would never examine a job nobody
    listed, so a new lane could carry any label at all and satisfy every
    other assertion here.
    """
    observed = set(_jobs())
    expected = set(EXPECTED_PLACEMENT) | DELEGATED_JOBS
    assert observed == expected, (
        "runner placement is pinned per job; unpinned jobs "
        f"{sorted(observed - expected)} and stale pins "
        f"{sorted(expected - observed)} must be reconciled"
    )


@pytest.mark.parametrize(
    ("coordinate", "label"),
    sorted(
        (key, value) for key, value in EXPECTED_PLACEMENT.items() if value is not None
    ),
    ids=_case_id,
)
def test_the_job_carries_its_reviewed_label(
    coordinate: tuple[str, str], label: str
) -> None:
    """Scenario: a lane drifts back to Namespace, or onto a larger shape.

    Invariant: the job declares exactly the reviewed label and nothing
    beside it. Equality, not containment: a substring reading lets
    ``ubicloud-standard-8`` satisfy a check for ``standard-2``, and a list
    reading lets a second label ride along.
    """
    declared = _runner_value(_jobs()[coordinate])
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
    value = _runner_value(_jobs()[FORK_FALLBACK_JOB])
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
    value = str(_runner_value(_jobs()[FORK_FALLBACK_JOB]))
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
    ids=_case_id,
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
    declared = _jobs()[coordinate].get("timeout-minutes")
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
        for coordinate, definition in _jobs().items()
        if any(
            label.startswith(UBICLOUD_LABEL_PREFIX)
            for label in _job_labels(definition)
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
    definition = _jobs()[CEILING_FREE_JOB]
    assert definition.get("timeout-minutes") is None, (
        f"{CEILING_FREE_JOB[0]}:{CEILING_FREE_JOB[1]} must not declare a "
        "ceiling: it sleeps for the caller's delay_minutes input, and a "
        "ceiling would cancel a legitimate longer delay"
    )
    assert _runner_value(definition) == GITHUB_HOSTED_LABEL, (
        "the ceiling-free exception is only safe on a runner that does not "
        f"bill per minute, so this lane must stay on {GITHUB_HOSTED_LABEL!r}"
    )


@pytest.mark.parametrize("coordinate", sorted(DELEGATED_JOBS), ids=_case_id)
def test_a_reusable_caller_declares_no_runner(coordinate: tuple[str, str]) -> None:
    """Scenario: a scheduled or administrative lane acquires a runner label.

    Invariant: these coordinates call a reusable workflow and declare no
    runner, so the callee places them. A ``runs-on`` appearing here is the
    repository taking a placement decision it has not reviewed.
    """
    definition = _jobs()[coordinate]
    assert "uses" in definition, (
        f"{coordinate[0]}:{coordinate[1]} is recorded as a reusable-workflow "
        "caller but declares no uses key"
    )
    assert _runner_value(definition) is None, (
        f"{coordinate[0]}:{coordinate[1]} calls a reusable workflow, so its "
        f"runner is the callee's; found runs-on {_runner_value(definition)!r}"
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
    needing_registration = _labels_in_use() - set(GITHUB_HOSTED_LABELS)
    registered = _registered_labels()
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
        for label in _labels_in_use()
        if any(fragment in label.lower() for fragment in FOREIGN_RUNNER_FRAGMENTS)
    )
    assert not offenders, (
        "this repository has no Namespace, Windows, macOS or ad-hoc "
        f"self-hosted lane; found {offenders}"
    )
