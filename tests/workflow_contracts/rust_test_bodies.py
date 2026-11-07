"""Read what one named test does, rather than what its file contains.

An override's exact matcher names a test, so the test that has to compile the
cases is the named one. Asking whether the *file* drives trybuild is a weaker
question: a sibling test can construct ``trybuild::TestCases`` while the
selected test does nothing, and the override then bounds no work while the
compilation still runs under the default deadline.

This module answers the narrow question. It finds the crate-root ``#[test]``
item with the given name and returns the text between that function's own
braces, using the token vocabulary and scope state of
``rust_source_scanning``. Comments and literals are blanked first, so a driver
mentioned only in a comment or a string is not evidence.
"""

from __future__ import annotations

import re

from rust_source_lexing import trybuild_shortfall, without_comments_and_literals
from rust_source_scanning import (
    ITEM_TOKEN,
    ScanState,
    enter_block,
    enter_module,
    leave_block,
    note_function,
)


def _brace_depth_delta(token: re.Match[str]) -> int:
    """Return how much one token changes the enclosing brace depth.

    A ``mod NAME {`` token consumes its own opening brace, so it opens a block
    just as a bare ``{`` does. Counting only ``open`` would end the body at the
    module's closing brace and hide everything the test does after it.
    """
    return {"open": 1, "module": 1, "close": -1}.get(token.lastgroup, 0)


def _closing_brace(cleaned: str, open_index: int) -> int | None:
    """Return the index of the brace closing the one at ``open_index``."""
    depth = 0
    for token in ITEM_TOKEN.finditer(cleaned, open_index):
        depth += _brace_depth_delta(token)
        if depth == 0:
            return token.start()
    return None


def _braced_body(cleaned: str, open_index: int) -> str:
    """Return the text between the braces that open at ``open_index``.

    An unterminated body yields the rest of the source rather than raising:
    the contract reports a missing driver either way, and a partial file is
    still worth reading rather than crashing the whole gate.
    """
    end = _closing_brace(cleaned, open_index)
    if end is None:
        return cleaned[open_index + 1 :]
    return cleaned[open_index + 1 : end]


#: A nested ``fn`` item inside a test body. Unlike a block, it runs only when
#: something calls it, so a driver written inside one may never execute.
_NESTED_FUNCTION: re.Pattern[str] = re.compile(
    r"\bfn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\("
)


def _is_called_outside(body: str, name: str, span: tuple[int, int]) -> bool:
    """Return whether ``name`` is called anywhere but inside its own definition.

    The definition's own span is removed first, so a recursive call does not
    count as evidence that the test reaches it.
    """
    rest = f"{body[: span[0]]}{body[span[1] :]}"
    return re.search(rf"\b{re.escape(name)}\s*\(", rest) is not None


def _without_uncalled_nested_functions(body: str) -> str:
    """Blank every nested function definition the test body never calls.

    A nested ``fn`` item is a declaration, not a call: a driver written inside
    one compiles nothing unless the test invokes it. Blanking the definition
    keeps the rest of the body's evidence intact, so a test that also drives
    trybuild directly is still accepted.
    """
    for definition in reversed(list(_NESTED_FUNCTION.finditer(body))):
        open_index = body.find("{", definition.end())
        if open_index == -1:
            continue
        end = _closing_brace(body, open_index)
        stop = len(body) if end is None else end + 1
        span = (definition.start(), stop)
        if _is_called_outside(body, definition["name"], span):
            continue
        body = f"{body[: span[0]]}{' ' * (stop - span[0])}{body[stop:]}"
    return body


def test_function_body(source: str, name: str) -> str | None:
    """Return the body of the crate-root ``#[test] fn name``, or ``None``.

    The body is the text between the function's own braces, so a caller can
    ask what that one test does rather than what the file contains. A
    definition inside an inline module, an ``impl`` or another function is not
    the crate-root test an exact matcher selects, so it yields ``None``.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    str | None
        The body text, or ``None`` when no crate-root test has that name.

    Examples
    --------
    >>> test_function_body("#[test]\\nfn a() { work(); }", "a")
    ' work(); '
    >>> test_function_body("mod m { #[test] fn a() {} }", "a") is None
    True
    """
    cleaned = without_comments_and_literals(source)
    state = ScanState()
    is_awaiting_body = False
    for token in ITEM_TOKEN.finditer(cleaned):
        if is_awaiting_body and token.lastgroup == "open":
            return _braced_body(cleaned, token.start())
        match token.lastgroup:
            case "module":
                enter_module(state, token)
                is_awaiting_body = False
            case "open":
                enter_block(state)
            case "close":
                leave_block(state)
            case "semicolon":
                state.is_cfg_pending = False
                is_awaiting_body = False
            case "cfg_attribute":
                state.is_cfg_pending = True
            case "test":
                state.is_test_pending = True
            case "function":
                is_awaiting_body = note_function(state, token) == name
    return None


def trybuild_shortfall_in_test(source: str, name: str) -> str | None:
    """Return why the named crate-root test does not drive trybuild.

    A sibling test that constructs ``trybuild::TestCases`` does not make the
    override's test compile anything, so the question is scoped to the selected
    function rather than the file. A driver reached through a helper function
    is not recognized; the contract reports that as missing rather than
    assuming it, which fails loudly instead of passing silently. The same holds
    for a driver written inside a nested ``fn`` the test never calls: the item
    is declared, not run, so it compiles nothing.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    str | None
        A phrase naming what the body lacks, or ``None`` when it both
        constructs a trybuild driver and runs a fixture through it.

    Examples
    --------
    >>> trybuild_shortfall_in_test(
    ...     '#[test]\\nfn a() {\\n'
    ...     '    let t = trybuild::TestCases::new();\\n'
    ...     '    t.pass("a.rs");\\n}', "a") is None
    True
    >>> trybuild_shortfall_in_test(
    ...     "#[test]\\nfn a() { let t = trybuild::TestCases::new(); }", "a")
    'constructs trybuild::TestCases but runs no fixture through it'
    >>> trybuild_shortfall_in_test(
    ...     "#[test]\\nfn a() {}\\n#[test]\\nfn b() {"
    ...     " let t = trybuild::TestCases::new(); t.pass(\\"b.rs\\"); }", "a")
    'constructs no trybuild::TestCases'
    """
    body = test_function_body(source, name)
    if body is None:
        return "has no crate-root test body to read"
    shortfall = trybuild_shortfall(_without_uncalled_nested_functions(body))
    if shortfall is not None and trybuild_shortfall(body) is None:
        return "declares its trybuild driver in a nested function the test never calls"
    return shortfall


def drives_trybuild_in_test(source: str, name: str) -> bool:
    """Return whether the named crate-root test's own body drives trybuild.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    bool
        ``True`` only when that test's body constructs ``trybuild::TestCases``
        and runs a fixture through it.

    Examples
    --------
    >>> drives_trybuild_in_test(
    ...     '#[test]\\nfn a() {\\n'
    ...     '    let t = trybuild::TestCases::new();\\n'
    ...     '    t.pass("a.rs");\\n}', "a")
    True
    >>> drives_trybuild_in_test(
    ...     "#[test]\\nfn a() {}\\n#[test]\\nfn b() {"
    ...     " let t = trybuild::TestCases::new(); t.pass(\\"b.rs\\"); }", "a")
    False
    """
    return trybuild_shortfall_in_test(source, name) is None