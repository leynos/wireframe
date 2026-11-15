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
_LINE_CONTINUATION: typ.Final = re.compile(r"\\\n")
_SUBSTITUTION: typ.Final = re.compile(r"\$\(([^()]*)\)|`([^`]*)`")


def script_lines(run: str) -> list[str]:
    """Return a script's lines, with continuation lines joined to their head.

    The shell removes a backslash-newline pair outright, so ``ma\\`` followed by
    ``ke`` is one word, ``make``.
    """
    return _LINE_CONTINUATION.sub("", run).splitlines()


def tokenize(line: str) -> list[Token]:
    """Return a line's ``(kind, text)`` tokens, ending at a comment.

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
        if match.lastgroup == "comment":
            break
        if match.lastgroup != "space":
            tokens.append((str(match.lastgroup), match.group()))
    return tokens


def commands(tokens: list[Token]) -> list[list[str]]:
    """Group word tokens into commands, split at control operators."""
    grouped: list[list[str]] = [[]]
    for kind, text in tokens:
        if kind == "operator":
            grouped.append([])
        else:
            grouped[-1].append(text)
    return [command for command in grouped if command]


def substitutions(tokens: list[Token]) -> list[str]:
    """Return the scripts inside the line's command substitutions.

    They are looked for in the tokens, so a comment is not read, and across the
    whole line, so a backtick pair that spans several words is found. Quotes do
    not hide one: a substitution inside double quotes still runs.
    """
    code = " ".join(text for _kind, text in tokens)
    return [
        inner for groups in _SUBSTITUTION.findall(code) for inner in groups if inner
    ]
