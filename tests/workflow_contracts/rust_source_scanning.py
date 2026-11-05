"""Read Rust test modules well enough to prove a nextest exact matcher.

Nextest names a test by its module path, so ``test(=compile_tests)`` selects a
crate-root ``#[test] fn compile_tests`` and nothing else. Proving that from
source means knowing which ``fn`` items are tests, which module each sits in,
and which of the surrounding text is actually code: a commented-out definition
or a string literal containing the name is not evidence.

The scanning here is deliberately small. It handles the layouts a contract file
can meaningfully inspect — inline ``mod`` blocks, item braces, comments,
standard strings, raw strings and character literals — and ignores everything
else. It is not a Rust parser, and it says so: a definition it cannot see is
reported as missing rather than assumed.
"""

from __future__ import annotations

import re

#: A trybuild driver, however the path is spaced.
_TRYBUILD: re.Pattern[str] = re.compile(r"trybuild\s*::\s*TestCases")

#: Tokens that name a test or change the scope around it. ``mod NAME {``
#: consumes its opening brace so the module block is counted once. The Loom
#: gate is a token so it can be attributed to the scope that encloses it.
_ITEM_TOKEN: re.Pattern[str] = re.compile(
    r"""
    (?P<loom_gate>\#\!\s*\[\s*cfg\s*\(\s*loom\s*\)\s*\])
    | (?P<module>\bmod\s+(?P<module_name>[A-Za-z_][A-Za-z0-9_]*)\s*\{)
    | (?P<open>\{)
    | (?P<close>\})
    | (?P<test>\#\s*\[\s*test\s*\])
    | (?P<function>\bfn\s+(?P<function_name>[A-Za-z_][A-Za-z0-9_]*))
    """,
    re.VERBOSE,
)

#: An active, whole-line crate-level coverage opt-out. Leading whitespace is
#: not accepted, so a gate indented inside an inline module does not qualify.
_COVERAGE_GATE: re.Pattern[str] = re.compile(
    r"#![ \t]*\[[ \t]*cfg[ \t]*\([ \t]*not[ \t]*\([ \t]*coverage[ \t]*\)[ \t]*\)"
    r"[ \t]*\]"
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


def _without_comments_and_literals(source: str) -> str:
    """Return ``source`` with comments and literals replaced by spaces.

    Discovery runs over the result, so a commented-out ``#[test]`` or a string
    that happens to contain ``trybuild::TestCases`` cannot count as evidence.
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


class _ScanState:
    """Track open blocks and a pending ``#[test]`` while items are scanned."""

    def __init__(self) -> None:
        self.blocks: list[str | None] = []
        self.is_test_pending = False


def _enter_module(state: _ScanState, token: re.Match[str]) -> None:
    """Record an inline module and clear any pending test attribute."""
    state.blocks.append(token.group("module_name"))
    state.is_test_pending = False


def _enter_block(state: _ScanState) -> None:
    """Record a block that is not a module, such as an ``impl`` or ``fn``."""
    state.blocks.append(None)
    state.is_test_pending = False


def _leave_block(state: _ScanState) -> None:
    """Close the innermost block, if any."""
    if state.blocks:
        state.blocks.pop()
    state.is_test_pending = False


def _note_function(state: _ScanState, token: re.Match[str]) -> str | None:
    """Return the qualified name of a pending test, or ``None`` for other items."""
    if not state.is_test_pending:
        return None
    state.is_test_pending = False
    modules = [block for block in state.blocks if block is not None]
    if len(modules) != len(state.blocks):
        return None
    return "::".join([*modules, token.group("function_name")])


def qualified_test_names(source: str) -> frozenset[str]:
    """Return the module-qualified names of the tests nextest would list.

    Nextest names a test by its module path, so an inline ``mod contracts {
    #[test] fn compile_tests() }`` is ``contracts::compile_tests``. A
    ``#[test]`` inside any other block (an ``impl`` or a function body) is
    not a test nextest runs, so it contributes no name. Comments and literals
    are stripped first, so a commented-out definition contributes nothing.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    frozenset[str]
        Fully qualified test names, such as ``{"contracts::compile_tests"}``.

    Examples
    --------
    >>> sorted(qualified_test_names("#[test]\\nfn a() {}"))
    ['a']
    >>> sorted(qualified_test_names("mod m { #[test] fn b() {} }"))
    ['m::b']
    """
    text = _without_comments_and_literals(source)
    state = _ScanState()
    paths: set[str] = set()
    for token in _ITEM_TOKEN.finditer(text):
        # ``lastgroup`` names the alternative that matched, not its inner
        # capture: a module token reports ``module``, a function ``function``.
        match token.lastgroup:
            case "module":
                _enter_module(state, token)
            case "open":
                _enter_block(state)
            case "close":
                _leave_block(state)
            case "test":
                state.is_test_pending = True
            case "function":
                path = _note_function(state, token)
                if path is not None:
                    paths.add(path)
    return frozenset(paths)


def drives_trybuild(source: str) -> bool:
    """Return whether live code in ``source`` constructs ``trybuild::TestCases``.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    bool
        ``True`` when live code constructs ``trybuild::TestCases``; ``False``
        when it appears only in a comment or a string literal.
    """
    return _TRYBUILD.search(_without_comments_and_literals(source)) is not None


def is_loom_gated(source: str) -> bool:
    """Return whether crate-scope live code carries ``#![cfg(loom)]``.

    An inner attribute applies to the item that encloses it, so a gate inside
    an inline module gates that module rather than the crate, however it is
    indented. Tracking brace scope therefore beats matching indentation: a
    module-nested gate at column zero is not crate-level, and an indented
    crate-level gate still is. Comments and literals are stripped first, so a
    commented-out gate contributes nothing.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    bool
        ``True`` only when the gate applies to the crate root.

    Examples
    --------
    >>> is_loom_gated("#![cfg(loom)]\\n")
    True
    >>> is_loom_gated("mod m {\\n#![cfg(loom)]\\n}\\n")
    False
    """
    state = _ScanState()
    for token in _ITEM_TOKEN.finditer(_without_comments_and_literals(source)):
        match token.lastgroup:
            case "loom_gate":
                if not state.blocks:
                    return True
            case "module":
                _enter_module(state, token)
            case "open":
                _enter_block(state)
            case "close":
                _leave_block(state)
    return False


def has_coverage_gate(source: str) -> bool:
    """Return whether live source opens a crate-level coverage opt-out.

    Comments and literals are removed before the search, so an attribute that
    was merely commented out is not a guard. The attribute must also start its
    own line, which keeps a gate indented inside an inline module from counting
    as a crate-level one.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    bool
        ``True`` when ``#![cfg(not(coverage))]`` is active at crate level.

    Examples
    --------
    >>> has_coverage_gate("#![cfg(not(coverage))]\\n")
    True
    >>> has_coverage_gate("// #![cfg(not(coverage))]\\n")
    False
    """
    return any(
        _COVERAGE_GATE.match(line)
        for line in _without_comments_and_literals(source).splitlines()
    )


def defines_test_function(source: str, name: str) -> bool:
    """Return whether ``source`` defines a crate-root ``#[test] fn name``.

    Nextest matches ``test(=NAME)`` against the fully qualified name, so a
    definition inside an inline module is ``module::NAME`` and the exact
    matcher does not select it. Comments and literals are ignored, so a
    commented-out definition is not evidence.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    bool
        ``True`` only for a crate-root test definition.

    Examples
    --------
    >>> defines_test_function("#[test]\\nfn compile_tests() {}", "compile_tests")
    True
    >>> defines_test_function("mod m { #[test] fn c() {} }", "c")
    False
    """
    return name in qualified_test_names(source)


def describe_qualified_definitions(source: str, name: str) -> str:
    """Explain a definition the filter's exact name does not reach.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    str
        A clause naming the module-qualified definitions, or an empty string
        when no test with that bare name exists at all.
    """
    qualified = sorted(
        path
        for path in qualified_test_names(source)
        if path.rsplit("::", 1)[-1] == name
    )
    if not qualified:
        return ""
    return (
        "; it is defined only as "
        + ", ".join(qualified)
        + ", which nextest names by module path"
    )
