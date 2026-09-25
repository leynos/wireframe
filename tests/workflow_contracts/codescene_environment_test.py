"""Prove the `codescene` environment sits on the uploading job alone.

Each test mutates a copy of this repository's workflows the way a later edit
could, and asserts the clause meant to catch it does. The check step, the ref
guard and `access-token:` stay held by the repository contract.
"""

from __future__ import annotations

import copy
import typing as typ

import pytest
from codescene_environment import (
    MISSING,
    REACHABLE,
    STRAY,
    UPLOAD_ACTION,
    environment_violations,
)
from codescene_placement_reader import jobs
from workflow_loader import Document, repository_workflows

PUBLISHER: typ.Final[str] = "coverage-main.yml"
LANE: typ.Final[str] = "ci.yml"


@pytest.fixture
def documents() -> dict[str, Document]:
    """Return a private copy of the repository's workflows to mutate.

    Returns
    -------
    dict[str, Document]
        The parsed workflows, deep-copied for this test alone.

    """
    return copy.deepcopy(repository_workflows())


def _first_job(documents: dict[str, Document], name: str) -> Document:
    """Return one workflow's first job, for mutation in place.

    Returns
    -------
    dict[str, object]
        The job mapping.

    """
    return next(iter(jobs(documents[name]).values()))


def _reports(documents: dict[str, Document], fragment: str) -> None:
    """Fail unless the rule reports a violation containing `fragment`."""
    found = environment_violations(documents)
    assert any(fragment in problem for problem in found), (
        f"expected a violation naming {fragment!r}, got {found}"
    )


def test_repository_places_the_environment(documents: dict[str, Document]) -> None:
    """The publisher declares the environment and nothing else does."""
    found = environment_violations(documents)
    assert not found, f"expected no violations, got {found}"


def test_publisher_cannot_drop_the_environment(documents: dict[str, Document]) -> None:
    """Without it the moved token never reaches the upload, which then skips."""
    del _first_job(documents, PUBLISHER)["environment"]
    _reports(documents, MISSING)


def test_publisher_cannot_name_another_environment(
    documents: dict[str, Document],
) -> None:
    """Another environment holds no CodeScene token."""
    _first_job(documents, PUBLISHER)["environment"] = "production"
    _reports(documents, MISSING)


def test_mapping_form_is_accepted(documents: dict[str, Document]) -> None:
    """`{name: codescene}` is the same declaration as the bare string."""
    _first_job(documents, PUBLISHER)["environment"] = {"name": "codescene"}
    found = environment_violations(documents)
    assert not found, f"the mapping form must be accepted, got {found}"


def test_no_other_job_may_declare_it(documents: dict[str, Document]) -> None:
    """A second holder of the token widens what can read it."""
    declared = typ.cast("dict[str, object]", documents[PUBLISHER]["jobs"])
    declared["other"] = {
        "runs-on": "ubuntu-latest",
        "environment": "codescene",
        "steps": [{"run": "true"}],
    }
    _reports(documents, STRAY)


def test_no_pull_request_job_may_declare_it(documents: dict[str, Document]) -> None:
    """A pull request's own code must never be able to request the token."""
    _first_job(documents, LANE)["environment"] = {"name": "codescene"}
    _reports(documents, REACHABLE)


def test_an_empty_reading_is_refused(documents: dict[str, Document]) -> None:
    """With no uploader left the rule says so rather than passing."""
    job = _first_job(documents, PUBLISHER)
    job["steps"] = [
        step
        for step in typ.cast("list[dict[str, object]]", job["steps"])
        if UPLOAD_ACTION not in str(step.get("uses", ""))
    ]
    _reports(documents, "no workflow job invokes")
