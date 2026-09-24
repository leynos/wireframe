"""Prove the runner-placement readings against constructed job definitions.

Wireframe's own workflows declare every runner as a scalar, so a reading that
mishandles the sequence or mapping form of ``runs-on`` passes against them
either way. Each form is driven here with a constructed job, and the last
test holds every job in the tree to a form the reader models.
"""

from __future__ import annotations

import pytest
from runner_placement_policy import UBICLOUD_LABEL_PREFIX
from runner_placement_reader import (
    RunnerShapeError,
    case_id,
    job_labels,
    jobs,
)


@pytest.mark.parametrize(
    ("runs_on", "expected"),
    [
        pytest.param("ubuntu-latest", {"ubuntu-latest"}, id="scalar"),
        pytest.param(
            ["self-hosted", "ubicloud-standard-4"],
            {"self-hosted", "ubicloud-standard-4"},
            id="sequence",
        ),
        pytest.param(
            {"labels": "ubicloud-standard-4"},
            {"ubicloud-standard-4"},
            id="mapping-scalar-labels",
        ),
        pytest.param(
            {"labels": ["ubicloud-standard-4", "arm"]},
            {"ubicloud-standard-4", "arm"},
            id="mapping-sequence-labels",
        ),
        pytest.param(
            "${{ x && 'ubuntu-latest' || 'ubicloud-standard-4' }}",
            {"ubuntu-latest", "ubicloud-standard-4"},
            id="fork-fallback",
        ),
    ],
)
def test_every_runs_on_form_is_read(runs_on: object, expected: set[str]) -> None:
    """Scalar, sequence and ``labels`` mapping each yield their labels.

    The mapping rows are the ones that matter: a reader that stringified the
    mapping would report one label beginning ``{``, which no Ubicloud prefix
    test matches, so a paid lane would escape its ceiling.
    """
    assert job_labels({"runs-on": runs_on}) == expected


def test_a_mapping_placed_lane_is_seen_as_ubicloud() -> None:
    """The ceiling rule's own predicate sees a mapping-form Ubicloud lane."""
    labels = job_labels({"runs-on": {"labels": ["ubicloud-standard-2"]}})
    assert any(label.startswith(UBICLOUD_LABEL_PREFIX) for label in labels)


@pytest.mark.parametrize(
    "runs_on",
    [
        pytest.param({"group": "paid", "labels": "linux"}, id="group"),
        pytest.param({"group": "paid"}, id="group-only"),
        pytest.param({"labels": "linux", "extra": 1}, id="unknown-key"),
        pytest.param({}, id="empty-mapping"),
        pytest.param({"labels": {"labels": "linux"}}, id="nested-mapping"),
        pytest.param([], id="empty-sequence"),
        pytest.param(["linux", 4], id="non-string-label"),
        pytest.param(4, id="number"),
        pytest.param("${{ matrix.os }}", id="matrix-expression"),
        pytest.param("${{ matrix.os || 'ubuntu-latest' }}", id="matrix-with-fallback"),
        pytest.param("${{ inputs.runner }}", id="caller-input"),
        pytest.param(None, id="explicit-null"),
    ],
)
def test_an_unreadable_runs_on_is_refused(runs_on: object) -> None:
    """Refused: reading it as "no runner" exempts it from everything.

    ``${{ inputs.runner }}`` is refused too: no workflow here takes its
    runner from a caller, so the reader has no caller-side input to read the
    labels from.
    """
    with pytest.raises(RunnerShapeError):
        job_labels({"runs-on": runs_on})


@pytest.mark.parametrize("coordinate", sorted(jobs()), ids=case_id)
def test_every_runner_declaration_is_readable(
    coordinate: tuple[str, str],
) -> None:
    """Scenario: a lane is placed in a form the reader does not model.

    Invariant: every job's declarations read as a scalar, a sequence or a
    ``labels`` mapping, and every expression among them is the fork
    fallback. A form the reader cannot see would otherwise read as "no
    runner", which exempts the lane from every placement, ceiling and
    registry assertion here at once; this names the coordinate instead.
    """
    try:
        job_labels(jobs()[coordinate])
    except RunnerShapeError as error:
        pytest.fail(f"{coordinate[0]}:{coordinate[1]}: {error}")
