"""Prove the CodeScene placement readings against constructed workflow trees.

The contract in ``ci_codescene_placement_test`` runs over this repository's
own workflows, which all happen to be written the one way the first reader
understood. A reading that mishandles a shape nobody here uses passes there
either way, so each reading is driven here with the shape it must handle.
"""

from __future__ import annotations

import textwrap
import typing as typ
from pathlib import Path

import pytest
from codescene_placement_reader import (
    DuplicateKeyError,
    calls,
    external_secret_inheritors,
    load_workflow,
    local_callee,
    mentions,
    pull_request_closure,
    read_workflows,
    triggers,
)


def _tree(tmp_path: Path, files: dict[str, str]) -> Path:
    """Write workflow files into a directory and return it."""
    for name, text in files.items():
        (tmp_path / name).write_text(textwrap.dedent(text), encoding="utf-8")
    return tmp_path


def test_a_duplicate_key_is_refused(tmp_path: Path) -> None:
    """A lane declaring ``runs-on`` twice is refused, not half read.

    PyYAML keeps the last value, so the paid label in the discarded half
    would read as hosted and every placement assertion would pass over it.
    """
    directory = _tree(
        tmp_path,
        {
            "ci.yml": """\
                on: pull_request
                jobs:
                  build:
                    runs-on: ubicloud-standard-4
                    runs-on: ubuntu-latest
                    steps: []
            """
        },
    )
    with pytest.raises(DuplicateKeyError, match="runs-on"):
        read_workflows(directory)


def test_an_upper_case_extension_is_read(tmp_path: Path) -> None:
    """GitHub runs ``CI.YML``, so the reader must not skip it."""
    directory = _tree(tmp_path, {"CI.YML": "on: push\njobs: {}\n"})
    assert list(read_workflows(directory)) == ["CI.YML"]


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
    assert list(triggers(load_workflow(text))) == expected


@pytest.mark.parametrize(
    "text",
    [
        pytest.param("on: 17\n", id="number"),
        pytest.param(
            "on: [push, {pull_request: null}]\n", id="mixed-sequence"
        ),
        pytest.param("jobs: {}\n", id="missing"),
    ],
)
def test_an_unreadable_trigger_is_refused(text: str) -> None:
    """Refused: an empty reading lets the workflow escape every clause."""
    with pytest.raises(ValueError, match="on:"):
        triggers(load_workflow(text))


@pytest.mark.parametrize(
    ("uses", "expected"),
    [
        pytest.param(
            "./.github/workflows/release.yml", "release.yml", id="dot"
        ),
        pytest.param(
            "$/.github/workflows/release.yml", "release.yml", id="dollar"
        ),
        pytest.param(
            ".github/workflows/release.yml", "release.yml", id="bare"
        ),
        pytest.param(
            "leynos/wireframe/.github/workflows/release.yml@main",
            "release.yml",
            id="qualified-self",
        ),
        pytest.param(
            "Leynos/Wireframe/.github/workflows/release.yml@v1",
            "release.yml",
            id="qualified-self-case",
        ),
        pytest.param(
            "leynos/shared-actions/.github/workflows/x.yml@v1",
            None,
            id="other-repo",
        ),
        pytest.param("./.github/actions/setup", None, id="local-action"),
        pytest.param("actions/checkout@v5", None, id="marketplace-action"),
        pytest.param(
            "./.github/workflows/nested/x.yml", None, id="nested-path"
        ),
    ],
)
def test_a_local_call_is_recognized_by_shape(
    uses: str, expected: str | None
) -> None:
    """Every spelling that resolves under this repository's workflows is local.

    The last four rows are the narrowness half: another repository, an
    action and a path GitHub would not resolve as a workflow stay out.
    """
    assert local_callee(uses) == expected


PROBE: typ.Final = """\
    on: workflow_call
    jobs:
      leak:
        runs-on: ubuntu-latest
        steps:
          - run: >-
              curl -H "Authorization: ${{ secrets.CS_ACCESS_TOKEN }}"
              https://API.CodeScene.io/v2/projects
"""


@pytest.mark.parametrize(
    "spelling",
    [
        "./.github/workflows/probe.yml",
        "$/.github/workflows/probe.yml",
        "leynos/wireframe/.github/workflows/probe.yml@main",
    ],
)
def test_the_closure_reaches_a_called_workflow(
    tmp_path: Path, spelling: str
) -> None:
    """The episodic probe: a ``workflow_call`` workflow inheriting the token.

    It declares no pull-request trigger, so a trigger-only enumeration never
    reads it, while a pull-request job calls it with ``secrets: inherit`` and
    it curls CodeScene's API with the token. The closure must reach it in
    every spelling, and the token and host sweeps must then see it.
    """
    caller = f"""\
        on: pull_request
        jobs:
          call:
            uses: {spelling}
            secrets: inherit
    """
    documents = read_workflows(
        _tree(tmp_path, {"ci.yml": caller, "probe.yml": PROBE})
    )
    assert pull_request_closure(documents) == ["ci.yml", "probe.yml"]
    assert mentions(documents["probe.yml"], "CS_ACCESS_TOKEN")
    assert mentions(documents["probe.yml"], "codescene.io", ignore_case=True)


def test_the_closure_is_narrow(tmp_path: Path) -> None:
    """A reusable workflow nothing calls stays out, and cycles terminate.

    Without the first half a repository that complies would be failed by a
    dispatch-only helper; without the second the contract would hang.
    """
    documents = read_workflows(
        _tree(
            tmp_path,
            {
                "ci.yml": """\
                    on: pull_request
                    jobs:
                      a: {uses: ./.github/workflows/a.yml}
                      ext: {uses: other/repo/.github/workflows/b.yml@v1}
                """,
                "a.yml": """\
                    on: workflow_call
                    jobs:
                      back: {uses: ./.github/workflows/ci.yml}
                """,
                "b.yml": PROBE,
                "publish.yml": "on: {push: {branches: [main]}}\njobs: {}\n",
            },
        )
    )
    assert pull_request_closure(documents) == ["a.yml", "ci.yml"]


def test_only_inheritance_into_another_repository_is_flagged() -> None:
    """``secrets: inherit`` is visible locally, and invisible anywhere else."""
    document = load_workflow(
        textwrap.dedent(
            """\
            on: pull_request
            jobs:
              local: {uses: ./.github/workflows/a.yml, secrets: inherit}
              foreign:
                uses: other/repo/.github/workflows/b.yml@v1
                secrets: inherit
              named:
                uses: other/repo/.github/workflows/b.yml@v1
                secrets: {X: y}
            """
        )
    )
    assert external_secret_inheritors(document) == ["foreign"]


def test_a_job_level_call_is_read_as_a_call() -> None:
    """A job calling a reusable workflow has no steps but is still a call."""
    document = load_workflow(
        "on: pull_request\n"
        "jobs:\n"
        "  scan: {uses: codescene/scan/.github/workflows/x.yml@v1}\n"
    )
    assert [call["uses"] for call in calls(document)] == [
        "codescene/scan/.github/workflows/x.yml@v1"
    ]
