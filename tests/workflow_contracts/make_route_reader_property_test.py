"""Drive the ``make install-whitaker`` route detector across generated scripts.

The named cases pin the spellings reviewers found. These property cases build
an install command from independent parts (a keyword or chain, an environment
or wrapper prefix, Make options, extra targets, a shell wrapper and its
quoting) and place it anywhere in a script of harmless lines, so a detector
that only recognizes the examples it was written against fails here. The
converse holds too: a script of harmless lines, or of lines that merely print
or comment the install, is never reported, and neither is the same command
built for another target.
"""

from __future__ import annotations

from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from make_route_reader import runs_make_install_whitaker

PROPERTY_SETTINGS = settings(
    max_examples=200, deadline=None, suppress_health_check=[HealthCheck.too_slow]
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
PREFIXES = ("", "env ", "env -i ", "sudo ", "sudo -n ", "MAKEFLAGS=-s ", "FOO=1 BAR=2 ")
OPTIONS = ("", "-s ", "--no-print-directory ", "-j4 -s ")
BEFORE_TARGET = ("", "lint ")
AFTER_TARGET = ("", " lint", " test")
CHAINS = (
    "{command}",
    "if {command}; then :; fi",
    "echo ok && {command}",
    "make lint; {command}",
    "true || {command}",
    "while ! {command}; do sleep 1; done",
    "({command})",
    "{{ {command}; }}",
    'echo "$({command})"',
    "x=$({command})",
)
SHELLS = ("", "bash -c ", "bash -ec ", "sh -euc ", "/bin/bash -lc ")


def _wrap_in_shell(command: str, shell: str) -> str:
    """Return `command` run through `shell`, quoted as one argument."""
    return f"{shell}'{command}'" if shell else command


@st.composite
def make_commands(draw: st.DrawFn, target: str) -> str:
    """Draw a `make` invocation of `target`, with prefixes, options and quoting."""
    command = (
        f"{draw(st.sampled_from(PREFIXES))}make {draw(st.sampled_from(OPTIONS))}"
        f"{draw(st.sampled_from(BEFORE_TARGET))}{target}"
        f"{draw(st.sampled_from(AFTER_TARGET))}"
    )
    return _wrap_in_shell(command, draw(st.sampled_from(SHELLS)))


@st.composite
def install_lines(draw: st.DrawFn, target: str = "install-whitaker") -> str:
    """Draw one script line that runs `make` on `target` inside a chain."""
    chain = draw(st.sampled_from(CHAINS))
    command = draw(make_commands(target))
    # A chain that nests the command in a shell wrapper's single quotes cannot
    # hold the single quotes the wrapper adds, so such a chain runs it bare.
    if "'" in command and "$(" in chain:
        chain = "{command}"
    return chain.format(command=command)


harmless_lines = st.lists(st.sampled_from(HARMLESS_LINES), max_size=6)


@PROPERTY_SETTINGS
@given(before=harmless_lines, install=install_lines(), after=harmless_lines)
def test_an_install_line_is_found_wherever_it_sits(
    before: list[str], install: str, after: list[str]
) -> None:
    """Report the install however it is composed and wherever it is placed."""
    script = "\n".join([*before, install, *after])
    assert runs_make_install_whitaker(script), script


@PROPERTY_SETTINGS
@given(before=harmless_lines, other=install_lines(target="lint"), after=harmless_lines)
def test_the_same_command_for_another_target_is_not_reported(
    before: list[str], other: str, after: list[str]
) -> None:
    """Leave alone the same composed command when it runs another target."""
    script = "\n".join([*before, other, *after])
    assert not runs_make_install_whitaker(script), script


@PROPERTY_SETTINGS
@given(lines=harmless_lines)
def test_harmless_lines_are_never_reported(lines: list[str]) -> None:
    """Leave scripts alone that only print, comment or run other targets."""
    script = "\n".join(lines)
    assert not runs_make_install_whitaker(script), script
