"""Split a workflow step's shell script into lines, commands and substitutions.

This is the lexical half of reading a ``run`` script the way the shell does:
line continuations are joined, quoted text stays inside its word, a comment
starts only at the beginning of a word, and unquoted control operators separate
commands. It is not a shell parser. A line it cannot tokenize, because a quote
is left open, raises ``ValueError`` so the caller can fail closed.
"""

from __future__ import annotations

import re
import typing as typ

type Token = tuple[str, str]

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
_SUBSTITUTION: typ.Final = re.compile(r"\$\(([^()]*)\)|`([^`]*)`")


def _continues(line: str) -> bool:
    """Return whether a line ends in a backslash that joins it to the next.

    An odd run of trailing backslashes continues the line, except in a comment,
    which the shell ends at the newline whatever it ends with. A line that
    cannot be tokenized has an open quote, so it is read as continuing.
    """
    trailing = len(line) - len(line.rstrip("\\"))
    if trailing % 2 == 0:
        return False
    try:
        tokens = tokenize(line[:-1])
    except ValueError:
        return True
    return not any(kind == "comment" for kind, _text in tokens)


def script_lines(run: str) -> list[str]:
    """Return a script's lines, with continuation lines joined to their head.

    The shell removes a backslash-newline pair outright, so ``ma\\`` followed by
    ``ke`` is one word, ``make``; a comment is the exception, since it ends at
    its newline.
    """
    lines: list[str] = []
    pending = ""
    for raw in run.splitlines():
        current = pending + raw
        if _continues(current):
            pending = current[:-1]
        else:
            lines.append(current)
            pending = ""
    if pending:
        lines.append(pending)
    return lines


def tokenize(line: str) -> list[Token]:
    """Return a line's ``(kind, text)`` tokens, ending at a comment token.

    Raises
    ------
    ValueError
        If the line cannot be tokenized, which is an unterminated quote.
    """
    tokens: list[Token] = []
    position = 0
    while position < len(line):
        match = _TOKEN.match(line, position)
        if match is None:
            message = f"cannot read {line[position:]!r}"
            raise ValueError(message)
        position = match.end()
        if match.lastgroup != "space":
            tokens.append((str(match.lastgroup), match.group()))
        if match.lastgroup == "comment":
            break
    return tokens


def commands(tokens: list[Token]) -> list[list[str]]:
    """Group word tokens into commands, split at control operators."""
    grouped: list[list[str]] = [[]]
    for kind, text in tokens:
        if kind == "operator":
            grouped.append([])
        elif kind == "word":
            grouped[-1].append(text)
    return [command for command in grouped if command]


def substitutions(tokens: list[Token]) -> list[str]:
    """Return the scripts inside the line's command substitutions.

    They are looked for in the tokens, so a comment is not read, and across the
    whole line, so a backtick pair that spans several words is found. Quotes do
    not hide one: a substitution inside double quotes still runs.
    """
    code = " ".join(text for kind, text in tokens if kind != "comment")
    return [
        inner for groups in _SUBSTITUTION.findall(code) for inner in groups if inner
    ]
