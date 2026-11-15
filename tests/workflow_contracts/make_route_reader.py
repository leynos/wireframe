"""Read a workflow step's shell script for a ``make install-whitaker`` route.

CI installs Whitaker through the shared action; the Makefile target is the
contributor's route. A step that runs the target after the action would
reinstall the contributor-pinned suite over the action's, so the CI contract
rejects it. This module answers whether a script runs that target. It reads
the script the way the shell does rather than searching its text: line
continuations are joined, quoted text is data, a comment starts only at the
beginning of a word, unquoted control operators separate commands, and a
command's control keywords, environment assignments and wrapper prefixes
(with their options) are skipped. Command substitutions are read as scripts
in their own right, wherever they appear.

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

TARGET = "install-whitaker"

#: One token at a position: a word-initial comment, a word (quoted segments
#: included, so a quoted operator is part of its word), a run of control
#: operators, or whitespace.
_TOKEN: typ.Final = re.compile(
    r"""
    (?P<comment>(?:^|(?<=[\s;&|()]))\#.*)
    |(?P<word>(?:'[^']*'|"(?:\\.|[^"\\])*"|\\.|[^\s;&|()'"\\])+)
    |(?P<operator>[;&|()]+)
    |(?P<space>\s+)
    """,
    re.VERBOSE,
)
_ASSIGNMENT_RE: typ.Final = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
_LINE_CONTINUATION_RE: typ.Final = re.compile(r"\\\n")
_SUBSTITUTION_RE: typ.Final = re.compile(r"\$\(([^()]*)\)|`([^`]*)`")

#: Words that run the command after them, so the command is still `make`.
_WRAPPERS: typ.Final = frozenset({"env", "command", "exec", "time", "sudo", "nice"})
#: Words that introduce the command after them (`if make ...; then`).
_KEYWORDS: typ.Final = frozenset(
    {"if", "then", "elif", "else", "do", "while", "until", "!", "{", "}"}
)
_PROGRAMS: typ.Final = frozenset({"make", "sh", "bash"})


def _tokenize(line: str) -> list[tuple[str, str]]:
    """Return a line's ``(kind, text)`` tokens, ending at a comment.

    Raises
    ------
    ValueError
        If the line cannot be tokenized, which is an unterminated quote.
    """
    tokens: list[tuple[str, str]] = []
    position = 0
    while position < len(line):
        match = _TOKEN.match(line, position)
        if match is None:
            message = f"cannot read {line[position:]!r}"
            raise ValueError(message)
        position = match.end()
        if match.lastgroup == "comment":
            break
        if match.lastgroup != "space":
            tokens.append((str(match.lastgroup), match.group()))
    return tokens


def _commands(tokens: list[tuple[str, str]]) -> list[list[str]]:
    """Group word tokens into commands, split at control operators."""
    commands: list[list[str]] = [[]]
    for kind, text in tokens:
        if kind == "operator":
            commands.append([])
        else:
            commands[-1].append(text)
    return [command for command in commands if command]


def _after_prefixes(words: list[str]) -> list[str]:
    """Return a command's words without keywords, assignments and wrappers.

    A wrapper such as ``env`` or ``sudo`` is dropped along with its options
    and their arguments, up to the first program word.
    """
    remaining = list(words)
    while remaining and (
        remaining[0] in _KEYWORDS or _ASSIGNMENT_RE.match(remaining[0])
    ):
        remaining.pop(0)
    if remaining and remaining[0] in _WRAPPERS:
        starts = [
            position
            for position, word in enumerate(remaining)
            if word.rsplit("/", 1)[-1] in _PROGRAMS
        ]
        remaining = remaining[starts[0] :] if starts else []
    return remaining


def _is_make_install(words: list[str]) -> bool:
    """Return whether one command is ``make`` with the target among its words."""
    command = _after_prefixes(shlex.split(" ".join(words)))
    if not command:
        return False
    program = command[0].rsplit("/", 1)[-1]
    if program in {"sh", "bash"} and "-c" in command[1:-1]:
        # `bash -c 'make install-whitaker'` runs the quoted script.
        return runs_make_install_whitaker(command[command.index("-c") + 1])
    return program == "make" and TARGET in command[1:]


def _substitutes_make_install(tokens: list[tuple[str, str]]) -> bool:
    """Return whether the line's code holds a command substitution that installs.

    The substitutions are looked for in the tokens, so a comment is not read,
    and across the whole line, so a backtick pair that spans several words is
    found.
    """
    code = " ".join(text for _kind, text in tokens)
    return any(
        runs_make_install_whitaker(inner)
        for groups in _SUBSTITUTION_RE.findall(code)
        for inner in groups
        if inner
    )


def _line_runs_make_install(line: str) -> bool:
    """Return whether one line of a script runs ``make`` on the target."""
    try:
        tokens = _tokenize(line)
        commands = _commands(tokens)
        return any(_is_make_install(words) for words in commands) or (
            _substitutes_make_install(tokens)
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
    >>> runs_make_install_whitaker("echo 'make install-whitaker'")
    False
    """
    lines = _LINE_CONTINUATION_RE.sub(" ", run).splitlines()
    return any(_line_runs_make_install(line) for line in lines)
