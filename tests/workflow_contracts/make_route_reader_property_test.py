"""Drive the ``make install-whitaker`` route detector across generated scripts.

The named cases pin the spellings reviewers found. These property cases draw a
script of harmless lines, then place one install line anywhere in it, in any
spelling, so a detector that only recognizes the examples it was written
against fails here. The converse holds too: a script of harmless lines, or of
lines that merely print or comment the install, is never reported.
"""

from __future__ import annotations

from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from make_route_reader import runs_make_install_whitaker

PROPERTY_SETTINGS = settings(
    max_examples=60, deadline=None, suppress_health_check=[HealthCheck.too_slow]
)

HARMLESS_LINES = (
    "make lint",
    "make test",
    "echo ok",
    "cargo build --release",
    "set -euo pipefail",
    "cd subdir",
    "# make install-whitaker",
    "echo 'make install-whitaker'",
    'echo "a; make install-whitaker"',
    "echo '&&' make install-whitaker",
    "make lint # then make install-whitaker",
)
INSTALL_SPELLINGS = (
    "make install-whitaker",
    "make -s install-whitaker",
    "make --no-print-directory install-whitaker lint",
    "env -i make install-whitaker",
    "sudo -n make install-whitaker",
    "MAKEFLAGS=-s make install-whitaker",
    "if make install-whitaker; then :; fi",
    "echo ok && make install-whitaker",
    "make lint; make install-whitaker",
    'echo "$(make install-whitaker)"',
    "bash -c 'make install-whitaker'",
    "make -s \\\n  install-whitaker",
)

harmless_lines = st.lists(st.sampled_from(HARMLESS_LINES), max_size=6)


@PROPERTY_SETTINGS
@given(
    before=harmless_lines,
    install=st.sampled_from(INSTALL_SPELLINGS),
    after=harmless_lines,
)
def test_an_install_line_is_found_wherever_it_sits(
    before: list[str], install: str, after: list[str]
) -> None:
    """Report the install however it is spelled and wherever it is placed."""
    script = "\n".join([*before, install, *after])
    assert runs_make_install_whitaker(script), script


@PROPERTY_SETTINGS
@given(lines=harmless_lines)
def test_harmless_lines_are_never_reported(lines: list[str]) -> None:
    """Leave scripts alone that only print, comment or run other targets."""
    script = "\n".join(lines)
    assert not runs_make_install_whitaker(script), script
