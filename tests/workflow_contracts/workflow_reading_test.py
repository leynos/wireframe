"""Prove the workflow readings against constructed workflow trees.

The contracts next door run over this repository's own workflows, which all
happen to be written the one way the first reader understood. A reading that
mishandles a shape nobody here uses passes there either way, so each reading
is driven here with the shape it must handle.
"""

from __future__ import annotations

import pytest
from workflow_loader import load_workflow
from workflow_reading import triggers


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        pytest.param("on: pull_request\n", ["pull_request"], id="scalar"),
        pytest.param(
            "on: [push, pull_request]\n",
            ["push", "pull_request"],
            id="sequence",
        ),
        pytest.param(
            "on:\n  push:\n  pull_request:\n",
            ["push", "pull_request"],
            id="mapping",
        ),
        pytest.param(
            "'on': pull_request\n", ["pull_request"], id="quoted-key"
        ),
    ],
)
def test_every_trigger_form_is_read(text: str, expected: list[str]) -> None:
    """Scalar, sequence and mapping, under the boolean and the string key.

    Parsed through the loader the contract uses, since only a resolving
    loader turns an unquoted ``on`` into ``True``; a reader proved against
    hand-built dictionaries would not show that.
    """
    assert list(triggers(load_workflow(text))) == expected, (
        f"{text!r} should read as triggers {expected}"
    )


@pytest.mark.parametrize(
    "text",
    [
        pytest.param("on: 17\n", id="number"),
        pytest.param(
            "on: [push, {pull_request: null}]\n", id="mixed-sequence"
        ),
        pytest.param("jobs: {}\n", id="missing"),
        pytest.param(
            "on: push\n'on': pull_request\njobs: {}\n", id="both-keys"
        ),
    ],
)
def test_an_unreadable_trigger_is_refused(text: str) -> None:
    """Refused: an empty reading lets the workflow escape every clause."""
    with pytest.raises(ValueError, match="on:"):
        triggers(load_workflow(text))
