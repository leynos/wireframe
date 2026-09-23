"""Prove the strict workflow loader on constructed files."""

from __future__ import annotations

import textwrap
from pathlib import Path

import pytest
from workflow_loader import DuplicateKeyError, read_workflows


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
