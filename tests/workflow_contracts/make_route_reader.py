"""Read a workflow step's shell script for a ``make install-whitaker`` route.

CI installs Whitaker through the shared action; the Makefile target is the
contributor's route. A step that runs the target after the action would
reinstall the contributor-pinned suite over the action's, so the CI contract
rejects it. This module answers whether a script runs that target. It reads
the script the way the shell does rather than searching its text, using
``shell_script_reader`` for the lexical work, and then skips a command's
control keywords, environment assignments and wrapper prefixes (with their
options). Command substitutions and ``sh -c`` scripts are read as scripts in
their own right.

Shell is not parsed completely. Where this reader cannot be sure, it fails
closed: a line with an unterminated quote is a route if it names ``make``
before the target, and a command substitution is scanned even inside quotes,
so text that only prints one is reported rather than passed. A script that
merely names the target, such as ``echo 'make install-whitaker'``, is not a
route.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

from shell_script_reader import commands, script_lines, substitutions, tokenize

TARGET = "install-whitaker"

_ASSIGNMENT: typ.Final = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
#: A short-option cluster containing `c`, which makes a shell run its next word.
_COMMAND_OPTION: typ.Final = re.compile(r"-[A-Za-z]*c[A-Za-z]*")
#: Words that run the command after them, so the command is still `make`.
_WRAPPERS: typ.Final = frozenset({"env", "command", "exec", "time", "sudo", "nice"})
#: Words that introduce the command after them (`if make ...; then`).
_KEYWORDS: typ.Final = frozenset(
    {"if", "then", "elif", "else", "do", "while", "until", "!", "{", "}"}
)
_SHELLS: typ.Final = frozenset({"sh", "bash"})
_PROGRAMS: typ.Final = frozenset({"make", *_SHELLS})


def _program(word: str) -> str:
    """Return a command word's program name, without any directory."""
    return word.rsplit("/", 1)[-1]


def _after_prefixes(words: list[str]) -> list[str]:
    """Return a command's words without keywords, assignments and wrappers.

    A wrapper such as ``env`` or ``sudo`` is dropped along with its options
    and their arguments, up to the first program word.
    """
    remaining = list(words)
    while remaining and (remaining[0] in _KEYWORDS or _ASSIGNMENT.match(remaining[0])):
        remaining.pop(0)
    if remaining and remaining[0] in _WRAPPERS:
        starts = [i for i, word in enumerate(remaining) if _program(word) in _PROGRAMS]
        remaining = remaining[starts[0] :] if starts else []
    return remaining


def _shell_script(words: list[str]) -> str | None:
    """Return the script a ``sh -c`` command runs, or None if it runs none.

    Combined short options count: ``bash -ec 'script'`` runs ``script``.
    """
    for position, word in enumerate(words[1:-1], start=1):
        if _COMMAND_OPTION.fullmatch(word):
            return words[position + 1]
    return None


def _is_make_install(words: list[str]) -> bool:
    """Return whether one command is ``make`` with the target among its words."""
    command = _after_prefixes(shlex.split(" ".join(words)))
    if not command:
        return False
    if _program(command[0]) in _SHELLS:
        script = _shell_script(command)
        return script is not None and runs_make_install_whitaker(script)
    return _program(command[0]) == "make" and TARGET in command[1:]


def _line_runs_make_install(line: str) -> bool:
    """Return whether one line of a script runs ``make`` on the target."""
    try:
        tokens = tokenize(line)
        return any(_is_make_install(words) for words in commands(tokens)) or any(
            runs_make_install_whitaker(inner) for inner in substitutions(tokens)
        )
    except ValueError:
        return re.search(rf"\bmake\b.*\b{TARGET}\b", line) is not None


def runs_make_install_whitaker(run: str) -> bool:
    """Return whether a step's script runs ``make`` on ``install-whitaker``.

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
    >>> runs_make_install_whitaker("if make install-whitaker; then :; fi")
    True
    >>> runs_make_install_whitaker("bash -ec 'make install-whitaker'")
    True
    >>> runs_make_install_whitaker("echo 'make install-whitaker'")
    False
    """
    return any(_line_runs_make_install(line) for line in script_lines(run))
