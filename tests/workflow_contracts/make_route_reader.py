"""Read a workflow step's shell script for a ``make install-whitaker`` route.

CI installs Whitaker through the shared action; the Makefile target is the
contributor's route. A step that runs the target after the action would
reinstall the contributor-pinned suite over the action's, so the CI contract
rejects it. This module answers whether a script runs that target. It reads
the script the way the shell does rather than searching its text: line
continuations are joined, quoted text is data, a comment starts only at the
beginning of a word, unquoted control operators separate commands, and a
command's environment and wrapper prefixes (with their options) are skipped.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

TARGET = "install-whitaker"

_ASSIGNMENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
_LINE_CONTINUATION_RE = re.compile(r"\\\n")

#: Characters that separate commands when they appear outside quotes.
_SEPARATORS: typ.Final = frozenset(";&|()")

#: Words that run the command after them, so the command is still `make`.
_WRAPPERS: typ.Final = frozenset({"env", "command", "exec", "time", "sudo", "nice"})


def _split_commands(line: str) -> list[str]:
    """Split one line into command strings at unquoted control operators.

    A ``#`` that begins a word outside quotes starts a comment and ends the
    line, whereas ``x#y`` and a quoted ``'#'`` are text.

    Raises
    ------
    ValueError
        If a quote is left open, so the line cannot be read reliably.
    """
    commands: list[str] = []
    current: list[str] = []
    quote = ""
    previous = " "
    index = 0
    while index < len(line):
        char = line[index]
        if quote:
            current.append(char)
            if char == "\\" and quote == '"' and index + 1 < len(line):
                index += 1
                current.append(line[index])
            elif char == quote:
                quote = ""
        elif char == "\\" and index + 1 < len(line):
            current.extend((char, line[index + 1]))
            index += 1
        elif char in "'\"":
            quote = char
            current.append(char)
        elif char == "#" and (previous.isspace() or previous in _SEPARATORS):
            break
        elif char in _SEPARATORS:
            commands.append("".join(current))
            current = []
        else:
            current.append(char)
        previous = char
        index += 1
    if quote:
        message = "unterminated quote"
        raise ValueError(message)
    commands.append("".join(current))
    return [command.strip() for command in commands if command.strip()]


def _after_prefixes(words: list[str]) -> list[str]:
    """Return a command's words with environment and wrapper prefixes removed.

    Assignments such as ``MAKEFLAGS=-s`` are dropped. A wrapper such as
    ``env`` or ``sudo`` is dropped along with its options and their arguments,
    up to the first word that is a program (``make``) rather than an option.
    """
    remaining = list(words)
    while remaining and _ASSIGNMENT_RE.match(remaining[0]):
        remaining.pop(0)
    if remaining and remaining[0] in _WRAPPERS:
        programs = [
            position
            for position, word in enumerate(remaining)
            if word.rsplit("/", 1)[-1] in {"make", "sh", "bash"}
        ]
        remaining = remaining[programs[0] :] if programs else []
    return remaining


def _is_make_install(command: str) -> bool:
    """Return whether one command is ``make`` with the target among its words."""
    words = _after_prefixes(shlex.split(command, comments=False, posix=True))
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
        try:
            if any(_is_make_install(command) for command in _split_commands(line)):
                return True
        except ValueError:
            if re.search(rf"\bmake\b.*\b{TARGET}\b", line):
                return True
    return False
