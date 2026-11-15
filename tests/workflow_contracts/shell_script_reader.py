"""Split a workflow step's shell script into lines, commands and substitutions.

This is the lexical half of reading a ``run`` script the way the shell does:
a backslash-newline joins lines except in a comment or inside single quotes,
quoted text stays inside its word (a quoted string that spans lines is one
logical line), a comment starts only at the beginning of a word and ends at its
newline, and unquoted control operators separate commands. It is not a shell
parser. A line it cannot tokenize, because a quote is left open, raises
``ValueError`` so the caller can fail closed.
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


class UnterminatedQuoteError(ValueError):
    """A line ends inside a quote; ``quote`` is the quote character left open."""

    def __init__(self, quote: str, rest: str) -> None:
        """Record the open quote and the text from where it was opened."""
        super().__init__(f"unterminated {quote} in {rest!r}")
        self.quote = quote


def tokenize(line: str) -> list[Token]:
    """Return a line's ``(kind, text)`` tokens, ending at a comment token.

    Raises
    ------
    UnterminatedQuoteError
        If a quote is left open.
    ValueError
        If the line cannot be read for another reason, such as a lone
        trailing backslash.
    """
    tokens: list[Token] = []
    position = 0
    while position < len(line):
        match = _TOKEN.match(line, position)
        if match is None:
            if line[position] in "'\"":
                raise UnterminatedQuoteError(line[position], line[position:])
            message = f"cannot read {line[position:]!r}"
            raise ValueError(message)
        position = match.end()
        if match.lastgroup != "space":
            tokens.append((str(match.lastgroup), match.group()))
        if match.lastgroup == "comment":
            break
    return tokens


def _next_pending(current: str) -> str | None:
    """Return the text to carry into the next line, or None if the line is done.

    An odd run of trailing backslashes joins the next line, with the pair
    removed, except in a comment (which ends at the newline) and inside single
    quotes (where the pair is literal text). A line that ends inside a quote
    carries its newline, so a quoted string stays one logical line.
    """
    is_continued = (len(current) - len(current.rstrip("\\"))) % 2 == 1
    body = current[:-1] if is_continued else current
    try:
        tokens = tokenize(body)
    except UnterminatedQuoteError as error:
        if error.quote == "'" or not is_continued:
            return current + "\n"
        return body
    has_comment = any(kind == "comment" for kind, _text in tokens)
    return body if is_continued and not has_comment else None


def script_lines(run: str) -> list[str]:
    """Return a script's logical lines.

    The shell removes a backslash-newline pair outright, so ``ma\\`` followed by
    ``ke`` is one word, ``make``; a comment is the exception, since it ends at
    its newline, and so is a single-quoted string, which keeps the pair.
    """
    lines: list[str] = []
    pending = ""
    for raw in run.splitlines():
        current = pending + raw
        carried = _next_pending(current)
        if carried is None:
            lines.append(current)
            pending = ""
        else:
            pending = carried
    if pending:
        lines.append(pending.rstrip("\n"))
    return lines


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
