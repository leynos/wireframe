"""Hold the ``make install-whitaker`` route detector to the shell's reading.

The CI contract is only as strong as this detector, so it is driven over
constructed scripts in both directions: every spelling of the Make route is
reported, and text that merely names the target is not.

Run these tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import pytest
from make_route_reader import runs_make_install_whitaker


@pytest.mark.parametrize(
    "run",
    [
        "make install-whitaker",
        "make -s install-whitaker",
        "make --no-print-directory install-whitaker",
        "make install-whitaker lint",
        "cd . && make install-whitaker",
        "make lint; make install-whitaker",
        "make lint || make install-whitaker",
        "(make install-whitaker)",
        "make -s \\\n  install-whitaker",
        "set -e\nmake install-whitaker\n",
        "make install-whitaker # install the suite",
        "env X=1 make install-whitaker",
        "MAKEFLAGS=-s make install-whitaker",
        "FOO=1 BAR=2 make install-whitaker",
        "command make install-whitaker",
        "/usr/bin/make install-whitaker",
        "printf '%s' 'hello # world'; make install-whitaker",
        "bash -c 'make install-whitaker'",
        "echo 'unbalanced; make install-whitaker",
        "echo x#y; make install-whitaker",
        "echo '&&' ; make install-whitaker",
        "env -i make install-whitaker",
        "sudo -n make install-whitaker",
        "sudo -u builder env -i make install-whitaker",
        "make install-whitaker && make lint",
        "echo ok\nmake install-whitaker\nmake lint",
        "make lint\nmake install-whitaker",
    ],
    ids=[
        "plain",
        "silent-option",
        "long-option",
        "extra-target",
        "after-and",
        "after-semicolon",
        "after-or",
        "subshell",
        "line-continuation",
        "multiline",
        "trailing-comment",
        "env-prefix",
        "assignment-prefix",
        "two-assignments",
        "command-prefix",
        "absolute-path",
        "hash-inside-quotes",
        "shell-dash-c",
        "unparsable-quoting",
        "hash-inside-a-word",
        "quoted-operator-then-real-command",
        "env-with-option",
        "sudo-with-option",
        "wrapper-options-with-arguments",
        "install-then-lint",
        "install-after-echo",
        "install-after-lint-line",
    ],
)
def test_every_spelling_of_the_make_route_is_reported(run: str) -> None:
    """Report a Make install however the script spells it."""
    assert runs_make_install_whitaker(run), run


@pytest.mark.parametrize(
    "run",
    [
        "make lint",
        "make test",
        "make check-fmt",
        "echo install-whitaker",
        "echo make install-whitaker",
        "echo 'example; make install-whitaker'",
        "# make install-whitaker",
        "make lint # then make install-whitaker",
        "printf '%s\\n' install-whitaker",
        "make lint \\\n  # make install-whitaker",
        "echo '&&' make install-whitaker",
        "echo \"a; make install-whitaker\"",
        "env -i make lint",
        "sudo -n make lint",
        "echo x # make install-whitaker",
    ],
    ids=[
        "lint",
        "test",
        "check-fmt",
        "echoed-target",
        "echoed-command",
        "quoted-separator",
        "comment",
        "trailing-comment-text",
        "printf",
        "commented-continuation",
        "quoted-operator-is-text",
        "double-quoted-separator",
        "wrapper-then-lint",
        "sudo-then-lint",
        "comment-after-echo",
    ],
)
def test_text_naming_the_target_is_not_a_route(run: str) -> None:
    """Leave other Make targets, comments and quoted text alone."""
    assert not runs_make_install_whitaker(run), run
