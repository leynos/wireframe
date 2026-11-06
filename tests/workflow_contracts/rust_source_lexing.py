"""Strip Rust comments and literals so scanning sees only live code.

A contract that reads Rust source as text has to tell code from the text that
merely looks like it: a commented-out ``#[test]``, a doc comment quoting an
attribute, or a string literal containing ``trybuild::TestCases`` is not
evidence that anything is compiled. This module answers that question once for
every scanner in the suite, by replacing each comment and literal with inert
text of the same shape.

It is deliberately not a Rust parser. It handles the forms a contract file can
meaningfully inspect — line comments, nestable block comments, standard
strings, raw strings and character literals — and leaves everything else
alone.
"""

from __future__ import annotations

import re

#: A trybuild driver being constructed, however the path is spaced.
_CONSTRUCTS_TRYBUILD: re.Pattern[str] = re.compile(
    r"trybuild\s*::\s*TestCases\s*::\s*new\s*\("
)

#: The call that asks a driver to compile one fixture.
_FIXTURE_CALL: str = r"(?:pass|compile_fail)\s*\("

#: A driver bound to a name, so the fixture call can be tied to that name.
#: An unrelated ``.pass()`` elsewhere in the body is then not evidence that
#: this driver ran anything.
_BINDS_TRYBUILD: re.Pattern[str] = re.compile(
    r"\blet\s+(?:mut\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*"
    r"(?::[^=;]*)?=\s*trybuild\s*::\s*TestCases\s*::\s*new\s*\("
)

#: A driver constructed and asked to run a fixture in one expression, which
#: binds no name to tie the call to.
_CHAINED_RUN: re.Pattern[str] = re.compile(
    rf"trybuild\s*::\s*TestCases\s*::\s*new\s*\(\s*\)\s*\.\s*{_FIXTURE_CALL}"
)

#: The start of a raw string literal, including its hash fence.
_RAW_STRING: re.Pattern[str] = re.compile(r'r(?P<hashes>#{0,32})"')


def _skip_line_comment(source: str, index: int) -> int:
    """Return the index of the newline ending a ``//`` comment."""
    newline = source.find("\n", index + 2)
    return len(source) if newline == -1 else newline


def _skip_block_comment(source: str, index: int) -> int:
    """Return the index just past a nestable ``/* ... */`` comment."""
    depth = 1
    index += 2
    while index < len(source) and depth:
        if source.startswith("/*", index):
            depth += 1
            index += 2
        elif source.startswith("*/", index):
            depth -= 1
            index += 2
        else:
            index += 1
    return index


def _skip_string(source: str, index: int) -> int:
    """Return the index just past a standard ``"..."`` literal."""
    index += 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
        elif source[index] == '"':
            return index + 1
        else:
            index += 1
    return index


def _follows_identifier(source: str, index: int) -> bool:
    """Return whether the character before ``index`` could end an identifier."""
    if index == 0:
        return False
    previous = source[index - 1]
    return previous.isalnum() or previous == "_"


def _skip_raw_string(source: str, index: int) -> int | None:
    """Return the index past a raw string, or ``None`` if this is not one."""
    if _follows_identifier(source, index):
        return None
    match = _RAW_STRING.match(source, index)
    if match is None:
        return None
    terminator = '"' + match.group("hashes")
    end = source.find(terminator, match.end())
    return len(source) if end == -1 else end + len(terminator)


def _skip_char(source: str, index: int) -> int | None:
    """Return the index past a character literal, or ``None`` for a lifetime."""
    if index + 1 >= len(source):
        return None
    if source[index + 1] == "\\":
        # ``'\''`` escapes a quote, so the search starts after the escaped
        # character; otherwise it matches that quote and leaves the closing
        # quote to be read as the start of another literal.
        end = source.find("'", index + 3)
        return None if end == -1 else end + 1
    if index + 2 < len(source) and source[index + 2] == "'":
        return index + 3
    return None


def _char_replacement(source: str, index: int) -> tuple[str, int]:
    """Return the replacement for a lifetime start or a character literal."""
    end = _skip_char(source, index)
    if end is None:
        return source[index], index + 1
    return "' '", end


def _literal_replacement(source: str, index: int) -> tuple[str, int] | None:
    """Return the text replacing the comment or literal at ``index``."""
    if source.startswith("//", index):
        return " ", _skip_line_comment(source, index)
    if source.startswith("/*", index):
        return " ", _skip_block_comment(source, index)
    if source[index] == '"':
        return '""', _skip_string(source, index)
    if source[index] == "'":
        return _char_replacement(source, index)
    end = _skip_raw_string(source, index)
    if end is None:
        return None
    return '""', end


def without_comments_and_literals(source: str) -> str:
    """Return ``source`` with comments and literals replaced by inert text.

    Scanning runs over the result, so a commented-out ``#[test]`` or a string
    that happens to contain ``trybuild::TestCases`` cannot count as evidence.

    Parameters
    ----------
    source : str
        Rust source text to clean.

    Returns
    -------
    str
        The same text with every comment and literal blanked out.

    Examples
    --------
    >>> without_comments_and_literals("// #[test]\\nfn a() {}")
    ' \\nfn a() {}'
    """
    pieces: list[str] = []
    index = 0
    while index < len(source):
        replacement = _literal_replacement(source, index)
        if replacement is None:
            pieces.append(source[index])
            index += 1
        else:
            text, index = replacement
            pieces.append(text)
    return "".join(pieces)


def _runs_fixture_through(cleaned: str, name: str) -> bool:
    """Return whether the binding ``name`` is asked to run a fixture."""
    pattern = rf"\b{re.escape(name)}\s*\.\s*{_FIXTURE_CALL}"
    return re.search(pattern, cleaned) is not None


def trybuild_shortfall(source: str) -> str | None:
    """Return why ``source`` is not a trybuild driver, or ``None`` if it is.

    Naming a type is not compiling anything. A body that only mentions
    ``trybuild::TestCases`` — as a type annotation, say — constructs no driver,
    and a driver that is constructed but never asked to run a fixture compiles
    nothing either. Both shapes would let an override bound no work while the
    compilation it was meant to cover still runs under the default deadline, so
    each is reported as its own shortfall rather than as a bare ``False``.

    The fixture call is tied to the driver's own binding. A body that binds a
    driver and then calls ``.pass()`` on some unrelated harness compiles
    nothing through the driver, so it is rejected exactly as an unrun driver
    is. A driver constructed and used in one expression binds no name; that
    shape is accepted on its chained call.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    str | None
        A phrase naming what is missing, or ``None`` when live code both
        constructs a driver and runs a fixture through it.

    Examples
    --------
    >>> trybuild_shortfall(
    ...     "let t = trybuild::TestCases::new(); t.pass(\\"a.rs\\");") is None
    True
    >>> trybuild_shortfall("trybuild::TestCases::new().pass(\\"a.rs\\");") is None
    True
    >>> trybuild_shortfall("let t: trybuild::TestCases;")
    'constructs no trybuild::TestCases'
    >>> trybuild_shortfall("let t = trybuild::TestCases::new();")
    'constructs trybuild::TestCases but runs no fixture through it'
    >>> trybuild_shortfall(
    ...     "let t = trybuild::TestCases::new(); other.pass(\\"a.rs\\");")
    'constructs trybuild::TestCases but runs no fixture through it'
    """
    cleaned = without_comments_and_literals(source)
    if _CONSTRUCTS_TRYBUILD.search(cleaned) is None:
        return "constructs no trybuild::TestCases"
    if _CHAINED_RUN.search(cleaned) is not None:
        return None
    bindings = _BINDS_TRYBUILD.findall(cleaned)
    if not bindings:
        return "constructs trybuild::TestCases but binds it to no name"
    if not any(_runs_fixture_through(cleaned, name) for name in bindings):
        return "constructs trybuild::TestCases but runs no fixture through it"
    return None


def drives_trybuild(source: str) -> bool:
    """Return whether live code in ``source`` compiles trybuild fixtures.

    Both halves are required: a driver has to be constructed, and a fixture has
    to be run through it. A type-only reference or an unused driver compiles
    nothing, so neither counts as evidence that a target does the work an
    override is meant to bound. The call must name the driver the body bound,
    so a ``.pass()`` on some other object is not mistaken for one.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    bool
        ``True`` when live code constructs ``trybuild::TestCases`` and runs a
        fixture through it; ``False`` when either half is missing, when the
        call is made on something else, or when both appear only in a comment
        or a string literal.

    Examples
    --------
    >>> drives_trybuild(
    ...     "let t = trybuild::TestCases::new(); t.pass(\\"a.rs\\");")
    True
    >>> drives_trybuild("let t = trybuild::TestCases::new();")
    False
    >>> drives_trybuild("let t = trybuild::TestCases::new(); h.pass(\\"a.rs\\");")
    False
    >>> drives_trybuild('// trybuild::TestCases')
    False
    """
    return trybuild_shortfall(source) is None
