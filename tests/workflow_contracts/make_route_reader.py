"""Read a workflow step's shell script for a ``make install-whitaker`` route.

CI installs Whitaker through the shared action; the Makefile target is the
contributor's route. A step that runs the target after the action would
reinstall the contributor-pinned suite over the action's, so the CI contract
rejects it. This module answers whether a script runs that target. It reads
the script the way the shell does rather than searching its text: line
continuations are joined, quoted text is data, a comment ends its line,
control operators separate commands, and a command's environment and wrapper
prefixes are skipped.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

TARGET = "install-whitaker"

#: A token made only of shell control characters (``;``, ``&&``, ``||``, ``|``,
#: ``&``, parentheses) separates commands.
_OPERATOR_RE = re.compile(r"^[;&|()]+$")
_ASSIGNMENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
_LINE_CONTINUATION_RE = re.compile(r"\\\n")

#: Words that run the command after them, so the command is still `make`.
_WRAPPERS: typ.Final = frozenset({"env", "command", "exec", "time", "sudo", "nice"})


def _tokens(line: str) -> list[str] | None:
    """Return a line's shell tokens, or None when its quoting is unbalanced.

    ``shlex`` in POSIX mode with punctuation splitting treats quoted text as
    data, drops comments, and returns control operators as their own tokens.
    """
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    try:
        return list(lexer)
    except ValueError:
        return None


def _commands(tokens: list[str]) -> list[list[str]]:
    """Split tokens into commands at control operators."""
    commands: list[list[str]] = [[]]
    for token in tokens:
        if _OPERATOR_RE.fullmatch(token):
            commands.append([])
        else:
            commands[-1].append(token)
    return [command for command in commands if command]


def _is_make_install(command: list[str]) -> bool:
    """Return whether a command is ``make`` with the target among its words."""
    words = list(command)
    while words and (_ASSIGNMENT_RE.match(words[0]) or words[0] in _WRAPPERS):
        words.pop(0)
    if not words:
        return False
    program = words[0].rsplit("/", 1)[-1]
    if program in {"sh", "bash"} and "-c" in words[1:-1]:
        # `bash -c 'make install-whitaker'` runs the quoted script.
        return runs_make_install_whitaker(words[words.index("-c") + 1])
    return program == "make" and TARGET in words[1:]


def runs_make_install_whitaker(run: str) -> bool:
    """Return whether a step's script runs ``make`` on ``install-whitaker``.

    A script whose quoting cannot be parsed is reported as a route when its
    text names the target after ``make``, because a contract that cannot read
    a script must not pass it.

    Parameters
    ----------
    run
        The step's ``run`` script.

    Returns
    -------
    bool
        True when any command in the script is such a ``make`` invocation.

    Examples
    --------
    >>> runs_make_install_whitaker("make -s install-whitaker")
    True
    >>> runs_make_install_whitaker("echo 'make install-whitaker'")
    False
    """
    for line in _LINE_CONTINUATION_RE.sub(" ", run).splitlines():
        tokens = _tokens(line)
        if tokens is None:
            if re.search(rf"\bmake\b.*\b{TARGET}\b", line):
                return True
            continue
        if any(_is_make_install(command) for command in _commands(tokens)):
            return True
    return False
