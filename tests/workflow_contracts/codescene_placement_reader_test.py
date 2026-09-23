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
    calls,
    external_secret_inheritors,
    local_callee,
    mentions,
    pull_request_closure,
    qualified_self_call,
    qualified_self_callers,
    triggers,
)
from workflow_loader import load_workflow, read_workflows


def _tree(tmp_path: Path, files: dict[str, str]) -> Path:
    """Write workflow files into a directory and return it."""
    for name, text in files.items():
        (tmp_path / name).write_text(textwrap.dedent(text), encoding="utf-8")
    return tmp_path


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
            None,
            id="qualified-self",
        ),
        pytest.param(
            "./.github/workflows/release.yml@main", None, id="local-with-ref"
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
    """Every spelling that runs a checked-out workflow file is local.

    The rows returning ``None`` are the narrowness half. A call to this
    repository by ref runs the file at that ref, not the checked-out one, so
    it is not local either; ``qualified_self_call`` identifies it instead.
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
              self-by-ref:
                uses: leynos/wireframe/.github/workflows/a.yml@main
                secrets: inherit
            """
        )
    )
    assert external_secret_inheritors(document) == ["foreign", "self-by-ref"]


@pytest.mark.parametrize(
    ("uses", "expected"),
    [
        pytest.param(
            "leynos/wireframe/.github/workflows/x.yml@main",
            True,
            id="qualified",
        ),
        pytest.param(
            "Leynos/Wireframe/.github/workflows/x.yml@v1",
            True,
            id="qualified-case",
        ),
        pytest.param("./.github/workflows/x.yml", False, id="local"),
        pytest.param(
            "leynos/wireframe/.github/workflows/x.yml", False, id="no-ref"
        ),
        pytest.param(
            "leynos/wireframe-fork/.github/workflows/x.yml@main",
            False,
            id="similar-name",
        ),
        pytest.param(
            "leynos/wireframe/.github/actions/setup@main",
            False,
            id="own-action",
        ),
        pytest.param(
            "other/repo/.github/workflows/x.yml@v1", False, id="other-repo"
        ),
    ],
)
def test_a_call_to_this_repository_by_ref_is_recognized(
    uses: str, expected: bool
) -> None:
    """The form the contract refuses: this repository's workflow at a ref.

    GitHub runs it at the named ref, so the checked-out file the closure
    reads is not the one that runs. The ``False`` rows keep the refusal
    narrow: a local call, another repository, a repository whose name
    merely begins the same way, and this repository's own actions.
    """
    assert qualified_self_call(uses) is expected


def test_callers_by_ref_are_listed() -> None:
    """``qualified_self_callers`` names the jobs the contract refuses."""
    document = load_workflow(
        "on: pull_request\n"
        "jobs:\n"
        "  ok: {uses: ./.github/workflows/a.yml}\n"
        "  pinned: {uses: leynos/wireframe/.github/workflows/a.yml@main}\n"
    )
    assert qualified_self_callers(document) == ["pinned"]


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
